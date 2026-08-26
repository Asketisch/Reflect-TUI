//! 多选择器部件，用于从列表中多选项目。
//!
//! 本模块提供一个可模糊搜索、可滚动的选择器，允许用户切换多个项目的开关状态。支持：
//!
//! - **模糊搜索**：输入以按名称筛选项目
//! - **切换选择**：空格切换项目开关
//! - **重排序**：可选的左右箭头支持以重排项目
//! - **实时预览**：可选回调以显示当前选择的预览
//! - **回调**：变更、确认和取消事件钩子
//!
//! # 示例
//!
//! ```ignore
//! let picker = MultiSelectPicker::new(
//!     "选择项目".to_string(),
//!     Some("选择要启用的项目".to_string()),
//!     app_event_tx,
//! )
//! .items(vec![
//!     MultiSelectItem {
//!         id: "a".into(),
//!         name: "项目 A".into(),
//!         description: None,
//!         enabled: true,
//!         orderable: true,
//!         section_break_after: false,
//!     },
//!     MultiSelectItem {
//!         id: "b".into(),
//!         name: "项目 B".into(),
//!         description: None,
//!         enabled: false,
//!         orderable: true,
//!         section_break_after: false,
//!     },
//! ])
//! .on_confirm(|selected_ids, tx| { /* 处理确认 */ })
//! .build();
//! ```

use crate::utils_fuzzy_match::fuzzy_match;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Widget;

use super::selection_popup_common::GenericDisplayRow;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::bottom_pane::CancellationEvent;
use crate::tui_core::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::tui_core::bottom_pane::popup_consts::MAX_POPUP_ROWS;
use crate::tui_core::bottom_pane::scroll_state::ScrollState;
use crate::tui_core::bottom_pane::selection_popup_common::render_rows_single_line;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::key_hint::is_plain_text_key_event;
use crate::tui_core::keymap::ListKeymap;
use crate::tui_core::keymap::RuntimeKeymap;
use crate::tui_core::keymap::primary_binding;
use crate::tui_core::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::tui_core::render::Insets;
use crate::tui_core::render::RectExt;
use crate::tui_core::render::renderable::ColumnRenderable;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::style::user_message_style;
use crate::tui_core::text_formatting::truncate_text;

/// 项目名称在截断前的最大显示长度。
const ITEM_NAME_TRUNCATE_LEN: usize = 21;

/// 搜索输入为空时显示的占位符文本。
const SEARCH_PLACEHOLDER: &str = "Type to search";

/// 搜索查询前显示的提示符（模拟命令提示符）。
const SEARCH_PROMPT_PREFIX: &str = "> ";

const SECTION_BREAK_ROW: &str = "  ───────────────────────";

/// 列表中重新排序项目的方向。
enum Direction {
    Up,
    Down,
}

/// 当任何项目状态变更（切换或重排序）时调用的回调。
/// 接收完整的项目列表和事件发送器。
pub type ChangeCallBack = Box<dyn Fn(&[MultiSelectItem], &AppEventSender) + Send + Sync>;

/// 当用户确认选择（按 Enter）时调用的回调。
/// 接收所有启用项目的 ID 列表。
pub type ConfirmCallback = Box<dyn Fn(&[String], &AppEventSender) + Send + Sync>;

/// 当用户取消选择器（按 Escape）时调用的回调。
pub type CancelCallback = Box<dyn Fn(&AppEventSender) + Send + Sync>;

/// 根据当前项目状态生成可选预览行的回调。
/// 返回 `None` 以隐藏预览区域。
pub type PreviewCallback = Box<dyn Fn(&[MultiSelectItem]) -> Option<Line<'static>> + Send + Sync>;

/// 多选择器中的单个可选项目。
///
/// 每个项目具有唯一标识符、显示名称、可选描述，
/// 以及用户可以切换的启用/禁用状态。
pub(crate) struct MultiSelectItem {
    /// 该项目启用时在确认回调中返回的唯一标识符。
    pub id: String,

    /// 在选择器列表中显示的展示名称。如果过长将显示省略号截断。
    pub name: String,

    /// 与名称并排显示的可选描述（变暗显示）。
    pub description: Option<String>,

    /// 该项目当前是否被选中/启用。
    pub enabled: bool,

    /// 启用排序时该项目是否可以被移动。
    pub orderable: bool,

    /// 当该项目之后还有可见项目时，是否在其后绘制分隔线。
    pub section_break_after: bool,
}

