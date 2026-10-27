//! 编辑器侧(Composer-side)的 Ctrl+R 反向历史搜索状态与渲染辅助。
//!
//! 持久化历史与本地历史存储位于 `chat_composer_history`,但编辑器拥有当前搜索会话的所有权,
//! 因为它需要快照/恢复可编辑草稿、在 textarea 中预览匹配项,并在页脚行充当搜索输入时渲染页脚提示。
//!
//! 本模块负责搜索会话面向 UI 的生命周期:识别进入并驱动搜索模式的按键、保持页脚查询与 textarea
//! 预览相互独立、在取消或未命中时恢复原始草稿,以及将历史搜索结果转换为编辑器可见的状态。
//! 它刻意不决定哪些历史条目匹配、如何跳过重复结果、或何时应获取持久化历史;这些遍历不变量
//! 由 `ChatComposerHistory` 负责。
//!
//! 搜索会话以空页脚查询的空闲(idle)状态开始,因此单独打开 Ctrl+R 不会自动预览最新的历史条目。
//! 输入查询会从最新到最旧重新开始遍历,重复按 Ctrl+R/Up 和 Ctrl+S/Down 会在不同匹配项之间移动,
//! `Enter` 将当前预览作为可编辑草稿接受,`Esc` 或 Ctrl+C 则恢复搜索开始前存在的精确草稿。

use std::ops::Range;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;

use super::super::chat_composer_history::HistorySearchDirection;
use super::super::chat_composer_history::HistorySearchResult;
use super::super::footer::footer_height;
use super::super::footer::reset_mode_after_activity;
use super::ActivePopup;
use super::ChatComposer;
use super::ComposerDraft;
use super::InputResult;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::key_hint::has_ctrl_or_alt;
use crate::tui_core::ui_consts::FOOTER_INDENT_COLS;

/// 一次 Ctrl+R 搜索交互中,由编辑器拥有的活动状态。
///
/// 该会话仅由 [`ChatComposer::begin_history_search`] 创建,并且仅在接受、取消或替换搜索模式时清除。
/// 它将原始草稿与页脚查询分开存储,这样临时预览永远不会破坏用户正在进行的编辑器内容。
#[derive(Clone, Debug)]
pub(super) struct HistorySearchSession {
    /// 当搜索被取消或查询无匹配时要恢复的草稿。
    original_draft: ComposerDraft,
    /// Ctrl+R 搜索激活期间键入的、由页脚持有的查询文本。
    query: String,
    /// 用户可见的搜索状态,用于选择页脚提示和编辑器预览行为。
    status: HistorySearchStatus,
}

/// 活动中的 Ctrl+R 搜索会话的用户可见阶段。
///
/// 搜索保持页脚查询与编辑器预览相互独立:`Idle` 保持原始草稿不变,`Searching` 等待持久化历史,
/// `Match` 预览找到的条目,`NoMatch` 恢复原始草稿,同时让搜索输入保持打开以便继续输入。
#[derive(Clone, Debug)]
enum HistorySearchStatus {
    Idle,
    Searching,
    Match,
    NoMatch,
}

impl ChatComposer {
    #[cfg(test)]
    pub(super) fn history_search_active(&self) -> bool {
        self.history_search.is_some()
    }

    /// 返回某个按键事件是否应打开反向历史搜索,或步进到更旧的匹配项。
    ///
    /// 该检查同时接受正常的 Ctrl+R 上报,以及某些终端发出的原始控制字符变体。
    /// 调用方只应在通用文本处理之前使用此检查;若把原始控制字符当作普通输入,
    /// 会在搜索查询或编辑草稿中插入一个不可见的字节。
    pub(super) fn is_history_search_key(key_event: &KeyEvent, bindings: &[KeyBinding]) -> bool {
        bindings.is_pressed(*key_event)
    }

    fn is_history_search_forward_key(key_event: &KeyEvent, bindings: &[KeyBinding]) -> bool {
        bindings.is_pressed(*key_event)
    }

