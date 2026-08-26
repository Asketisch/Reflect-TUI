//! TextArea 文本编辑组件。
//!
//! 注：本文件超过 800 行红线，但 `impl TextArea`（约 1755 行，106 个方法）为内聚的完整
//! 状态机（光标移动/kill-ring/词边界/换行/vim 模式），与上游逐行对应，按 AGENTS.md
//! 「内聚完整状态机例外」保留。

//! TextArea 拥有可编辑的 composer 文本、占位符元素、光标/换行状态和单条杀入缓冲区。
//!
//! 整个缓冲区替换 API 有意仅重建可见的草稿状态。它们清除元素范围和派生的光标/换行缓存，
//! 但保持杀入缓冲区完整，因此调用方可清除或重写草稿，仍允许 `Ctrl+Y` 恢复用户最近
//! 的 `Ctrl+K`。这是更高层 composer 流程在提交、slash 命令分发和其他合成清除后依赖的契约。
//!
//! 本模块不实现 Emacs 风格的多条目杀入环；仅保留最近被杀的片段。

use crate::protocol_compat::user_input::ByteRange;
use crate::protocol_compat::user_input::TextElement as UserTextElement;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::key_hint::is_altgr;
use crate::tui_core::keymap::EditorKeymap;
use crate::tui_core::keymap::RuntimeKeymap;
use crate::tui_core::keymap::VimNormalKeymap;
use crate::tui_core::keymap::VimOperatorKeymap;
use crate::tui_core::keymap::VimTextObjectKeymap;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::widgets::StatefulWidgetRef;
use ratatui::widgets::WidgetRef;
use std::borrow::Cow;
use std::cell::Ref;
use std::cell::RefCell;
use std::ops::Range;
use textwrap::Options;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

mod vim;
use self::vim::VimMode;
use self::vim::VimMotion;
use self::vim::VimOperator;
use self::vim::VimPending;
use self::vim::VimTextObjectScope;

/// fork ratatui 下 `Style::default()` 可能与背景同色导致正文不可见。
// ── 文本工具辅助（外移子模块） ──
mod text_utils;
use text_utils::*;

#[derive(Debug, Clone)]
struct TextElement {
    id: u64,
    range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TextElementSnapshot {
    pub(crate) id: u64,
    pub(crate) range: Range<usize>,
    pub(crate) text: String,
}