impl Default for MultiSelectItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            description: None,
            enabled: false,
            orderable: true,
            section_break_after: false,
        }
    }
}

struct BuiltRows {
    rows: Vec<GenericDisplayRow>,
    state: ScrollState,
}

/// 带有模糊搜索和可选重排序的多选选择器部件。
///
/// 选择器显示带有复选框的可滚动项目列表。用户可以：
/// - 输入以模糊搜索和筛选列表
/// - 使用上/下（或 Ctrl+P/Ctrl+N）导航
/// - 按空格切换选中项目
/// - 按 Enter 确认并关闭
/// - 按 Escape 取消并关闭
/// - 使用左/右箭头重新排序项目（如果启用了排序）
///
/// 使用构建器模式通过 [`MultiSelectPicker::new`] 创建实例。
pub(crate) struct MultiSelectPicker {
    /// 选择器中的所有项目（未过滤）。
    items: Vec<MultiSelectItem>,

    /// 可见列表的滚动和选择状态。
    state: ScrollState,

    /// 选择器是否已关闭（已确认或已取消）。
    pub(crate) complete: bool,

    /// 用于发送应用事件的通道。
    app_event_tx: AppEventSender,

    /// 显示标题和副标题的头部部件。
    header: Box<dyn Renderable>,

    /// 显示键盘提示的页脚行。
    footer_hint: Line<'static>,

    /// 用户输入的当前搜索/过滤查询。
    search_query: String,

    /// 与当前过滤器匹配的 `items` 索引，按显示顺序排列。
    filtered_indices: Vec<usize>,

    /// 是否启用左/右箭头重新排序。
    ordering_enabled: bool,

    /// 用于导航和完成的共享列表按键绑定。
    keymap: ListKeymap,

    /// 可选回调，根据当前项目状态生成预览行。
    preview_builder: Option<PreviewCallback>,

    /// 缓存的预览行（在项目变更时更新）。
    preview_line: Option<Line<'static>>,

    /// 项目变更（切换或重排）时调用的回调。
    on_change: Option<ChangeCallBack>,

    /// 用户确认选择时调用的回调。
    on_confirm: Option<ConfirmCallback>,

    /// 用户取消选择器时调用的回调。
    on_cancel: Option<CancelCallback>,
}

impl MultiSelectPicker {
    /// 创建一个新的构建器来构造 `MultiSelectPicker`。
    ///
    /// # 参数
    ///
    /// * `title` - 显示在选择器顶部的主标题
    /// * `subtitle` - 显示在标题下方的可选副标题（变暗显示）
    /// * `app_event_tx` - 用于派发应用事件的事件发送器
    pub fn builder(
        title: String,
        subtitle: Option<String>,
        app_event_tx: AppEventSender,
    ) -> MultiSelectPickerBuilder {
        MultiSelectPickerBuilder::new(title, subtitle, app_event_tx)
    }

    /// 应用当前搜索查询以过滤和排序项目。
    ///
    /// 更新 `filtered_indices` 使其仅包含匹配的项目，按模糊匹配分数排序。
    /// 如果当前选中项仍然匹配过滤器，则尝试保留该选中项。
    fn apply_filter(&mut self) {
        // 在尽可能保留当前选中项的同时进行过滤和排序。
        let previously_selected = self
            .state
            .selected_idx
            .and_then(|visible_idx| self.filtered_indices.get(visible_idx).copied());

        let filter = self.search_query.trim();
        if filter.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
        } else {
            let mut matches: Vec<(usize, i32)> = Vec::new();
            for (idx, item) in self.items.iter().enumerate() {
                let display_name = item.name.as_str();
                if let Some((_indices, score)) = match_item(filter, display_name, &item.name) {
                    matches.push((idx, score));
                }
            }

            matches.sort_by(|a, b| {
                a.1.cmp(&b.1).then_with(|| {
                    let an = self.items[a.0].name.as_str();
                    let bn = self.items[b.0].name.as_str();
                    an.cmp(bn)
                })
            });

            self.filtered_indices = matches.into_iter().map(|(idx, _score)| idx).collect();
        }

        let len = self.filtered_indices.len();
        self.state.selected_idx = previously_selected
            .and_then(|actual_idx| {
                self.filtered_indices
                    .iter()
                    .position(|idx| *idx == actual_idx)
            })
            .or_else(|| (len > 0).then_some(0));