    /// 打开由页脚持有的反向历史搜索,但暂不预览任何历史。
    ///
    /// 进入搜索模式时,首先刷新待处理的粘贴突发(paste-burst)文本,然后快照完整的编辑器草稿,
    /// 清除任何文件/搜索弹窗状态,并重置历史遍历。只有在页脚查询变为非空之后才产生第一个可见匹配,
    /// 这样可以避免在用户尚未搜索任何内容时,Ctrl+R 就用最新提示词替换空白的编辑器。
    pub(super) fn begin_history_search(&mut self) -> (InputResult, bool) {
        if let Some(pasted) = self.draft.paste_burst.flush_before_modified_input() {
            self.handle_paste(pasted);
        }
        self.draft.paste_burst.clear_window_after_non_char();

        if self.popups.current_file_query.is_some() {
            self.app_event_tx
                .send(AppEvent::StartFileSearch(String::new()));
            self.popups.current_file_query = None;
        }
        self.popups.active = ActivePopup::None;
        self.attachments.clear_remote_image_selection();
        self.history_search = Some(HistorySearchSession {
            original_draft: self.snapshot_draft(),
            query: String::new(),
            status: HistorySearchStatus::Idle,
        });
        self.history.reset_search();
        (InputResult::None, true)
    }

    /// 当页脚充当历史搜索输入时,处理每一个按键。
    ///
    /// 该方法会在普通编辑器编辑看到这些键之前先消费搜索模式按键。它保证 `Esc` 和 Ctrl+C 恢复原始草稿,
    /// `Enter` 只接受真正的匹配,普通字符编辑页脚查询,导航键则将遍历委托给 `ChatComposerHistory`。
    /// 在没有搜索会话时调用此方法,对会被忽略的按键是无害的,但会使查询编辑分支成为空操作,
    /// 因此只应在已确认 `history_search.is_some()` 之后路由到这里。
    pub(super) fn handle_history_search_key(&mut self, key_event: KeyEvent) -> (InputResult, bool) {
        if key_event.kind == KeyEventKind::Release {
            return (InputResult::None, false);
        }

        if Self::is_history_search_key(&key_event, &self.history_search_previous_keys)
            || matches!(key_event.code, KeyCode::Up)
        {
            let result = self.history_search_in_direction(HistorySearchDirection::Older);
            return (result, true);
        }

        if Self::is_history_search_forward_key(&key_event, &self.history_search_next_keys)
            || matches!(key_event.code, KeyCode::Down)
        {
            let result = self.history_search_in_direction(HistorySearchDirection::Newer);
            return (result, true);
        }

        match key_event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.cancel_history_search();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) && c.eq_ignore_ascii_case(&'c') => {
                self.cancel_history_search();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Char('\u{0003}'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.cancel_history_search();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if self
                    .history_search
                    .as_ref()
                    .is_some_and(|search| matches!(search.status, HistorySearchStatus::Match))
                {
                    self.history_search = None;
                    self.history.reset_search();
                    self.footer.mode = reset_mode_after_activity(self.footer.mode);
                    self.move_cursor_to_end();
                }
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                if let Some(search) = self.history_search.as_ref() {
                    let mut query = search.query.clone();
                    query.pop();
                    self.update_history_search_query(query);
                }
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.update_history_search_query(String::new());
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Char(ch),
                modifiers,
                ..
            } if !has_ctrl_or_alt(modifiers) => {
                if let Some(search) = self.history_search.as_ref() {
                    let mut query = search.query.clone();
                    query.push(ch);
                    self.update_history_search_query(query);
                }
                (InputResult::None, true)
            }
            _ => (InputResult::None, true),
        }
    }

    fn history_search_in_direction(&mut self, direction: HistorySearchDirection) -> InputResult {
        let Some((query, original_draft)) = self
            .history_search
            .as_ref()
            .map(|search| (search.query.clone(), search.original_draft.clone()))
        else {
            return InputResult::None;
        };
        if query.is_empty() {
            self.history.reset_search();
            if let Some(search) = self.history_search.as_mut() {
                search.status = HistorySearchStatus::Idle;
            }
            self.restore_draft(original_draft);
            return InputResult::None;
        }
        let result = self.history.search(
            &query,
            direction,
            /*restart*/ false,
            &self.app_event_tx,
        );
        self.apply_history_search_result(result);
        InputResult::None
    }

    fn update_history_search_query(&mut self, query: String) {
        let Some(original_draft) = self
            .history_search
            .as_ref()
            .map(|search| search.original_draft.clone())
        else {
            return;
        };
        if let Some(search) = self.history_search.as_mut() {
            search.query = query.clone();
            search.status = HistorySearchStatus::Searching;
        }
        self.restore_draft(original_draft);
        if query.is_empty() {
            self.history.reset_search();
            if let Some(search) = self.history_search.as_mut() {
                search.status = HistorySearchStatus::Idle;
            }
            return;
        }
        let result = self.history.search(
            &query,
            HistorySearchDirection::Older,
            /*restart*/ true,
            &self.app_event_tx,
        );
        self.apply_history_search_result(result);
    }

    /// 取消进行中的历史搜索,并恢复搜索模式打开之前的草稿。
    ///
    /// 这会同时清除普通的历史导航与搜索遍历,因为预览匹配项会临时更新共享的历史光标。
    /// 处理全局取消(如 Ctrl+C)的调用方,应使用布尔返回值来消费该按键,而不要同时清除已恢复的
    /// 草稿或触发退出/中断行为。
    pub(crate) fn cancel_history_search(&mut self) -> bool {
        let Some(search) = self.history_search.take() else {
            return false;
        };
        self.history.reset_navigation();
        self.footer.mode = reset_mode_after_activity(self.footer.mode);
        self.restore_draft(search.original_draft);
        true
    }

    /// 将遍历结果应用到 composer 预览和搜索状态。
    ///
    /// `Found` 预览匹配条目，`Pending` 在异步持久条目查找未完成时
    /// 让页脚保持等待状态，`AtBoundary` 保留当前匹配，
    /// `NotFound` 恢复原始草稿同时保留查询供进一步编辑，
    /// `Unavailable` 执行同样操作但不声称没有匹配。把 `AtBoundary`
    /// 当作 `NotFound` 处理会在单项结果的末尾产生可见的“无匹配”闪烁，
    /// 并使向上/向下计数失步。
    pub(super) fn apply_history_search_result(&mut self, result: HistorySearchResult) {
        match result {
            HistorySearchResult::Found(entry) => {
                if let Some(search) = self.history_search.as_mut() {
                    search.status = HistorySearchStatus::Match;
                }
                self.apply_history_entry(entry);
            }
            HistorySearchResult::Pending => {
                if let Some(search) = self.history_search.as_mut() {
                    search.status = HistorySearchStatus::Searching;
                }
            }
            HistorySearchResult::AtBoundary => {
                if let Some(search) = self.history_search.as_mut() {
                    search.status = HistorySearchStatus::Match;
                }
            }
            result @ (HistorySearchResult::NotFound | HistorySearchResult::Unavailable) => {
                let original_draft = self
                    .history_search
                    .as_ref()
                    .map(|search| search.original_draft.clone());
                if let Some(search) = self.history_search.as_mut() {
                    search.status = if matches!(result, HistorySearchResult::NotFound) {
                        HistorySearchStatus::NoMatch
                    } else {
                        HistorySearchStatus::Idle
                    };
                }
                if let Some(original_draft) = original_draft {
                    self.restore_draft(original_draft);
                }
            }
        }
    }

    /// 构建反向历史搜索激活时显示的页脚行。
    ///
    /// 页脚将查询显示为可编辑字段，并根据状态决定
    /// 显示搜索中、匹配操作还是无匹配反馈。该行刻意与
    /// 光标位置分离，使终端过小无法分配独立提示行时
    /// 渲染可以回退到普通页脚布局。
    pub(super) fn history_search_footer_line(&self) -> Option<Line<'static>> {
        let search = self.history_search.as_ref()?;
        let mut line = Line::from(vec![
            "reverse-i-search: ".dim(),
            search.query.clone().cyan(),
        ]);
        match search.status {
            HistorySearchStatus::Idle => {}
            HistorySearchStatus::Searching => line.push_span("  searching".dim()),
            HistorySearchStatus::Match => {
                line.push_span("  ".dim());
                line.push_span(Self::history_search_action_key_span(KeyCode::Enter));
                line.push_span(" accept".dim());
                line.push_span(" · ".dim());
                line.push_span(Self::history_search_action_key_span(KeyCode::Esc));
                line.push_span(" cancel".dim());
            }
            HistorySearchStatus::NoMatch => line.push_span("  no match".red()),
        }
        Some(line)
    }

    fn history_search_action_key_span(key: KeyCode) -> Span<'static> {
        Span::from(key_hint::plain(key)).cyan().bold().not_dim()
    }

    /// 返回当前 composer 预览中应高亮的字节范围。
    ///
    /// 仅在正在预览匹配的历史条目时暴露高亮。一旦用户用 `Enter`
    /// 确认，搜索会话即被清除，本方法返回空集合，
    /// 使被接受的文本重新成为普通可编辑草稿。
    pub(super) fn history_search_highlight_ranges(&self) -> Vec<Range<usize>> {
        let Some(search) = self.history_search.as_ref() else {
            return Vec::new();
        };
        if !matches!(search.status, HistorySearchStatus::Match) || search.query.is_empty() {
            return Vec::new();
        }
        Self::case_insensitive_match_ranges(self.draft.textarea.text(), &search.query)
    }

    fn case_insensitive_match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
        if query.is_empty() {
            return Vec::new();
        }

        let query_lower = query
            .chars()
            .flat_map(char::to_lowercase)
            .collect::<String>();
        if query_lower.is_empty() {
            return Vec::new();
        }

        let mut folded = String::new();
        let mut folded_spans: Vec<(Range<usize>, Range<usize>)> = Vec::new();
        for (original_start, ch) in text.char_indices() {
            let original_range = original_start..original_start + ch.len_utf8();
            for lower in ch.to_lowercase() {
                let folded_start = folded.len();
                folded.push(lower);
                folded_spans.push((folded_start..folded.len(), original_range.clone()));
            }
        }

        let mut ranges = Vec::new();
        let mut search_from = 0;
        while search_from <= folded.len()
            && let Some(relative_start) = folded[search_from..].find(&query_lower)
        {
            let folded_start = search_from + relative_start;
            let folded_end = folded_start + query_lower.len();
            if let Some((_, first_original)) = folded_spans.iter().find(|(folded_range, _)| {
                folded_range.end > folded_start && folded_range.start < folded_end
            }) {
                let original_end = folded_spans
                    .iter()
                    .rev()
                    .find(|(folded_range, _)| {
                        folded_range.end > folded_start && folded_range.start < folded_end
                    })
                    .map(|(_, original_range)| original_range.end)
                    .unwrap_or(first_original.end);
                ranges.push(first_original.start..original_end);
            }
            search_from = folded_end;
        }
        ranges
    }

    /// 搜索模式激活时，返回页脚查询的屏幕光标位置。
    ///
    /// 光标跟踪页脚查询的末尾而不是 textarea 预览。若页脚区域
    /// 已折叠或太窄，x 坐标会被限制在提示矩形的范围之内，
    /// 使终端后端不会收到屏幕外的光标位置。
    pub(super) fn history_search_cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let search = self.history_search.as_ref()?;
        let [_, _, _, popup_rect] = self.layout_areas(area);
        if popup_rect.is_empty() {
            return None;
        }

        let footer_props = self.footer_props();
        let footer_hint_height = self
            .custom_footer_height()
            .unwrap_or_else(|| footer_height(&footer_props));
        let footer_spacing = Self::footer_spacing(footer_hint_height);
        let hint_rect = if footer_spacing > 0 && footer_hint_height > 0 {
            let [_, hint_rect] = Layout::vertical([
                Constraint::Length(footer_spacing),
                Constraint::Length(footer_hint_height),
            ])
            .areas(popup_rect);
            hint_rect
        } else {
            popup_rect
        };
        if hint_rect.is_empty() {
            return None;
        }

        let prompt_width = Line::from("reverse-i-search: ").width() as u16;
        let query_width = Line::from(search.query.clone()).width() as u16;
        let desired_x = hint_rect
            .x
            .saturating_add(FOOTER_INDENT_COLS as u16)
            .saturating_add(prompt_width)
            .saturating_add(query_width);
        let max_x = hint_rect
            .x
            .saturating_add(hint_rect.width.saturating_sub(1));
        Some((desired_x.min(max_x), hint_rect.y))
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