    /// `TextArea` 是 TUI composer 背后的可编辑缓冲区。
    ///
    /// 它拥有原始 UTF-8 文本、必须随编辑原子移动的占位符式文本元素、用于渲染的光标/换行状态，
    /// 以及用于 `Ctrl+K` / `Ctrl+Y` 式编辑的单条目杀入缓冲区。调用方可通过
    /// [`Self::set_text_clearing_elements`] 或 [`Self::set_text_with_elements`] 替换整个可见缓冲区
    /// 而不干扰杀入缓冲区；如果他们错误地假设这些方法会完全重置编辑状态，那么之后的 yank
    /// 在用户看来就像恢复了过期文本。
#[derive(Debug)]
pub(crate) struct TextArea {
    text: String,
    cursor_pos: usize,
    wrap_cache: RefCell<Option<WrapCache>>,
    preferred_col: Option<usize>,
    elements: Vec<TextElement>,
    next_element_id: u64,
    kill_buffer: String,
    kill_buffer_kind: KillBufferKind,
    vim_enabled: bool,
    vim_mode: VimMode,
    vim_pending: VimPending,
    editor_keymap: EditorKeymap,
    vim_normal_keymap: VimNormalKeymap,
    vim_operator_keymap: VimOperatorKeymap,
    vim_text_object_keymap: VimTextObjectKeymap,
}

#[derive(Debug, Clone)]
struct WrapCache {
    width: u16,
    lines: Vec<Range<usize>>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TextAreaState {
    /// 首个可见行在换行后的行列表中的索引。
    scroll: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KillBufferKind {
    /// 按字符杀的片段在光标处粘贴。
    Characterwise,
    /// 按行杀的片段作为整行粘贴到光标行下方。
    Linewise,
}

impl TextArea {
    pub fn new() -> Self {
        let defaults = RuntimeKeymap::defaults();
        Self {
            text: String::new(),
            cursor_pos: 0,
            wrap_cache: RefCell::new(None),
            preferred_col: None,
            elements: Vec::new(),
            next_element_id: 1,
            kill_buffer: String::new(),
            kill_buffer_kind: KillBufferKind::Characterwise,
            vim_enabled: false,
            vim_mode: VimMode::Insert,
            vim_pending: VimPending::None,
            editor_keymap: defaults.editor,
            vim_normal_keymap: defaults.vim_normal,
            vim_operator_keymap: defaults.vim_operator,
            vim_text_object_keymap: defaults.vim_text_object,
        }
    }

    /// 替换后续文本编辑输入所使用的编辑器与 Vim 键位表。
    ///
    /// 此方法刻意只替换键位表缓存。它不会重新解释待处理的输入、改变 Vim 模式、
    /// 移动光标或修改杀入缓冲区，因此调用方可以在完全保留当前草稿的情况下安全地应用
    /// 实时的配置更新。
    pub fn set_keymap_bindings(&mut self, keymap: &RuntimeKeymap) {
        self.editor_keymap = keymap.editor.clone();
        self.vim_normal_keymap = keymap.vim_normal.clone();
        self.vim_operator_keymap = keymap.vim_operator.clone();
        self.vim_text_object_keymap = keymap.vim_text_object.clone();
    }

    /// 替换可见的 textarea 文本并清除所有现有的文本元素。
    ///
    /// 这是面向「只需要纯文本、无需占位符范围」的调用方的「全新缓冲区」路径。它刻意保留当前的
    /// 杀入缓冲区，因为诸如提交或 slash 命令分发等更上层的流程会通过此方法清除草稿，但仍希望
    /// `Ctrl+Y` 能恢复用户最近一次杀入的文本。
    pub fn set_text_clearing_elements(&mut self, text: &str) {
        self.set_text_inner(text, /*elements*/ None);
    }

    /// 替换可见的 textarea 文本并按提供的文本元素重建元素列表。
    ///
    /// 与 [`Self::set_text_clearing_elements`] 类似，此方法只重置从可见缓冲区派生的状态。
    /// 杀入缓冲区得以保留，这样恢复草稿或外部编辑的调用方不会悄悄丢弃待 yank 的目标。
    pub fn set_text_with_elements(&mut self, text: &str, elements: &[UserTextElement]) {
        self.set_text_inner(text, Some(elements));
    }

    fn set_text_inner(&mut self, text: &str, elements: Option<&[UserTextElement]>) {
        // 阶段 1：替换原始文本，并将光标保持在安全的字节范围内。
        self.text = text.to_string();
        self.cursor_pos = self.cursor_pos.clamp(0, self.text.len());
        // 阶段 2：基于新文本从头重建元素范围。
        self.elements.clear();
        if let Some(elements) = elements {
            for elem in elements {
                let mut start = elem.byte_range.start.min(self.text.len());
                let mut end = elem.byte_range.end.min(self.text.len());
                start = self.clamp_pos_to_char_boundary(start);
                end = self.clamp_pos_to_char_boundary(end);
                if start >= end {
                    continue;
                }
                let id = self.next_element_id();
                self.elements.push(TextElement {
                    id,
                    range: start..end,
                });
            }
            self.elements.sort_by_key(|e| e.range.start);
        }
        // 阶段 3：钳制光标并重置与先前内容相关的派生状态。
        // 杀入缓冲区属于编辑历史而非可见缓冲区状态，因此整缓冲区替换刻意保留它不动。
        self.cursor_pos = self.clamp_pos_to_nearest_boundary(self.cursor_pos);
        self.wrap_cache.replace(None);
        self.preferred_col = None;
    }

    /// 启用或禁用 textarea 的模态 Vim 编辑。
    ///
    /// 启用时总是进入普通模式，禁用时总是回到插入语义。两种方向的切换都会清除待处理的
    /// 操作符，因此切换不会让下一次按键被解释为旧的 `d` 或 `y` 命令的后半部分。
    pub(crate) fn set_vim_enabled(&mut self, enabled: bool) {
        self.vim_enabled = enabled;
        self.vim_pending = VimPending::None;
        self.vim_mode = if enabled {
            VimMode::Normal
        } else {
            VimMode::Insert
        };
    }

    /// 返回模态 Vim 编辑当前是否已启用。
    pub(crate) fn is_vim_enabled(&self) -> bool {
        self.vim_enabled
    }

    /// 返回 Vim 模式是否已启用且当前正停留在普通模式。
    ///
    /// Composer 层的事件处理器用此判断：只有当普通模式移动到达文本边界后，才应把
    /// Up/Down 留给历史导航使用。
    pub(crate) fn is_vim_normal_mode(&self) -> bool {
        self.vim_enabled && self.vim_mode == VimMode::Normal
    }

    /// 返回在 Vim 普通模式中代表最后一个可编辑项的光标位置。
    pub(crate) fn vim_normal_end_cursor(&self) -> usize {
        if self.text.is_empty() {
            0
        } else {
            self.prev_atomic_boundary(self.text.len())
        }
    }

    /// 返回是否有 Vim 操作符正在等待一个移动命令。
    ///
    /// 该状态可被观察到，以便 composer 避免把 `d{motion}` 或 `y{motion}` 的第二个按键
    /// 抢占为更高级的快捷键。
    pub(crate) fn is_vim_operator_pending(&self) -> bool {
        !matches!(self.vim_pending, VimPending::None)
    }

    /// 若模态编辑已启用则进入 Vim 插入模式。
    ///
    /// 在 Vim 被禁用时调用此方法不产生任何效果，这使得父级流程在提交后无需先判断当前
    /// 键位表状态即可重置模式。
    pub(crate) fn enter_vim_insert_mode(&mut self) {
        if self.vim_enabled {
            self.vim_mode = VimMode::Insert;
            self.vim_pending = VimPending::None;
        }
    }

    /// 若模态编辑已启用则进入 Vim 普通模式。
    ///
    /// 这会清除所有待处理的操作符和首选垂直列。后者符合离开插入模式后对普通 Vim 导航的
    /// 预期；若保留旧列，下一次 `j` 或 `k` 会跳到过期的视觉目标上。
    pub(crate) fn enter_vim_normal_mode(&mut self) {
        if self.vim_enabled {
            self.vim_mode = VimMode::Normal;
            self.vim_pending = VimPending::None;
            self.preferred_col = None;
        }
    }

    /// 返回快速的普通按键连发是否应被视为粘贴输入。
    ///
    /// 在 Vim 普通模式下粘贴连发检测被禁用，因此像 `dd` 或 `yw` 这样的快速按键序列仍是
    /// 命令输入，而不会被转换成字面文本。
    pub(crate) fn allows_paste_burst(&self) -> bool {
        !self.vim_enabled || self.vim_mode == VimMode::Insert
    }

    /// 返回渲染时是否应使用插入模式的光标样式。
    pub(crate) fn uses_vim_insert_cursor(&self) -> bool {
        self.vim_enabled && self.vim_mode == VimMode::Insert
    }

    /// 返回 Escape 是否应在 composer 层路由之前被拦截。
    ///
    /// 在 Vim 插入模式下，Escape 是编辑状态的转换而非弹窗取消/回退快捷键。若让 composer
    /// 先处理它，会在 textarea 仍处于插入模式时关闭 UI 界面。
    pub(crate) fn should_handle_vim_insert_escape(&self, event: KeyEvent) -> bool {
        self.vim_enabled
            && self.vim_mode == VimMode::Insert
            && event.code == KeyCode::Esc
            && event.modifiers == KeyModifiers::NONE
            && matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat)
    }

    /// 返回当前 Vim 模式的底部状态栏标签。
    ///
    /// `None` 表示 Vim 编辑已禁用，因此调用方应省略模式指示器，而不是为普通的非模态编辑
    /// 渲染插入模式的标签。
    pub(crate) fn vim_mode_label(&self) -> Option<&'static str> {
        if !self.vim_enabled {
            return None;
        }
        Some(match self.vim_mode {
            VimMode::Normal => "Normal",
            VimMode::Insert => "Insert",
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn insert_str(&mut self, text: &str) {
        self.insert_str_at(self.cursor_pos, text);
    }

    pub fn insert_str_at(&mut self, pos: usize, text: &str) {
        let pos = self.clamp_pos_for_insertion(pos);
        self.text.insert_str(pos, text);
        self.wrap_cache.replace(None);
        if pos <= self.cursor_pos {
            self.cursor_pos += text.len();
        }
        self.shift_elements(pos, /*removed*/ 0, text.len());
        self.preferred_col = None;
    }

    pub fn replace_range(&mut self, range: std::ops::Range<usize>, text: &str) {
        let range = self.expand_range_to_element_boundaries(range);
        self.replace_range_raw(range, text);
    }

    fn replace_range_raw(&mut self, range: std::ops::Range<usize>, text: &str) {
        assert!(range.start <= range.end);
        let start = range.start.clamp(0, self.text.len());
        let end = range.end.clamp(0, self.text.len());
        let removed_len = end - start;
        let inserted_len = text.len();
        if removed_len == 0 && inserted_len == 0 {
            return;
        }
        let diff = inserted_len as isize - removed_len as isize;

        self.text.replace_range(range, text);
        self.wrap_cache.replace(None);
        self.preferred_col = None;
        self.update_elements_after_replace(start, end, inserted_len);

        // 根据本次编辑更新光标位置。
        self.cursor_pos = if self.cursor_pos < start {
            // 光标位于编辑范围之前——无需偏移。
            self.cursor_pos
        } else if self.cursor_pos <= end {
            // 光标位于被替换范围内——移动到新文本的末尾。
            start + inserted_len
        } else {
            // 光标位于被替换范围之后——按长度差偏移。
            ((self.cursor_pos as isize) + diff) as usize
        }
        .min(self.text.len());

        // 确保光标不在元素内部
        self.cursor_pos = self.clamp_pos_to_nearest_boundary(self.cursor_pos);
    }

    pub fn cursor(&self) -> usize {
        self.cursor_pos
    }

    pub fn set_cursor(&mut self, pos: usize) {
        self.cursor_pos = pos.clamp(0, self.text.len());
        self.cursor_pos = self.clamp_pos_to_nearest_boundary(self.cursor_pos);
        self.preferred_col = None;
    }

    pub fn desired_height(&self, width: u16) -> u16 {
        self.wrapped_lines(width).len() as u16
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.cursor_pos_with_state(area, TextAreaState::default())
    }

    /// 计算考虑滚动之后的光标在屏幕上的位置。
    pub fn cursor_pos_with_state(&self, area: Rect, state: TextAreaState) -> Option<(u16, u16)> {
        let lines = self.wrapped_lines(area.width);
        let effective_scroll = self.effective_scroll(area.height, &lines, state.scroll);
        let i = Self::wrapped_line_index_by_start(&lines, self.cursor_pos)?;
        let ls = &lines[i];
        let col = self.text[ls.start..self.cursor_pos].width() as u16;
        let screen_row = i
            .saturating_sub(effective_scroll as usize)
            .try_into()
            .unwrap_or(0);
        Some((area.x + col, area.y + screen_row))
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn current_display_col(&self) -> usize {
        let bol = self.beginning_of_current_line();
        self.text[bol..self.cursor_pos].width()
    }

    fn wrapped_line_index_by_start(lines: &[Range<usize>], pos: usize) -> Option<usize> {
        // partition_point 返回第一个使谓词为假的元素下标，
        // 也就是 start <= pos 的元素个数。
        let idx = lines.partition_point(|r| r.start <= pos);
        if idx == 0 { None } else { Some(idx - 1) }
    }

    fn move_to_display_col_on_line(
        &mut self,
        line_start: usize,
        line_end: usize,
        target_col: usize,
    ) {
        let mut width_so_far = 0usize;
        for (i, g) in self.text[line_start..line_end].grapheme_indices(true) {
            width_so_far += g.width();
            if width_so_far > target_col {
                self.cursor_pos = line_start + i;
                // 避免落在元素内部；就近取整到最近边界
                self.cursor_pos = self.clamp_pos_to_nearest_boundary(self.cursor_pos);
                return;
            }
        }
        self.cursor_pos = line_end;
        self.cursor_pos = self.clamp_pos_to_nearest_boundary(self.cursor_pos);
    }

    fn beginning_of_line(&self, pos: usize) -> usize {
        self.text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0)
    }
    fn beginning_of_current_line(&self) -> usize {
        self.beginning_of_line(self.cursor_pos)
    }

    fn first_non_blank_of_current_line(&self) -> usize {
        let bol = self.beginning_of_current_line();
        let eol = self.end_of_current_line();
        self.text[bol..eol]
            .char_indices()
            .find_map(|(offset, ch)| (!ch.is_whitespace()).then_some(bol + offset))
            .unwrap_or(eol)
    }

    fn end_of_line(&self, pos: usize) -> usize {
        self.text[pos..]
            .find('\n')
            .map(|i| i + pos)
            .unwrap_or(self.text.len())
    }
    fn end_of_current_line(&self) -> usize {
        self.end_of_line(self.cursor_pos)
    }

    pub fn input(&mut self, event: KeyEvent) {
        // 只处理按键按下或重复事件；忽略松开事件，避免在修饰键不再上报的
        // 按键抬起时插入字符。
        if !matches!(event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        if self.vim_enabled {
            self.handle_vim_input(event);
        } else {
            let keymap = self.editor_keymap.clone();
            self.input_with_keymap(event, &keymap);
        }
    }

    pub fn input_with_keymap(&mut self, event: KeyEvent, keymap: &EditorKeymap) {
        if keymap.insert_newline.is_pressed(event) {
            self.insert_str("\n");
            return;
        }

        if keymap.delete_backward_word.is_pressed(event) {
            self.delete_backward_word();
            return;
        }

        // Windows 的 AltGr 会生成 ALT|CONTROL。除非上面已有特定的快捷键匹配，
        // 否则为 AltGr 用户保留输入的字符。
        if let KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            ..
        } = event
            && is_altgr(modifiers)
        {
            self.insert_str(&c.to_string());
            return;
        }

        if keymap.delete_backward.is_pressed(event) {
            self.delete_backward(/*n*/ 1);
            return;
        }
        if keymap.delete_forward_word.is_pressed(event) {
            self.delete_forward_word();
            return;
        }
        if keymap.delete_forward.is_pressed(event) {
            self.delete_forward(/*n*/ 1);
            return;
        }
        if keymap.kill_line_start.is_pressed(event) {
            self.kill_to_beginning_of_line();
            return;
        }
        if keymap.kill_whole_line.is_pressed(event) {
            self.kill_current_line();
            return;
        }
        if keymap.kill_line_end.is_pressed(event) {
            self.kill_to_end_of_line();
            return;
        }
        if keymap.yank.is_pressed(event) {
            self.yank();
            return;
        }
        if keymap.move_word_left.is_pressed(event) {
            self.set_cursor(self.beginning_of_previous_word());
            return;
        }
        if keymap.move_word_right.is_pressed(event) {
            self.set_cursor(self.end_of_next_word());
            return;
        }
        if keymap.move_left.is_pressed(event) {
            self.move_cursor_left();
            return;
        }
        if keymap.move_right.is_pressed(event) {
            self.move_cursor_right();
            return;
        }
        if keymap.move_up.is_pressed(event) {
            self.move_cursor_up();
            return;
        }
        if keymap.move_down.is_pressed(event) {
            self.move_cursor_down();
            return;
        }
        if keymap.move_line_start.is_pressed(event) {
            let move_up_at_bol = matches!(
                event,
                KeyEvent {
                    code: KeyCode::Char('a'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                }
            );
            self.move_cursor_to_beginning_of_line(move_up_at_bol);
            return;
        }
        if keymap.move_line_end.is_pressed(event) {
            let move_down_at_eol = matches!(
                event,
                KeyEvent {
                    code: KeyCode::Char('e'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                }
            );
            self.move_cursor_to_end_of_line(move_down_at_eol);
            return;
        }

        if let KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
            ..
        } = event
        {
            // 插入普通字符（以及带 Shift 修饰的字符）。按住 ALT 时不插入，
            // 因为许多终端会把 Option/Meta 组合映射为 ALT+<char>。
            if c.is_ascii_control() {
                return;
            }
            self.insert_str(&c.to_string());
        }

        tracing::debug!("Unhandled key event in TextArea: {:?}", event);
    }

    fn handle_vim_input(&mut self, event: KeyEvent) {
        match self.vim_mode {
            VimMode::Insert => self.handle_vim_insert(event),
            VimMode::Normal => self.handle_vim_normal(event),
        }
    }

    fn handle_vim_insert(&mut self, event: KeyEvent) {
        if matches!(event.code, KeyCode::Esc) {
            let bol = self.beginning_of_current_line();
            if self.cursor_pos > bol {
                self.cursor_pos = self.prev_atomic_boundary(self.cursor_pos).max(bol);
            }
            self.enter_vim_normal_mode();
            return;
        }
        let keymap = self.editor_keymap.clone();
        self.input_with_keymap(event, &keymap);
    }

    fn handle_vim_normal(&mut self, event: KeyEvent) {
        let pending = std::mem::replace(&mut self.vim_pending, VimPending::None);
        match pending {
            VimPending::None => {}
            VimPending::Operator(op) => {
                self.handle_vim_operator(op, event);
                return;
            }
            VimPending::TextObject { operator, scope } => {
                self.handle_vim_text_object(operator, scope, event);
                return;
            }
        }

        if self.vim_normal_keymap.enter_insert.is_pressed(event) {
            self.vim_mode = VimMode::Insert;
            return;
        }
        if self.vim_normal_keymap.append_after_cursor.is_pressed(event) {
            let next = self.next_atomic_boundary(self.cursor_pos);
            self.set_cursor(next);
            self.vim_mode = VimMode::Insert;
            return;
        }
        if self.vim_normal_keymap.append_line_end.is_pressed(event) {
            self.set_cursor(self.end_of_current_line());
            self.vim_mode = VimMode::Insert;
            return;
        }
        if self.vim_normal_keymap.insert_line_start.is_pressed(event) {
            self.set_cursor(self.first_non_blank_of_current_line());
            self.vim_mode = VimMode::Insert;
            return;
        }
        if self.vim_normal_keymap.open_line_below.is_pressed(event) {
            let eol = self.end_of_current_line();
            let old_len = self.text.len();
            let insert_at = if eol < old_len { eol + 1 } else { eol };
            self.insert_str_at(insert_at, "\n");
            let cursor = if eol < old_len {
                insert_at
            } else {
                insert_at + 1
            };
            self.set_cursor(cursor);
            self.vim_mode = VimMode::Insert;
            return;
        }
        if self.vim_normal_keymap.open_line_above.is_pressed(event) {
            let bol = self.beginning_of_current_line();
            self.insert_str_at(bol, "\n");
            self.set_cursor(bol);
            self.vim_mode = VimMode::Insert;
            return;
        }
        if self.vim_normal_keymap.move_left.is_pressed(event) {
            self.move_cursor_left();
            return;
        }
        if self.vim_normal_keymap.move_right.is_pressed(event) {
            self.move_cursor_right();
            return;
        }
        if self.vim_normal_keymap.move_down.is_pressed(event) {
            self.move_cursor_down();
            return;
        }
        if self.vim_normal_keymap.move_up.is_pressed(event) {
            self.move_cursor_up();
            return;
        }
        if self.vim_normal_keymap.move_word_forward.is_pressed(event) {
            self.set_cursor(self.beginning_of_next_word());
            return;
        }
        if self.vim_normal_keymap.move_word_backward.is_pressed(event) {
            self.set_cursor(self.beginning_of_previous_word());
            return;
        }
        if self.vim_normal_keymap.move_word_end.is_pressed(event) {
            self.set_cursor(self.vim_word_end_cursor());
            return;
        }
        if self.vim_normal_keymap.move_line_start.is_pressed(event) {
            self.set_cursor(self.beginning_of_current_line());
            return;
        }
        if self.vim_normal_keymap.move_line_end.is_pressed(event) {
            self.set_cursor(self.vim_line_end_cursor());
            return;
        }
        if self.vim_normal_keymap.delete_char.is_pressed(event) {
            self.delete_forward_kill(/*n*/ 1);
            return;
        }
        if self.vim_normal_keymap.substitute_char.is_pressed(event) {
            if self.cursor_pos < self.end_of_current_line() {
                self.delete_forward_kill(/*n*/ 1);
            }
            self.vim_mode = VimMode::Insert;
            return;
        }
        if self.vim_normal_keymap.delete_to_line_end.is_pressed(event) {
            self.vim_kill_to_end_of_line();
            return;
        }
        if self.vim_normal_keymap.change_to_line_end.is_pressed(event) {
            self.vim_kill_to_end_of_line();
            self.vim_mode = VimMode::Insert;
            return;
        }
        if self.vim_normal_keymap.yank_line.is_pressed(event) {
            self.yank_current_line();
            return;
        }
        if self.vim_normal_keymap.paste_after.is_pressed(event) {
            self.paste_after_cursor();
            return;
        }
        if self
            .vim_normal_keymap
            .start_delete_operator
            .is_pressed(event)
        {
            self.vim_pending = VimPending::Operator(VimOperator::Delete);
            return;
        }
        if self.vim_normal_keymap.start_yank_operator.is_pressed(event) {
            self.vim_pending = VimPending::Operator(VimOperator::Yank);
            return;
        }
        if self
            .vim_normal_keymap
            .start_change_operator
            .is_pressed(event)
        {
            self.vim_pending = VimPending::Operator(VimOperator::Change);
            return;
        }
        if self.vim_normal_keymap.cancel_operator.is_pressed(event) {
            self.vim_pending = VimPending::None;
        }
    }

    fn handle_vim_operator(&mut self, op: VimOperator, event: KeyEvent) -> bool {
        if op == VimOperator::Delete && self.vim_operator_keymap.delete_line.is_pressed(event) {
            self.kill_current_line();
            return true;
        }
        if op == VimOperator::Yank && self.vim_operator_keymap.yank_line.is_pressed(event) {
            self.yank_current_line();
            return true;
        }
        if self.vim_operator_keymap.cancel.is_pressed(event) {
            return true;
        }
        if let Some(scope) = self.vim_text_object_scope_for_event(event) {
            self.vim_pending = VimPending::TextObject {
                operator: op,
                scope,
            };
            return true;
        }

        if op != VimOperator::Change
            && let Some(motion) = self.vim_motion_for_event(event)
        {
            self.apply_vim_operator(op, motion);
            return true;
        }
        false
    }

    fn handle_vim_text_object(
        &mut self,
        op: VimOperator,
        scope: VimTextObjectScope,
        event: KeyEvent,
    ) -> bool {
        if self.vim_text_object_keymap.cancel.is_pressed(event) {
            return true;
        }
        let Some(object) = self.vim_text_object_for_event(event) else {
            return false;
        };
        if let Some(range) = self.text_object_range(object, scope) {
            self.apply_vim_operator_to_range(op, range);
        }
        true
    }

    fn vim_motion_for_event(&self, event: KeyEvent) -> Option<VimMotion> {
        if self.vim_operator_keymap.motion_left.is_pressed(event) {
            return Some(VimMotion::Left);
        }
        if self.vim_operator_keymap.motion_right.is_pressed(event) {
            return Some(VimMotion::Right);
        }
        if self.vim_operator_keymap.motion_down.is_pressed(event) {
            return Some(VimMotion::Down);
        }
        if self.vim_operator_keymap.motion_up.is_pressed(event) {
            return Some(VimMotion::Up);
        }
        if self
            .vim_operator_keymap
            .motion_word_forward
            .is_pressed(event)
        {
            return Some(VimMotion::WordForward);
        }
        if self
            .vim_operator_keymap
            .motion_word_backward
            .is_pressed(event)
        {
            return Some(VimMotion::WordBackward);
        }
        if self.vim_operator_keymap.motion_word_end.is_pressed(event) {
            return Some(VimMotion::WordEnd);
        }
        if self.vim_operator_keymap.motion_line_start.is_pressed(event) {
            return Some(VimMotion::LineStart);
        }
        if self.vim_operator_keymap.motion_line_end.is_pressed(event) {
            return Some(VimMotion::LineEnd);
        }
        None
    }

    fn apply_vim_operator(&mut self, op: VimOperator, motion: VimMotion) {
        let Some(range) = self.range_for_motion(motion) else {
            return;
        };
        match op {
            VimOperator::Delete => self.kill_range(range),
            VimOperator::Yank => self.yank_range(range),
            VimOperator::Change => {}
        }
    }

    fn apply_vim_operator_to_range(&mut self, op: VimOperator, range: Range<usize>) {
        match op {
            VimOperator::Delete => self.kill_range(range),
            VimOperator::Yank => self.yank_range(range),
            VimOperator::Change => {
                self.kill_range(range);
                self.vim_mode = VimMode::Insert;
            }
        }
    }

    fn range_for_motion(&mut self, motion: VimMotion) -> Option<Range<usize>> {
        if matches!(motion, VimMotion::Up | VimMotion::Down) {
            return self.linewise_range_for_vertical_motion(motion);
        }
        let start = self.cursor_pos;
        let target = self.target_for_motion(motion);
        if start == target {
            return None;
        }
        let (range_start, range_end) = if target < start {
            (target, start)
        } else {
            (start, target)
        };
        Some(range_start..range_end)
    }

    fn linewise_range_for_vertical_motion(&self, motion: VimMotion) -> Option<Range<usize>> {
        let current = self.current_line_range_with_newline();
        let range = match motion {
            VimMotion::Up => {
                let start = if current.start == 0 {
                    current.start
                } else {
                    self.beginning_of_line(current.start.saturating_sub(1))
                };
                start..current.end
            }
            VimMotion::Down => {
                let end = if current.end >= self.text.len() {
                    current.end
                } else {
                    let next_eol = self.end_of_line(current.end);
                    if next_eol < self.text.len() {
                        next_eol + 1
                    } else {
                        next_eol
                    }
                };
                current.start..end
            }
            VimMotion::Left
            | VimMotion::Right
            | VimMotion::WordForward
            | VimMotion::WordBackward
            | VimMotion::WordEnd
            | VimMotion::LineStart
            | VimMotion::LineEnd => return None,
        };
        (range.start < range.end).then_some(range)
    }

    fn target_for_motion(&mut self, motion: VimMotion) -> usize {
        let original_cursor = self.cursor_pos;
        let original_preferred = self.preferred_col;
        match motion {
            VimMotion::Left => self.move_cursor_left(),
            VimMotion::Right => self.move_cursor_right(),
            VimMotion::Up => self.move_cursor_up(),
            VimMotion::Down => self.move_cursor_down(),
            VimMotion::WordForward => self.set_cursor(self.beginning_of_next_word()),
            VimMotion::WordBackward => self.set_cursor(self.beginning_of_previous_word()),
            VimMotion::WordEnd => self.set_cursor(self.vim_word_end_exclusive()),
            VimMotion::LineStart => self.set_cursor(self.beginning_of_current_line()),
            VimMotion::LineEnd => self.set_cursor(self.end_of_current_line()),
        }
        let target = self.cursor_pos;
        self.cursor_pos = original_cursor;
        self.preferred_col = original_preferred;
        target
    }

    // ####### 输入函数 #######
    pub fn delete_backward(&mut self, n: usize) {
        if n == 0 || self.cursor_pos == 0 {
            return;
        }
        let mut target = self.cursor_pos;
        for _ in 0..n {
            target = self.prev_atomic_boundary(target);
            if target == 0 {
                break;
            }
        }
        self.replace_range(target..self.cursor_pos, "");
    }

    pub fn delete_forward(&mut self, n: usize) {
        if n == 0 || self.cursor_pos >= self.text.len() {
            return;
        }
        let mut target = self.cursor_pos;
        for _ in 0..n {
            target = self.next_atomic_boundary(target);
            if target >= self.text.len() {
                break;
            }
        }
        self.replace_range(self.cursor_pos..target, "");
    }

    pub fn delete_forward_kill(&mut self, n: usize) {
        if n == 0 || self.cursor_pos >= self.text.len() {
            return;
        }
        let mut target = self.cursor_pos;
        for _ in 0..n {
            target = self.next_atomic_boundary(target);
            if target >= self.text.len() {
                break;
            }
        }
        self.kill_range(self.cursor_pos..target);
    }

    pub fn delete_backward_word(&mut self) {
        let start = self.beginning_of_previous_word();
        self.kill_range(start..self.cursor_pos);
    }

    /// 使用「词」语义删除光标右侧的文本。
    ///
    /// 从当前光标位置删除到由 `end_of_next_word()` 确定的下一词末尾。光标与该词之间的
    /// 空白（包括换行）也会一并删除。
    pub fn delete_forward_word(&mut self) {
        let end = self.end_of_next_word();
        if end > self.cursor_pos {
            self.kill_range(self.cursor_pos..end);
        }
    }

    /// 从光标杀入到当前逻辑行的末尾。
    ///
    /// 如果光标已在行尾且存在行尾换行符，本方法会杀入该换行符，以便重复调用能持续推进。
    /// 被移除的文本会成为下一个 yank 目标，即使调用方之后通过 `set_text_*` 清除或重写可见
    /// 缓冲区，它仍然可用。
    pub fn kill_to_end_of_line(&mut self) {
        let eol = self.end_of_current_line();
        let range = if self.cursor_pos == eol {
            if eol < self.text.len() {
                Some(self.cursor_pos..eol + 1)
            } else {
                None
            }
        } else {
            Some(self.cursor_pos..eol)
        };

        if let Some(range) = range {
            self.kill_range(range);
        }
    }

    fn vim_kill_to_end_of_line(&mut self) {
        let eol = self.end_of_current_line();
        if self.cursor_pos < eol {
            self.kill_range(self.cursor_pos..eol);
        }
    }

    pub fn kill_to_beginning_of_line(&mut self) {
        let bol = self.beginning_of_current_line();
        let range = if self.cursor_pos == bol {
            if bol > 0 { Some(bol - 1..bol) } else { None }
        } else {
            Some(bol..self.cursor_pos)
        };

        if let Some(range) = range {
            self.kill_range(range);
        }
    }

    /// 在光标处插入最近一次被杀的文本。
    ///
    /// 这使用 textarea 的单条目杀入缓冲区。由于整缓冲区替换 API 不会清除该缓冲区，因此
    /// 在 composer 层的清除（如提交和 slash 命令分发）之后，`yank` 仍能恢复文本。
    pub fn yank(&mut self) {
        if self.kill_buffer.is_empty() {
            return;
        }
        let text = self.kill_buffer.clone();
        self.insert_str(&text);
    }

    fn kill_range(&mut self, range: Range<usize>) {
        self.kill_range_with_kind(range, KillBufferKind::Characterwise);
    }

    fn kill_line_range(&mut self, range: Range<usize>) {
        self.kill_range_with_kind(range, KillBufferKind::Linewise);
    }

    fn kill_range_with_kind(&mut self, range: Range<usize>, kind: KillBufferKind) {
        let range = self.expand_range_to_element_boundaries(range);
        if range.start >= range.end {
            return;
        }

        let removed = self.text[range.clone()].to_string();
        if removed.is_empty() {
            return;
        }

        self.store_kill_buffer(removed, kind);
        self.replace_range_raw(range, "");
    }

    fn yank_range(&mut self, range: Range<usize>) {
        self.yank_range_with_kind(range, KillBufferKind::Characterwise);
    }

    fn yank_line_range(&mut self, range: Range<usize>) {
        self.yank_range_with_kind(range, KillBufferKind::Linewise);
    }

    fn yank_range_with_kind(&mut self, range: Range<usize>, kind: KillBufferKind) {
        let range = self.expand_range_to_element_boundaries(range);
        if range.start >= range.end {
            return;
        }
        let removed = self.text[range].to_string();
        if removed.is_empty() {
            return;
        }
        self.store_kill_buffer(removed, kind);
    }

    fn store_kill_buffer(&mut self, text: String, kind: KillBufferKind) {
        self.kill_buffer = text;
        self.kill_buffer_kind = kind;
    }

    fn paste_after_cursor(&mut self) {
        if self.kill_buffer.is_empty() {
            return;
        }
        if self.kill_buffer_kind == KillBufferKind::Linewise {
            self.paste_line_after_current_line();
            return;
        }
        let insert_at = self.next_atomic_boundary(self.cursor_pos);
        self.set_cursor(insert_at);
        let text = self.kill_buffer.clone();
        self.insert_str(&text);
    }

    fn paste_line_after_current_line(&mut self) {
        let eol = self.end_of_current_line();
        let insert_at = if eol < self.text.len() { eol + 1 } else { eol };
        let cursor = if eol < self.text.len() {
            insert_at
        } else {
            insert_at + 1
        };
        let text = if eol < self.text.len() {
            if self.kill_buffer.ends_with('\n') {
                self.kill_buffer.clone()
            } else {
                format!("{}\n", self.kill_buffer)
            }
        } else {
            format!("\n{}", self.kill_buffer.trim_end_matches('\n'))
        };
        self.insert_str_at(insert_at, &text);
        self.set_cursor(cursor.min(self.text.len()));
    }

    fn yank_current_line(&mut self) {
        let range = self.current_line_range_with_newline();
        self.yank_line_range(range);
    }

    fn kill_current_line(&mut self) {
        let range = self.current_line_range_with_newline();
        self.kill_line_range(range);
    }

    fn current_line_range_with_newline(&self) -> Range<usize> {
        let bol = self.beginning_of_current_line();
        let eol = self.end_of_current_line();
        let end = if eol < self.text.len() { eol + 1 } else { eol };
        bol..end
    }

    /// 将光标向左移动一个字素簇。
    pub fn move_cursor_left(&mut self) {
        self.cursor_pos = self.prev_atomic_boundary(self.cursor_pos);
        self.preferred_col = None;
    }

    /// 将光标向右移动一个字素簇。
    pub fn move_cursor_right(&mut self) {
        self.cursor_pos = self.next_atomic_boundary(self.cursor_pos);
        self.preferred_col = None;
    }

    pub fn move_cursor_up(&mut self) {
        // 如果存在换行缓存，优先在换行后的（视觉）行之间导航。
        if let Some((target_col, maybe_line)) = {
            let cache_ref = self.wrap_cache.borrow();
            if let Some(cache) = cache_ref.as_ref() {
                let lines = &cache.lines;
                if let Some(idx) = Self::wrapped_line_index_by_start(lines, self.cursor_pos) {
                    let cur_range = &lines[idx];
                    let target_col = self
                        .preferred_col
                        .unwrap_or_else(|| self.text[cur_range.start..self.cursor_pos].width());
                    if idx > 0 {
                        let prev = &lines[idx - 1];
                        let line_start = prev.start;
                        let line_end = prev.end.saturating_sub(1);
                        Some((target_col, Some((line_start, line_end))))
                    } else {
                        Some((target_col, None))
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } {
            // 已有换行信息，据此应用移动。
            match maybe_line {
                Some((line_start, line_end)) => {
                    if self.preferred_col.is_none() {
                        self.preferred_col = Some(target_col);
                    }
                    self.move_to_display_col_on_line(line_start, line_end, target_col);
                    return;
                }
                None => {
                    // 已在第一个视觉行 -> 移动到开头
                    self.cursor_pos = 0;
                    self.preferred_col = None;
                    return;
                }
            }
        }

        // 若还没有换行信息，则回退到逻辑行导航。
        if let Some(prev_nl) = self.text[..self.cursor_pos].rfind('\n') {
            let target_col = match self.preferred_col {
                Some(c) => c,
                None => {
                    let c = self.current_display_col();
                    self.preferred_col = Some(c);
                    c
                }
            };
            let prev_line_start = self.text[..prev_nl].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let prev_line_end = prev_nl;
            self.move_to_display_col_on_line(prev_line_start, prev_line_end, target_col);
        } else {
            self.cursor_pos = 0;
            self.preferred_col = None;
        }
    }

    pub fn move_cursor_down(&mut self) {
        // 如果存在换行缓存，优先在换行后的（视觉）行之间导航。
        if let Some((target_col, move_to_last)) = {
            let cache_ref = self.wrap_cache.borrow();
            if let Some(cache) = cache_ref.as_ref() {
                let lines = &cache.lines;
                if let Some(idx) = Self::wrapped_line_index_by_start(lines, self.cursor_pos) {
                    let cur_range = &lines[idx];
                    let target_col = self
                        .preferred_col
                        .unwrap_or_else(|| self.text[cur_range.start..self.cursor_pos].width());
                    if idx + 1 < lines.len() {
                        let next = &lines[idx + 1];
                        let line_start = next.start;
                        let line_end = next.end.saturating_sub(1);
                        Some((target_col, Some((line_start, line_end))))
                    } else {
                        Some((target_col, None))
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } {
            match move_to_last {
                Some((line_start, line_end)) => {
                    if self.preferred_col.is_none() {
                        self.preferred_col = Some(target_col);
                    }
                    self.move_to_display_col_on_line(line_start, line_end, target_col);
                    return;
                }
                None => {
                    // 已在最后一个视觉行 -> 移动到末尾
                    self.cursor_pos = self.text.len();
                    self.preferred_col = None;
                    return;
                }
            }
        }

        // 若还没有换行信息，则回退到逻辑行导航。
        let target_col = match self.preferred_col {
            Some(c) => c,
            None => {
                let c = self.current_display_col();
                self.preferred_col = Some(c);
                c
            }
        };
        if let Some(next_nl) = self.text[self.cursor_pos..]
            .find('\n')
            .map(|i| i + self.cursor_pos)
        {
            let next_line_start = next_nl + 1;
            let next_line_end = self.text[next_line_start..]
                .find('\n')
                .map(|i| i + next_line_start)
                .unwrap_or(self.text.len());
            self.move_to_display_col_on_line(next_line_start, next_line_end, target_col);
        } else {
            self.cursor_pos = self.text.len();
            self.preferred_col = None;
        }
    }

    pub fn move_cursor_to_beginning_of_line(&mut self, move_up_at_bol: bool) {
        let bol = self.beginning_of_current_line();
        if move_up_at_bol && self.cursor_pos == bol {
            self.set_cursor(self.beginning_of_line(self.cursor_pos.saturating_sub(1)));
        } else {
            self.set_cursor(bol);
        }
        self.preferred_col = None;
    }

    pub fn move_cursor_to_end_of_line(&mut self, move_down_at_eol: bool) {
        let eol = self.end_of_current_line();
        if move_down_at_eol && self.cursor_pos == eol {
            let next_pos = (self.cursor_pos.saturating_add(1)).min(self.text.len());
            self.set_cursor(self.end_of_line(next_pos));
        } else {
            self.set_cursor(eol);
        }
    }

    // ===== 文本元素支持 =====

    pub fn element_payloads(&self) -> Vec<String> {
        self.elements
            .iter()
            .filter_map(|e| self.text.get(e.range.clone()).map(str::to_string))
            .collect()
    }

    pub fn text_elements(&self) -> Vec<UserTextElement> {
        self.elements
            .iter()
            .map(|e| {
                let placeholder = self.text.get(e.range.clone()).map(str::to_string);
                UserTextElement::new(
                    ByteRange {
                        start: e.range.start,
                        end: e.range.end,
                    },
                    placeholder,
                )
            })
            .collect()
    }

    pub(crate) fn text_element_snapshots(&self) -> Vec<TextElementSnapshot> {
        self.elements
            .iter()
            .filter_map(|element| {
                self.text
                    .get(element.range.clone())
                    .map(|text| TextElementSnapshot {
                        id: element.id,
                        range: element.range.clone(),
                        text: text.to_string(),
                    })
            })
            .collect()
    }

    /// 按起始位置的升序迭代借用的原子元素范围。
    pub(crate) fn text_element_ranges(&self) -> impl Iterator<Item = &Range<usize>> {
        self.elements.iter().map(|element| &element.range)
    }

    /// 迭代与 `range` 有交叠的有序原子元素范围。
    ///
    /// 恰好结束于范围起点或恰好开始于范围终点的元素会被排除。
    pub(crate) fn text_element_ranges_overlapping(
        &self,
        range: Range<usize>,
    ) -> impl Iterator<Item = &Range<usize>> {
        let first = self
            .elements
            .partition_point(|element| element.range.end <= range.start);
        self.elements[first..]
            .iter()
            .take_while(move |element| element.range.start < range.end)
            .map(|element| &element.range)
    }

    pub(crate) fn element_id_for_exact_range(&self, range: Range<usize>) -> Option<u64> {
        self.elements
            .iter()
            .find(|element| element.range == range)
            .map(|element| element.id)
    }

    /// 原地重命名单个文本元素，并保持其原子性。
    ///
    /// 当元素的有效内容是一个标识符（例如占位符）且必须在不把元素还原成普通文本的情况下
    /// 更新时，请使用此方法。
    pub fn replace_element_payload(&mut self, old: &str, new: &str) -> bool {
        let Some(idx) = self
            .elements
            .iter()
            .position(|e| self.text.get(e.range.clone()) == Some(old))
        else {
            return false;
        };

        let range = self.elements[idx].range.clone();
        let start = range.start;
        let end = range.end;
        if start > end || end > self.text.len() {
            return false;
        }

        let removed_len = end - start;
        let inserted_len = new.len();
        let diff = inserted_len as isize - removed_len as isize;

        self.text.replace_range(range, new);
        self.wrap_cache.replace(None);
        self.preferred_col = None;

        // 更新被修改元素的范围。
        self.elements[idx].range = start..(start + inserted_len);

        // 偏移位于被替换元素之后的所有元素范围。
        if diff != 0 {
            for (j, e) in self.elements.iter_mut().enumerate() {
                if j == idx {
                    continue;
                }
                if e.range.end <= start {
                    continue;
                }
                if e.range.start >= end {
                    e.range.start = ((e.range.start as isize) + diff) as usize;
                    e.range.end = ((e.range.end as isize) + diff) as usize;
                    continue;
                }

                // 元素之间不应部分交叠；将任何与被替换范围相交的元素就近吸附到新边界，
                // 以便优雅降级。
                e.range.start = start.min(e.range.start);
                e.range.end = (start + inserted_len).max(e.range.end.saturating_add_signed(diff));
            }
        }

        // 根据本次编辑更新光标位置。
        self.cursor_pos = if self.cursor_pos < start {
            self.cursor_pos
        } else if self.cursor_pos <= end {
            start + inserted_len
        } else {
            ((self.cursor_pos as isize) + diff) as usize
        };
        self.cursor_pos = self.clamp_pos_to_nearest_boundary(self.cursor_pos);

        // 保持元素顺序的确定性。
        self.elements.sort_by_key(|e| e.range.start);

        true
    }

    pub fn insert_element(&mut self, text: &str) -> u64 {
        let start = self.clamp_pos_for_insertion(self.cursor_pos);
        self.insert_str_at(start, text);
        let end = start + text.len();
        let id = self.add_element(start..end);
        // 将光标放在插入元素之后
        self.set_cursor(end);
        id
    }

    fn add_element(&mut self, range: Range<usize>) -> u64 {
        let id = self.next_element_id();
        self.elements.push(TextElement { id, range });
        self.elements.sort_by_key(|e| e.range.start);
        id
    }

    /// 在不改动文本的前提下，将一段已有文本范围标记为原子元素。
    ///
    /// 这用于把已输入的 token（如 `/plan`）转换为元素，使其以原子方式渲染和编辑。
    /// 交叠或重复的范围会被忽略。
    pub fn add_element_range(&mut self, range: Range<usize>) -> Option<u64> {
        let start = self.clamp_pos_to_char_boundary(range.start.min(self.text.len()));
        let end = self.clamp_pos_to_char_boundary(range.end.min(self.text.len()));
        if start >= end {
            return None;
        }
        if self
            .elements
            .iter()
            .any(|e| e.range.start == start && e.range.end == end)
        {
            return None;
        }
        if self
            .elements
            .iter()
            .any(|e| start < e.range.end && end > e.range.start)
        {
            return None;
        }
        let id = self.add_element(start..end);
        Some(id)
    }

    pub fn remove_element_range(&mut self, range: Range<usize>) -> bool {
        let start = self.clamp_pos_to_char_boundary(range.start.min(self.text.len()));
        let end = self.clamp_pos_to_char_boundary(range.end.min(self.text.len()));
        if start >= end {
            return false;
        }
        let len_before = self.elements.len();
        self.elements
            .retain(|elem| elem.range.start != start || elem.range.end != end);
        len_before != self.elements.len()
    }

    fn next_element_id(&mut self) -> u64 {
        let id = self.next_element_id;
        self.next_element_id = self.next_element_id.saturating_add(1);
        id
    }
    fn find_element_containing(&self, pos: usize) -> Option<usize> {
        self.elements
            .iter()
            .position(|e| pos > e.range.start && pos < e.range.end)
    }

    fn clamp_pos_to_char_boundary(&self, pos: usize) -> usize {
        let pos = pos.min(self.text.len());
        if self.text.is_char_boundary(pos) {
            return pos;
        }
        let mut prev = pos;
        while prev > 0 && !self.text.is_char_boundary(prev) {
            prev -= 1;
        }
        let mut next = pos;
        while next < self.text.len() && !self.text.is_char_boundary(next) {
            next += 1;
        }
        if pos.saturating_sub(prev) <= next.saturating_sub(pos) {
            prev
        } else {
            next
        }
    }

    fn clamp_pos_to_nearest_boundary(&self, pos: usize) -> usize {
        let pos = self.clamp_pos_to_char_boundary(pos);
        if let Some(idx) = self.find_element_containing(pos) {
            let e = &self.elements[idx];
            let dist_start = pos.saturating_sub(e.range.start);
            let dist_end = e.range.end.saturating_sub(pos);
            if dist_start <= dist_end {
                self.clamp_pos_to_char_boundary(e.range.start)
            } else {
                self.clamp_pos_to_char_boundary(e.range.end)
            }
        } else {
            pos
        }
    }

    fn clamp_pos_for_insertion(&self, pos: usize) -> usize {
        let pos = self.clamp_pos_to_char_boundary(pos);
        // 不允许插入到元素中间
        if let Some(idx) = self.find_element_containing(pos) {
            let e = &self.elements[idx];
            // 为插入选择最近的边缘
            let dist_start = pos.saturating_sub(e.range.start);
            let dist_end = e.range.end.saturating_sub(pos);
            if dist_start <= dist_end {
                self.clamp_pos_to_char_boundary(e.range.start)
            } else {
                self.clamp_pos_to_char_boundary(e.range.end)
            }
        } else {
            pos
        }
    }

    fn expand_range_to_element_boundaries(&self, mut range: Range<usize>) -> Range<usize> {
        // 扩展到完整包含所有相交的元素
        loop {
            let mut changed = false;
            for e in &self.elements {
                if e.range.start < range.end && e.range.end > range.start {
                    let new_start = range.start.min(e.range.start);
                    let new_end = range.end.max(e.range.end);
                    if new_start != range.start || new_end != range.end {
                        range.start = new_start;
                        range.end = new_end;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        range
    }

    fn shift_elements(&mut self, at: usize, removed: usize, inserted: usize) {
        // 通用偏移：纯插入时 removed = 0；删除时 inserted = 0。
        let end = at + removed;
        let diff = inserted as isize - removed as isize;
        // 移除被操作完全删除的元素，并偏移其余元素
        self.elements
            .retain(|e| !(e.range.start >= at && e.range.end <= end));
        for e in &mut self.elements {
            if e.range.end <= at {
                // 编辑前
            } else if e.range.start >= end {
                // 编辑后
                e.range.start = ((e.range.start as isize) + diff) as usize;
                e.range.end = ((e.range.end as isize) + diff) as usize;
            } else {
                // 与元素交叠但未完全包含（使用元素感知的替换时不应发生，
                // 但仍通过将元素吸附到新边界来优雅降级）
                let new_start = at.min(e.range.start);
                let new_end = at + inserted.max(e.range.end.saturating_sub(end));
                e.range.start = new_start;
                e.range.end = new_end;
            }
        }
    }

    fn update_elements_after_replace(&mut self, start: usize, end: usize, inserted_len: usize) {
        self.shift_elements(start, end.saturating_sub(start), inserted_len);
    }

    fn prev_atomic_boundary(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        // 如果当前位置在元素末尾或元素内部，跳到该元素的起点。
        if let Some(idx) = self
            .elements
            .iter()
            .position(|e| pos > e.range.start && pos <= e.range.end)
        {
            return self.elements[idx].range.start;
        }
        let mut gc = unicode_segmentation::GraphemeCursor::new(pos, self.text.len(), false);
        match gc.prev_boundary(&self.text, 0) {
            Ok(Some(b)) => {
                if let Some(idx) = self.find_element_containing(b) {
                    self.elements[idx].range.start
                } else {
                    b
                }
            }
            Ok(None) => 0,
            Err(_) => pos.saturating_sub(1),
        }
    }

    fn next_atomic_boundary(&self, pos: usize) -> usize {
        if pos >= self.text.len() {
            return self.text.len();
        }
        // 如果当前位置在元素起点或元素内部，跳到该元素的终点。
        if let Some(idx) = self
            .elements
            .iter()
            .position(|e| pos >= e.range.start && pos < e.range.end)
        {
            return self.elements[idx].range.end;
        }
        let mut gc = unicode_segmentation::GraphemeCursor::new(pos, self.text.len(), false);
        match gc.next_boundary(&self.text, 0) {
            Ok(Some(b)) => {
                if let Some(idx) = self.find_element_containing(b) {
                    self.elements[idx].range.end
                } else {
                    b
                }
            }
            Ok(None) => self.text.len(),
            Err(_) => pos.saturating_add(1),
        }
    }

    pub(crate) fn beginning_of_previous_word(&self) -> usize {
        let prefix = &self.text[..self.cursor_pos];
        let Some((first_non_ws_idx, ch)) = prefix
            .char_indices()
            .rev()
            .find(|&(_, ch)| !ch.is_whitespace())
        else {
            return 0;
        };
        let run_start = prefix[..first_non_ws_idx]
            .char_indices()
            .rev()
            .find(|&(_, ch)| ch.is_whitespace())
            .map_or(0, |(idx, ch)| idx + ch.len_utf8());
        let run_end = first_non_ws_idx + ch.len_utf8();
        let pieces = split_word_pieces(&prefix[run_start..run_end]);
        let mut pieces = pieces.into_iter().rev().peekable();
        let Some((piece_start, piece)) = pieces.next() else {
            return run_start;
        };
        let mut start = run_start + piece_start;

        if piece.chars().all(is_word_separator) {
            while let Some((idx, piece)) = pieces.peek() {
                if !piece.chars().all(is_word_separator) {
                    break;
                }
                start = run_start + *idx;
                pieces.next();
            }
        }

        self.adjust_pos_out_of_elements(start, /*prefer_start*/ true)
    }

    pub(crate) fn end_of_next_word(&self) -> usize {
        self.end_of_next_word_from(self.cursor_pos)
    }

    fn end_of_next_word_from(&self, cursor_pos: usize) -> usize {
        let suffix = &self.text[cursor_pos..];
        let Some(first_non_ws) = suffix.find(|ch: char| !ch.is_whitespace()) else {
            return self.text.len();
        };
        let run = &suffix[first_non_ws..];
        let run = &run[..run.find(char::is_whitespace).unwrap_or(run.len())];
        let mut pieces = split_word_pieces(run).into_iter().peekable();
        let Some((start, piece)) = pieces.next() else {
            return cursor_pos + first_non_ws;
        };
        let word_start = cursor_pos + first_non_ws + start;
        let mut end = word_start + piece.len();
        if piece.chars().all(is_word_separator) {
            while let Some((idx, piece)) = pieces.peek() {
                if !piece.chars().all(is_word_separator) {
                    break;
                }
                end = cursor_pos + first_non_ws + *idx + piece.len();
                pieces.next();
            }
        }

        self.adjust_pos_out_of_elements(end, /*prefer_start*/ false)
    }

    fn vim_word_end_exclusive(&self) -> usize {
        let end = self.end_of_next_word();
        let target = if end > self.cursor_pos {
            self.prev_atomic_boundary(end)
        } else {
            end
        };
        if target == self.cursor_pos && end < self.text.len() {
            self.end_of_next_word_from(end)
        } else {
            end
        }
    }

    fn vim_word_end_cursor(&self) -> usize {
        let end = self.vim_word_end_exclusive();
        if end > self.cursor_pos {
            self.prev_atomic_boundary(end)
        } else {
            end
        }
    }

    fn vim_line_end_cursor(&self) -> usize {
        let bol = self.beginning_of_current_line();
        let eol = self.end_of_current_line();
        if eol > bol {
            self.prev_atomic_boundary(eol).max(bol)
        } else {
            eol
        }
    }

    pub(crate) fn beginning_of_next_word(&self) -> usize {
        let Some(first_non_ws) = self.text[self.cursor_pos..].find(|c: char| !c.is_whitespace())
        else {
            return self.text.len();
        };
        let word_start = self.cursor_pos + first_non_ws;
        if word_start != self.cursor_pos {
            return self.adjust_pos_out_of_elements(word_start, /*prefer_start*/ true);
        }
        let end = self.end_of_next_word();
        if end >= self.text.len() {
            return self.text.len();
        }
        let Some(next_non_ws) = self.text[end..].find(|c: char| !c.is_whitespace()) else {
            return self.text.len();
        };
        self.adjust_pos_out_of_elements(end + next_non_ws, /*prefer_start*/ true)
    }

    fn adjust_pos_out_of_elements(&self, pos: usize, prefer_start: bool) -> usize {
        if let Some(idx) = self.find_element_containing(pos) {
            let e = &self.elements[idx];
            if prefer_start {
                e.range.start
            } else {
                e.range.end
            }
        } else {
            pos
        }
    }

    #[expect(clippy::unwrap_used)]
    fn wrapped_lines(&self, width: u16) -> Ref<'_, Vec<Range<usize>>> {
        // 确保缓存就绪（可能进行可变借用，然后立即释放）
        {
            let mut cache = self.wrap_cache.borrow_mut();
            let needs_recalc = match cache.as_ref() {
                Some(c) => c.width != width,
                None => true,
            };
            if needs_recalc {
                let display_text = text_for_display(&self.text);
                let lines = crate::tui_core::wrapping::wrap_ranges(
                    display_text.as_ref(),
                    Options::new(width as usize).wrap_algorithm(textwrap::WrapAlgorithm::FirstFit),
                );
                *cache = Some(WrapCache { width, lines });
            }
        }

        let cache = self.wrap_cache.borrow();
        Ref::map(cache, |c| &c.as_ref().unwrap().lines)
    }

    /// 计算在给定的区域大小与换行行列表下，为满足下列不变量应使用的滚动偏移。
    ///
    /// - 光标始终在屏幕上。
    /// - 内容能放进区域时不滚动。
    fn effective_scroll(
        &self,
        area_height: u16,
        lines: &[Range<usize>],
        current_scroll: u16,
    ) -> u16 {
        let total_lines = lines.len() as u16;
        if area_height >= total_lines {
            return 0;
        }

        // 光标在换行后的行中位于何处？优先将边界位置
        // （pos 等于某个换行行起点处）归属到更靠后的那一行。
        let cursor_line_idx =
            Self::wrapped_line_index_by_start(lines, self.cursor_pos).unwrap_or(0) as u16;

        let max_scroll = total_lines.saturating_sub(area_height);
        let mut scroll = current_scroll.min(max_scroll);

        // 确保光标在 [scroll, scroll + area_height) 范围内可见
        if cursor_line_idx < scroll {
            scroll = cursor_line_idx;
        } else if cursor_line_idx >= scroll + area_height {
            scroll = cursor_line_idx + 1 - area_height;
        }
        scroll
    }
}

impl WidgetRef for &TextArea {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let lines = self.wrapped_lines(area.width);
        self.render_lines(area, buf, &lines, 0..lines.len(), visible_text_style(), &[]);
    }
}

impl StatefulWidgetRef for &TextArea {
    type State = TextAreaState;

    fn render_ref(&self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let lines = self.wrapped_lines(area.width);
        let scroll = self.effective_scroll(area.height, &lines, state.scroll);
        state.scroll = scroll;

        let start = scroll as usize;
        let end = (scroll + area.height).min(lines.len() as u16) as usize;
        self.render_lines(area, buf, &lines, start..end, visible_text_style(), &[]);
    }
}

impl TextArea {
    pub(crate) fn render_ref_masked(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &mut TextAreaState,
        mask_char: char,
    ) {
        let lines = self.wrapped_lines(area.width);
        let scroll = self.effective_scroll(area.height, &lines, state.scroll);
        state.scroll = scroll;

        let start = scroll as usize;
        let end = (scroll + area.height).min(lines.len() as u16) as usize;
        self.render_lines_masked(area, buf, &lines, start..end, mask_char);
    }

    /// 使用 `base_style` 以及额外的仅用于渲染的高亮范围来渲染 textarea。
    ///
    /// 高亮范围是 `self.text` 中的字节范围。它们只影响缓冲区渲染，不会改动可编辑文本、
    /// 光标、元素元数据或换行缓存。
    pub(crate) fn render_ref_styled_with_highlights(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &mut TextAreaState,
        base_style: Style,
        highlights: &[(Range<usize>, Style)],
    ) {
        let lines = self.wrapped_lines(area.width);
        let scroll = self.effective_scroll(area.height, &lines, state.scroll);
        state.scroll = scroll;

        let start = scroll as usize;
        let end = (scroll + area.height).min(lines.len() as u16) as usize;
        self.render_lines(area, buf, &lines, start..end, base_style, highlights);
    }

    fn render_lines(
        &self,
        area: Rect,
        buf: &mut Buffer,
        lines: &[Range<usize>],
        range: std::ops::Range<usize>,
        base_style: Style,
        highlights: &[(Range<usize>, Style)],
    ) {
        for (row, idx) in range.enumerate() {
            let r = &lines[idx];
            let y = area.y + row as u16;
            let line_range = r.start..r.end - 1;
            buf.set_style(Rect::new(area.x, y, area.width, 1), base_style);
            // 用给定的样式绘制基础行。
            buf.set_string(
                area.x,
                y,
                text_for_display(&self.text[line_range.clone()]),
                base_style,
            );

            // 为与该行相交的元素覆盖绘制带样式的片段。
            for elem in &self.elements {
                // 计算与所显示切片的重叠部分。
                let overlap_start = elem.range.start.max(line_range.start);
                let overlap_end = elem.range.end.min(line_range.end);
                if overlap_start >= overlap_end {
                    continue;
                }
                let styled = &self.text[overlap_start..overlap_end];
                let x_off = self.text[line_range.start..overlap_start].width() as u16;
                let style = base_style.fg(Color::Cyan);
                buf.set_string(area.x + x_off, y, text_for_display(styled), style);
            }

            // 最后覆盖绘制仅用于渲染的高亮范围，这样即使临时搜索高亮与附件占位符
            // 或其他带样式的元素相交，也依然可见。
            for (highlight_range, style) in highlights {
                let overlap_start = highlight_range.start.max(line_range.start);
                let overlap_end = highlight_range.end.min(line_range.end);
                if overlap_start >= overlap_end {
                    continue;
                }
                let highlighted = &self.text[overlap_start..overlap_end];
                let x_off = self.text[line_range.start..overlap_start].width() as u16;
                buf.set_string(area.x + x_off, y, text_for_display(highlighted), *style);
            }
        }
    }

    fn render_lines_masked(
        &self,
        area: Rect,
        buf: &mut Buffer,
        lines: &[Range<usize>],
        range: std::ops::Range<usize>,
        mask_char: char,
    ) {
        for (row, idx) in range.enumerate() {
            let r = &lines[idx];
            let y = area.y + row as u16;
            let line_range = r.start..r.end - 1;
            let masked = self.text[line_range.clone()]
                .chars()
                .map(|_| mask_char)
                .collect::<String>();
            buf.set_string(area.x, y, &masked, visible_text_style());
        }
    }
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