        let visible = Self::max_visible_rows(len);
        self.state.clamp_selection(len);
        self.state.ensure_visible(len, visible);
    }

    /// 返回过滤后可见的项目数量。
    fn visible_len(&self) -> usize {
        self.filtered_indices.len()
    }

    /// 返回一次最多可以显示的行数。
    fn max_visible_rows(len: usize) -> usize {
        MAX_POPUP_ROWS.min(len.max(1))
    }

    /// 计算行内容可用的宽度（考虑边框）。
    fn rows_width(total_width: u16) -> u16 {
        total_width.saturating_sub(2)
    }

    /// 计算行列表区域所需的高度。
    fn rows_height(&self, rows: &BuiltRows) -> u16 {
        rows.rows
            .len()
            .clamp(1, MAX_POPUP_ROWS)
            .try_into()
            .unwrap_or(1)
    }

    /// 为所有当前可见（已过滤）的项目构建显示行。
    ///
    /// 每行显示：`› [x] 项目名称`，其中 `›` 表示光标位置，
    /// `[x]` 或 `[ ]` 表示启用/禁用状态。
    fn build_rows(&self) -> BuiltRows {
        let mut rows = Vec::new();
        let mut visible_to_row = Vec::with_capacity(self.filtered_indices.len());
        for (visible_idx, actual_idx) in self.filtered_indices.iter().enumerate() {
            let Some(item) = self.items.get(*actual_idx) else {
                continue;
            };
            visible_to_row.push(rows.len());
            let is_selected = self.state.selected_idx == Some(visible_idx);
            let prefix = if is_selected { '›' } else { ' ' };
            let marker = if item.enabled { 'x' } else { ' ' };
            let item_name = truncate_text(&item.name, ITEM_NAME_TRUNCATE_LEN);
            let name = format!("{prefix} [{marker}] {item_name}");
            rows.push(GenericDisplayRow {
                name,
                description: item.description.clone(),
                ..Default::default()
            });

            if item.section_break_after && visible_idx + 1 < self.filtered_indices.len() {
                rows.push(GenericDisplayRow {
                    name: SECTION_BREAK_ROW.to_string(),
                    is_disabled: true,
                    ..Default::default()
                });
            }
        }

        let selected_idx = self
            .state
            .selected_idx
            .and_then(|visible_idx| visible_to_row.get(visible_idx).copied());
        let scroll_top = visible_to_row
            .get(self.state.scroll_top)
            .copied()
            .unwrap_or(0);
        BuiltRows {
            rows,
            state: ScrollState {
                selected_idx,
                scroll_top,
            },
        }
    }

    /// 向上移动选择光标，若已在顶部则回绕到底部。
    fn move_up(&mut self) {
        let len = self.visible_len();
        self.state.move_up_wrap(len);
        let visible = Self::max_visible_rows(len);
        self.state.ensure_visible(len, visible);
    }

    /// 向下移动选择光标，若已在底部则回绕到顶部。
    fn move_down(&mut self) {
        let len = self.visible_len();
        self.state.move_down_wrap(len);
        let visible = Self::max_visible_rows(len);
        self.state.ensure_visible(len, visible);
    }

    fn page_up(&mut self) {
        let len = self.visible_len();
        let visible = Self::max_visible_rows(len);
        self.state.page_up_clamped(len, visible);
    }

    fn page_down(&mut self) {
        let len = self.visible_len();
        let visible = Self::max_visible_rows(len);
        self.state.page_down_clamped(len, visible);
    }

    fn jump_top(&mut self) {
        let len = self.visible_len();
        let visible = Self::max_visible_rows(len);
        self.state.jump_top(len, visible);
    }

    fn jump_bottom(&mut self) {
        let len = self.visible_len();
        let visible = Self::max_visible_rows(len);
        self.state.jump_bottom(len, visible);
    }

    /// 切换当前选中项目的启用状态。
    ///
    /// 更新预览行，并在设置了 `on_change` 回调时调用它。
    fn toggle_selected(&mut self) {
        let Some(idx) = self.state.selected_idx else {
            return;
        };
        let Some(actual_idx) = self.filtered_indices.get(idx).copied() else {
            return;
        };
        let Some(item) = self.items.get_mut(actual_idx) else {
            return;
        };

        item.enabled = !item.enabled;
        self.update_preview_line();
        if let Some(on_change) = &self.on_change {
            on_change(&self.items, &self.app_event_tx);
        }
    }

    /// 确认当前选择并关闭选择器。
    ///
    /// 收集所有启用项目的 ID 并将其传递给
    /// `on_confirm` 回调。如果已完成则不做任何事。
    fn confirm_selection(&mut self) {
        if self.complete {
            return;
        }
        self.complete = true;

        if let Some(on_confirm) = &self.on_confirm {
            let selected_ids: Vec<String> = self
                .items
                .iter()
                .filter(|item| item.enabled)
                .map(|item| item.id.clone())
                .collect();
            on_confirm(&selected_ids, &self.app_event_tx);
        }
    }

    /// 在列表中上下移动当前选中的项目。
    ///
    /// 仅在以下情况下有效：
    /// - 搜索查询为空（过滤期间禁用重排）
    /// - 通过 [`MultiSelectPickerBuilder::enable_ordering`] 启用了排序
    ///
    /// 更新预览行并调用 `on_change` 回调。
    fn move_selected_item(&mut self, direction: Direction) {
        if !self.search_query.is_empty() {
            return;
        }

        let Some(visible_idx) = self.state.selected_idx else {
            return;
        };
        let Some(actual_idx) = self.filtered_indices.get(visible_idx).copied() else {
            return;
        };

        let len = self.items.len();
        if len == 0 {
            return;
        }

        if !self
            .items
            .get(actual_idx)
            .is_some_and(|item| item.orderable)
        {
            return;
        }

        let new_idx = match direction {
            Direction::Up if actual_idx > 0 => actual_idx - 1,
            Direction::Down if actual_idx + 1 < len => actual_idx + 1,
            _ => return,
        };

        if !self.items.get(new_idx).is_some_and(|item| item.orderable) {
            return;
        }

        // 在底层列表中移动项目
        self.items.swap(actual_idx, new_idx);

        self.update_preview_line();
        if let Some(on_change) = &self.on_change {
            on_change(&self.items, &self.app_event_tx);
        }

        // 重建过滤索引以保持搜索/过滤的一致性
        self.apply_filter();

        // 将选择恢复到被移动的项目上
        let moved_idx = new_idx;
        if let Some(new_visible_idx) = self
            .filtered_indices
            .iter()
            .position(|idx| *idx == moved_idx)
        {
            self.state.selected_idx = Some(new_visible_idx);
        }
    }

    /// 使用预览回调重新生成预览行。
    ///
    /// 在任意项目状态变更（切换或重排）后调用。
    fn update_preview_line(&mut self) {
        self.preview_line = self
            .preview_builder
            .as_ref()
            .and_then(|builder| builder(&self.items));
    }

    /// 不确认选择即关闭选择器，并调用 `on_cancel` 回调。
    ///
    /// 如果已完成则不做任何事。
    pub fn close(&mut self) {
        if self.complete {
            return;
        }
        self.complete = true;
        if let Some(on_cancel) = &self.on_cancel {
            on_cancel(&self.app_event_tx);
        }
    }
}

impl BottomPaneView for MultiSelectPicker {
    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.close();
        CancellationEvent::Handled
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        // 可打印字符始终用于搜索。诸如纯 j/k 之类的移动别名
        // 仅通过非文本事件或带修饰键的绑定生效。
        let allow_plain_char_navigation = !is_plain_text_key_event(key_event);

        match key_event {
            _ if allow_plain_char_navigation
                && self.ordering_enabled
                && self.keymap.move_left.is_pressed(key_event) =>
            {
                self.move_selected_item(Direction::Up);
            }
            _ if allow_plain_char_navigation
                && self.ordering_enabled
                && self.keymap.move_right.is_pressed(key_event) =>
            {
                self.move_selected_item(Direction::Down);
            }
            _ if allow_plain_char_navigation && self.keymap.move_up.is_pressed(key_event) => {
                self.move_up()
            }
            _ if allow_plain_char_navigation && self.keymap.move_down.is_pressed(key_event) => {
                self.move_down()
            }
            _ if allow_plain_char_navigation && self.keymap.page_up.is_pressed(key_event) => {
                self.page_up()
            }
            _ if allow_plain_char_navigation && self.keymap.page_down.is_pressed(key_event) => {
                self.page_down()
            }
            _ if allow_plain_char_navigation && self.keymap.jump_top.is_pressed(key_event) => {
                self.jump_top()
            }
            _ if allow_plain_char_navigation && self.keymap.jump_bottom.is_pressed(key_event) => {
                self.jump_bottom()
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } => {
                self.search_query.pop();
                self.apply_filter();
            }
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            } => self.toggle_selected(),
            _ if self.keymap.accept.is_pressed(key_event) => self.confirm_selection(),
            _ if self.keymap.cancel.is_pressed(key_event) => self.close(),
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.search_query.push(c);
                self.apply_filter();
            }
            _ => {}
        }
    }
}

impl Renderable for MultiSelectPicker {
    fn desired_height(&self, width: u16) -> u16 {
        let rows = self.build_rows();
        let rows_height = self.rows_height(&rows);
        let preview_height = if self.preview_line.is_some() { 1 } else { 0 };

        let mut height = self.header.desired_height(width.saturating_sub(4));
        height = height.saturating_add(rows_height + 3);
        height = height.saturating_add(2);
        height.saturating_add(1 + preview_height)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        // 为按键提示行预留页脚区域。
        let preview_height = if self.preview_line.is_some() { 1 } else { 0 };
        let footer_height = 1 + preview_height;
        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(footer_height)]).areas(area);

        Block::default()
            .style(user_message_style())
            .render(content_area, buf);

        let header_height = self
            .header
            .desired_height(content_area.width.saturating_sub(4));
        let rows = self.build_rows();
        let rows_width = Self::rows_width(content_area.width);
        let rows_height = self.rows_height(&rows);
        let [header_area, _, search_area, list_area] = Layout::vertical([
            Constraint::Max(header_height),
            Constraint::Max(1),
            Constraint::Length(2),
            Constraint::Length(rows_height),
        ])
        .areas(content_area.inset(Insets::vh(/*v*/ 1, /*h*/ 2)));

        self.header.render(header_area, buf);

        // 将搜索提示渲染为两行以模拟输入器。
        if search_area.height >= 2 {
            let [placeholder_area, input_area] =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(search_area);
            Line::from(SEARCH_PLACEHOLDER.dim()).render(placeholder_area, buf);
            let line = if self.search_query.is_empty() {
                Line::from(vec![SEARCH_PROMPT_PREFIX.dim()])
            } else {
                Line::from(vec![
                    SEARCH_PROMPT_PREFIX.dim(),
                    self.search_query.clone().into(),
                ])
            };
            line.render(input_area, buf);
        } else if search_area.height > 0 {
            let query_span = if self.search_query.is_empty() {
                SEARCH_PLACEHOLDER.dim()
            } else {
                self.search_query.clone().into()
            };
            Line::from(query_span).render(search_area, buf);
        }

        if list_area.height > 0 {
            let render_area = Rect {
                x: list_area.x.saturating_sub(2),
                y: list_area.y,
                width: rows_width.max(1),
                height: list_area.height,
            };
            render_rows_single_line(
                render_area,
                buf,
                &rows.rows,
                &rows.state,
                render_area.height as usize,
                "no matches",
            );
        }

        let hint_area = if let Some(preview_line) = &self.preview_line {
            let [preview_area, hint_area] =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(footer_area);
            let preview_area = Rect {
                x: preview_area.x + 2,
                y: preview_area.y,
                width: preview_area.width.saturating_sub(2),
                height: preview_area.height,
            };
            let max_preview_width = preview_area.width.saturating_sub(2) as usize;
            let preview_line =
                truncate_line_with_ellipsis_if_overflow(preview_line.clone(), max_preview_width);
            preview_line.render(preview_area, buf);
            hint_area
        } else {
            footer_area
        };
        let hint_area = Rect {
            x: hint_area.x + 2,
            y: hint_area.y,
            width: hint_area.width.saturating_sub(2),
            height: hint_area.height,
        };
        self.footer_hint.clone().dim().render(hint_area, buf);
    }
}

/// 用于构建 [`MultiSelectPicker`] 的构建器，采用流畅 API。
///
/// # 示例
///
/// ```ignore
/// let picker = MultiSelectPicker::new("标题".into(), None, tx)
///     .items(items)
///     .enable_ordering()
///     .on_preview(|items| Some(Line::from("预览")))
///     .on_confirm(|ids, tx| { /* 处理 */ })
///     .on_cancel(|tx| { /* 处理 */ })
///     .build();
/// ```
pub(crate) struct MultiSelectPickerBuilder {
    title: String,
    subtitle: Option<String>,
    instructions: Vec<Span<'static>>,
    items: Vec<MultiSelectItem>,
    ordering_enabled: bool,
    app_event_tx: AppEventSender,
    keymap: ListKeymap,
    preview_builder: Option<PreviewCallback>,
    on_change: Option<ChangeCallBack>,
    on_confirm: Option<ConfirmCallback>,
    on_cancel: Option<CancelCallback>,
}

// ── 构建器与匹配辅助（外移子模块） ──
mod builder;
pub(crate) use builder::*;

#[cfg(test)]
#[cfg(test)]
mod tests;
