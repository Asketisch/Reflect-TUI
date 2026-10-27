//! 列表选择视图。
//!
//! 注：本文件超过 800 行红线，但 `impl ListSelectionView` 为内聚的完整状态机，
//! 与上游逐行对应，按 AGENTS.md「内聚完整状态机例外」保留。

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use itertools::Itertools as _;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use super::selection_popup_common::render_menu_surface;
use super::selection_popup_common::wrap_styled_line;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::clipboard_paste::normalize_pasted_search_query;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::key_hint::is_plain_text_key_event;
use crate::tui_core::keymap::ListKeymap;
use crate::tui_core::render::renderable::ColumnRenderable;
use crate::tui_core::render::renderable::Renderable;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;
use super::bottom_pane_view::ViewCompletion;
use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::ColumnWidthConfig;
pub(crate) use super::selection_popup_common::ColumnWidthMode;
use super::selection_popup_common::GenericDisplayRow;
use super::selection_popup_common::measure_rows_height_with_col_width_mode;
use super::selection_popup_common::render_rows_single_line_with_col_width_mode;
use super::selection_popup_common::render_rows_with_col_width_mode;
use super::selection_tabs::SelectionTab;
use super::selection_tabs::render_tab_bar;
use super::selection_tabs::tab_bar_height;
use unicode_width::UnicodeWidthStr;

/// 启用并排布局所需的最小列表宽度（以内容列计）。
/// 即使与侧边内容面板共享水平空间，也能保持列表可用。

const MIN_LIST_WIDTH_FOR_SIDE: u16 = 40;

/// 并排布局启用时，列表区域与侧边内容面板之间的水平间隔（以列计）。

const SIDE_CONTENT_GAP: u16 = 2;

/// 选择弹窗使用的共享菜单表面水平内边距（每侧 2 个单元格）。
const MENU_SURFACE_HORIZONTAL_INSET: u16 = 4;

/// 控制侧边内容面板相对于弹窗宽度的尺寸。
///
/// 中文说明：When the computed side width falls below `side_content_min_width` or the
/// 中文说明：remaining list area would be narrower than [`MIN_LIST_WIDTH_FOR_SIDE`], the
/// 中文说明：side-by-side layout is abandoned and the stacked fallback is used instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SideContentWidth {
    /// 固定列数。`Fixed(0)` 会完全禁用侧边内容。
    Fixed(u16),
    /// 将内容区域精确按 50/50 分割（减去列间距）。
    Half,
}

// ── 布局宽度计算辅助（外移子模块） ──
mod layout;
pub(crate) use layout::*;

impl Default for SelectionViewParams {
    fn default() -> Self {
        Self {
            view_id: None,
            title: None,
            subtitle: None,
            footer_note: None,
            footer_hint: None,
            tab_footer_hints: Vec::new(),
            items: Vec::new(),
            tabs: Vec::new(),
            initial_tab_id: None,
            is_searchable: false,
            search_placeholder: None,
            col_width_mode: ColumnWidthMode::AutoVisible,
            row_display: SelectionRowDisplay::Wrapped,
            name_column_width: None,
            header: Box::new(()),
            initial_selected_idx: None,
            side_content: Box::new(()),
            side_content_width: SideContentWidth::default(),
            side_content_min_width: 0,
            stacked_side_content: None,
            preserve_side_content_bg: false,
            on_selection_changed: None,
            allow_cancel: true,
            on_cancel: None,
        }
    }
}

/// 用于渲染列表式选择弹窗并与之交互的运行时状态。
///
/// 中文说明：This type is the single authority for filtered index mapping between
/// 中文说明：visible rows and source items and for preserving selection while filters
/// 中文说明：change.
pub(crate) struct ListSelectionView {
    view_id: Option<&'static str>,
    footer_note: Option<Line<'static>>,
    footer_hint: Option<Line<'static>>,
    tab_footer_hints: Vec<(String, Line<'static>)>,
    items: Vec<SelectionItem>,
    tabs: Vec<SelectionTab>,
    active_tab_idx: Option<usize>,
    state: ScrollState,
    completion: Option<ViewCompletion>,
    dismiss_after_child_accept: bool,
    app_event_tx: AppEventSender,
    is_searchable: bool,
    search_query: String,
    search_placeholder: Option<String>,
    col_width_mode: ColumnWidthMode,
    row_display: SelectionRowDisplay,
    name_column_width: Option<usize>,
    filtered_indices: Vec<usize>,
    last_selected_actual_idx: Option<usize>,
    header: Box<dyn Renderable>,
    initial_selected_idx: Option<usize>,
    side_content: Box<dyn Renderable>,
    side_content_width: SideContentWidth,
    side_content_min_width: u16,
    stacked_side_content: Option<Box<dyn Renderable>>,
    preserve_side_content_bg: bool,

    /// 高亮条目变化（导航、过滤、数字键）时调用。
    on_selection_changed: OnSelectionChangedCallback,

    allow_cancel: bool,

    /// 选择器未做选择便通过 Esc/Ctrl+C 关闭时调用。
    on_cancel: OnCancelCallback,
    keymap: ListKeymap,
}

const SELECTION_TOGGLE_ON_PREFIX: &str = "[*] ";
const SELECTION_TOGGLE_OFF_PREFIX: &str = "[ ] ";
pub(crate) const SELECTION_TOGGLE_UNAVAILABLE_PREFIX: &str = "[-] ";
pub(crate) const SELECTION_TOGGLE_BLOCKED_PREFIX: &str = "[!] ";

fn selection_toggle_prefix(toggle: &SelectionToggle) -> &'static str {
    if toggle.is_on {
        SELECTION_TOGGLE_ON_PREFIX
    } else {
        SELECTION_TOGGLE_OFF_PREFIX
    }
}

fn selection_item_toggle_prefix(item: &SelectionItem) -> Option<&'static str> {
    item.toggle
        .as_ref()
        .map(selection_toggle_prefix)
        .or(item.toggle_placeholder)
}

impl ListSelectionView {
    fn selected_item_has_toggle(&self) -> bool {
        self.selected_actual_idx()
            .and_then(|actual_idx| self.active_items().get(actual_idx))
            .is_some_and(|item| item.toggle.is_some() && Self::item_is_enabled(item))
    }

    fn selected_item_has_toggle_placeholder(&self) -> bool {
        self.selected_actual_idx()
            .and_then(|actual_idx| self.active_items().get(actual_idx))
            .is_some_and(|item| {
                item.toggle.is_none()
                    && item.toggle_placeholder.is_some()
                    && Self::item_is_enabled(item)
            })
    }

    fn toggle_selected(&mut self) {
        let Some(actual_idx) = self.selected_actual_idx() else {
            return;
        };
        let app_event_tx = self.app_event_tx.clone();
        let Some(item) = self.active_items_mut().get_mut(actual_idx) else {
            return;
        };
        if !Self::item_is_enabled(item) {
            return;
        }
        let Some(toggle) = item.toggle.as_mut() else {
            return;
        };

        toggle.is_on = !toggle.is_on;
        (toggle.action)(toggle.is_on, &app_event_tx);
    }
}

impl ListSelectionView {
    /// 创建已连接过滤、滚动和回调的选择弹窗视图。
    ///
    /// 中文说明：The constructor normalizes header/title composition and immediately
    /// 中文说明：applies filtering so `ScrollState` starts in a valid visible range.
    /// 中文说明：When search is enabled, rows without `search_value` will disappear as
    /// 中文说明：soon as the query is non-empty, which can look like dropped data unless
    /// 中文说明：callers intentionally populate that field.
    pub fn new(
        params: SelectionViewParams,
        app_event_tx: AppEventSender,
        keymap: ListKeymap,
    ) -> Self {
        let mut header = params.header;
        if params.title.is_some() || params.subtitle.is_some() {
            let title = params.title.map(|title| Line::from(title.bold()));
            let subtitle = params.subtitle.map(|subtitle| Line::from(subtitle.dim()));
            header = Box::new(ColumnRenderable::with([
                header,
                Box::new(title),
                Box::new(subtitle),
            ]));
        }
        let active_tab_idx = params.initial_tab_id.as_ref().and_then(|initial_tab_id| {
            params
                .tabs
                .iter()
                .position(|tab| tab.id.as_str() == initial_tab_id.as_str())
        });
        let active_tab_idx = if params.tabs.is_empty() {
            None
        } else {
            Some(active_tab_idx.unwrap_or(0))
        };
        let has_initial_selected_idx = params.initial_selected_idx.is_some();
        let mut s = Self {
            view_id: params.view_id,
            footer_note: params.footer_note,
            footer_hint: params.footer_hint,
            tab_footer_hints: params.tab_footer_hints,
            items: params.items,
            tabs: params.tabs,
            active_tab_idx,
            state: ScrollState::new(),
            completion: None,
            dismiss_after_child_accept: false,
            app_event_tx,
            is_searchable: params.is_searchable,
            search_query: String::new(),
            search_placeholder: if params.is_searchable {
                params.search_placeholder
            } else {
                None
            },
            col_width_mode: params.col_width_mode,
            row_display: params.row_display,
            name_column_width: params.name_column_width,
            filtered_indices: Vec::new(),
            last_selected_actual_idx: None,
            header,
            initial_selected_idx: params.initial_selected_idx,
            side_content: params.side_content,
            side_content_width: params.side_content_width,
            side_content_min_width: params.side_content_min_width,
            stacked_side_content: params.stacked_side_content,
            preserve_side_content_bg: params.preserve_side_content_bg,
            on_selection_changed: params.on_selection_changed,
            allow_cancel: params.allow_cancel,
            on_cancel: params.on_cancel,
            keymap,
        };
        s.apply_filter();
        if s.tabs_enabled() && !has_initial_selected_idx && s.state.selected_idx.is_none() {
            s.select_first_enabled_row();
        }
        s
    }

    fn visible_len(&self) -> usize {
        self.filtered_indices.len()
    }

    fn tabs_enabled(&self) -> bool {
        self.active_tab_idx.is_some()
    }

    fn active_items(&self) -> &[SelectionItem] {
        self.active_tab_idx
            .and_then(|idx| self.tabs.get(idx))
            .map(|tab| tab.items.as_slice())
            .unwrap_or(self.items.as_slice())
    }

    fn active_items_mut(&mut self) -> &mut [SelectionItem] {
        if let Some(idx) = self.active_tab_idx
            && let Some(tab) = self.tabs.get_mut(idx)
        {
            return tab.items.as_mut_slice();
        }
        self.items.as_mut_slice()
    }

    fn active_header(&self) -> &dyn Renderable {
        self.active_tab_idx
            .and_then(|idx| self.tabs.get(idx))
            .map(|tab| tab.header.as_ref())
            .unwrap_or(self.header.as_ref())
    }

    fn active_footer_hint(&self) -> Option<&Line<'static>> {
        self.active_tab_id()
            .and_then(|active_tab_id| {
                self.tab_footer_hints
                    .iter()
                    .find_map(|(tab_id, hint)| (tab_id.as_str() == active_tab_id).then_some(hint))
            })
            .or(self.footer_hint.as_ref())
    }

    fn active_tab_id(&self) -> Option<&str> {
        self.active_tab_idx
            .and_then(|idx| self.tabs.get(idx))
            .map(|tab| tab.id.as_str())
    }

    fn max_visible_rows(len: usize) -> usize {
        MAX_POPUP_ROWS.min(len.max(1))
    }

    fn selected_actual_idx(&self) -> Option<usize> {
        self.state
            .selected_idx
            .and_then(|visible_idx| self.filtered_indices.get(visible_idx).copied())
    }

    fn apply_filter(&mut self) {
        let previously_selected = self
            .selected_actual_idx()
            .filter(|actual_idx| self.enabled_actual_idx(*actual_idx).is_some())
            .or_else(|| {
                (!self.is_searchable)
                    .then(|| {
                        self.active_items()
                            .iter()
                            .position(|item| item.is_current && Self::item_is_enabled(item))
                    })
                    .flatten()
            })
            .or_else(|| {
                self.initial_selected_idx
                    .take()
                    .filter(|actual_idx| self.enabled_actual_idx(*actual_idx).is_some())
            });

        if self.is_searchable && !self.search_query.is_empty() {
            let query_lower = self.search_query.to_lowercase();
            self.filtered_indices = self
                .active_items()
                .iter()
                .positions(|item| {
                    item.search_value
                        .as_ref()
                        .is_some_and(|v| v.to_lowercase().contains(&query_lower))
                })
                .collect();
        } else {
            self.filtered_indices = (0..self.active_items().len()).collect();
        }

        let len = self.filtered_indices.len();
        let selected_visible_idx = self
            .state
            .selected_idx
            .and_then(|visible_idx| {
                self.filtered_indices
                    .get(visible_idx)
                    .and_then(|idx| self.filtered_indices.iter().position(|cur| cur == idx))
            })
            .or_else(|| {
                previously_selected.and_then(|actual_idx| {
                    self.filtered_indices
                        .iter()
                        .position(|idx| *idx == actual_idx)
                })
            });
        self.state.selected_idx = selected_visible_idx
            .filter(|visible_idx| {
                self.filtered_indices
                    .get(*visible_idx)
                    .and_then(|actual_idx| self.active_items().get(*actual_idx))
                    .is_some_and(Self::item_is_enabled)
            })
            .or_else(|| self.first_enabled_visible_idx())
            .or_else(|| (len > 0).then_some(0));

        let visible = Self::max_visible_rows(len);
        self.state.clamp_selection(len);
        self.state.ensure_visible(len, visible);

        // 当过滤改变实际选中条目时通知回调，
        // 以保持实时预览同步（例如在主题选择器中输入）。
        let new_actual = self.selected_actual_idx();
        if new_actual != previously_selected {
            self.fire_selection_changed();
        }
    }

    fn build_rows(&self) -> Vec<GenericDisplayRow> {
        let enabled_row_number_width = self
            .filtered_indices
            .iter()
            .filter(|actual_idx| {
                self.active_items()
                    .get(**actual_idx)
                    .is_some_and(Self::item_is_enabled)
            })
            .count()
            .max(1)
            .to_string()
            .len();
        let mut enabled_row_number = 0;
        self.filtered_indices
            .iter()
            .enumerate()
            .filter_map(|(visible_idx, actual_idx)| {
                self.active_items().get(*actual_idx).map(|item| {
                    let is_selected = self.state.selected_idx == Some(visible_idx);
                    let prefix = if is_selected { '›' } else { ' ' };
                    let name = item.name.as_str();
                    let marker = if item.is_current {
                        " (current)"
                    } else if item.is_default {
                        " (default)"
                    } else {
                        ""
                    };
                    let name_with_marker = format!("{name}{marker}");
                    let is_disabled = item.is_disabled || item.disabled_reason.is_some();
                    let wrap_prefix = if self.is_searchable {
                        // 启用搜索时数字键不起作用（因为我们允许
                        // 将数字用于搜索查询）。
                        format!("{prefix} ")
                    } else if is_disabled {
                        format!("{prefix} {}", " ".repeat(enabled_row_number_width + 2))
                    } else {
                        enabled_row_number += 1;
                        let n = enabled_row_number;
                        format!("{prefix} {n}. ")
                    };
                    let wrap_prefix_width = UnicodeWidthStr::width(wrap_prefix.as_str());
                    let mut name_prefix_spans = Vec::new();
                    name_prefix_spans.push(wrap_prefix.into());
                    if let Some(toggle_prefix) = selection_item_toggle_prefix(item) {
                        name_prefix_spans.push(toggle_prefix.into());
                    }
                    name_prefix_spans.extend(item.name_prefix_spans.clone());
                    let description = is_selected
                        .then(|| item.selected_description.clone())
                        .flatten()
                        .or_else(|| item.description.clone());
                    let wrap_indent = description.is_none().then_some(wrap_prefix_width);
                    GenericDisplayRow {
                        name: name_with_marker,
                        name_prefix_spans,
                        display_shortcut: item.display_shortcut,
                        match_indices: None,
                        description,
                        category_tag: None,
                        wrap_indent,
                        is_disabled,
                        disabled_reason: item.disabled_reason.clone(),
                    }
                })
            })
            .collect()
    }

    fn switch_tab(&mut self, step: isize) {
        let Some(active_idx) = self.active_tab_idx else {
            return;
        };
        let len = self.tabs.len();
        if len == 0 {
            return;
        }

        let next_idx = if step.is_negative() {
            active_idx.checked_sub(1).unwrap_or(len - 1)
        } else {
            (active_idx + 1) % len
        };
        self.active_tab_idx = Some(next_idx);
        self.search_query.clear();
        self.state.reset();
        self.apply_filter();
        if self.state.selected_idx.is_none() {
            self.select_first_enabled_row();
        }
        self.fire_selection_changed();
    }

    fn select_first_enabled_row(&mut self) {
        let selected_visible_idx = self
            .first_enabled_visible_idx()
            .or_else(|| (!self.filtered_indices.is_empty()).then_some(0));
        self.state.selected_idx = selected_visible_idx;
        self.state.scroll_top = 0;
    }

    fn first_enabled_visible_idx(&self) -> Option<usize> {
        self.filtered_indices.iter().position(|actual_idx| {
            self.active_items()
                .get(*actual_idx)
                .is_some_and(Self::item_is_enabled)
        })
    }

    fn enabled_actual_idx(&self, actual_idx: usize) -> Option<usize> {
        self.active_items()
            .get(actual_idx)
            .is_some_and(Self::item_is_enabled)
            .then_some(actual_idx)
    }

    fn item_is_enabled(item: &SelectionItem) -> bool {
        item.disabled_reason.is_none() && !item.is_disabled
    }

    fn actual_idx_for_enabled_number(&self, number: usize) -> Option<usize> {
        if number == 0 {
            return None;
        }

        self.active_items()
            .iter()
            .enumerate()
            .filter(|(_, item)| Self::item_is_enabled(item))
            .nth(number - 1)
            .map(|(idx, _)| idx)
    }

    /// 把「实际索引」(进 `active_items`)换算成「可见索引」(进 `filtered_indices`)
    /// 并写入 `state.selected_idx`。返回 `true` 表示命中且条目可用(已启用),
    /// 调用方可据此决定是否 `accept()`。
    ///
    /// 这统一了快捷键 / 数字键命中路径与导航路径对 `selected_idx` 语义的约定
    /// (此前这两条路径直接把实际索引当作可见索引,仅在 `!is_searchable` 的
    /// 恒等 filtered_indices 下侥幸正确)。
    fn select_actual_idx(&mut self, actual_idx: usize) -> bool {
        let Some(item) = self.active_items().get(actual_idx) else {
            return false;
        };
        if !Self::item_is_enabled(item) {
            return false;
        }
        let Some(visible_idx) = self
            .filtered_indices
            .iter()
            .position(|cur| *cur == actual_idx)
        else {
            return false;
        };
        self.state.selected_idx = Some(visible_idx);
        true
    }

    fn move_up(&mut self) {
        let before = self.selected_actual_idx();
        let len = self.visible_len();
        self.state.move_up_wrap(len);
        let visible = Self::max_visible_rows(len);
        self.skip_disabled_up();
        self.state.ensure_visible(len, visible);
        if self.selected_actual_idx() != before {
            self.fire_selection_changed();
        }
    }

    fn move_down(&mut self) {
        let before = self.selected_actual_idx();
        let len = self.visible_len();
        self.state.move_down_wrap(len);
        let visible = Self::max_visible_rows(len);
        self.skip_disabled_down();
        self.state.ensure_visible(len, visible);
        if self.selected_actual_idx() != before {
            self.fire_selection_changed();
        }
    }

    fn page_up(&mut self) {
        let before = self.selected_actual_idx();
        let len = self.visible_len();
        let visible = Self::max_visible_rows(len);
        self.state.page_up_clamped(len, visible);
        self.skip_disabled_up_clamped();
        self.state.ensure_visible(len, visible);
        if self.selected_actual_idx() != before {
            self.fire_selection_changed();
        }
    }

    fn page_down(&mut self) {
        let before = self.selected_actual_idx();
        let len = self.visible_len();
        let visible = Self::max_visible_rows(len);
        self.state.page_down_clamped(len, visible);
        self.skip_disabled_down_clamped();
        self.state.ensure_visible(len, visible);
        if self.selected_actual_idx() != before {
            self.fire_selection_changed();
        }
    }

    fn jump_top(&mut self) {
        let before = self.selected_actual_idx();
        let len = self.visible_len();
        let visible = Self::max_visible_rows(len);
        self.state.jump_top(len, visible);
        self.skip_disabled_down_clamped();
        self.state.ensure_visible(len, visible);
        if self.selected_actual_idx() != before {
            self.fire_selection_changed();
        }
    }

    fn jump_bottom(&mut self) {
        let before = self.selected_actual_idx();
        let len = self.visible_len();
        let visible = Self::max_visible_rows(len);
        self.state.jump_bottom(len, visible);
        self.skip_disabled_up_clamped();
        self.state.ensure_visible(len, visible);
        if self.selected_actual_idx() != before {
            self.fire_selection_changed();
        }
    }

    fn fire_selection_changed(&self) {
        if let Some(cb) = &self.on_selection_changed
            && let Some(actual) = self.selected_actual_idx()
        {
            cb(actual, &self.app_event_tx);
        }
    }

    fn accept(&mut self) {
        let selected_actual_idx = self
            .state
            .selected_idx
            .and_then(|idx| self.filtered_indices.get(idx).copied());
        let selected_is_enabled = selected_actual_idx
            .and_then(|actual_idx| self.active_items().get(actual_idx))
            .is_some_and(|item| item.disabled_reason.is_none() && !item.is_disabled);
        if selected_is_enabled {
            self.last_selected_actual_idx = selected_actual_idx;
            let Some(actual_idx) = selected_actual_idx else {
                return;
            };
            let Some(item) = self.active_items().get(actual_idx) else {
                return;
            };
            for act in &item.actions {
                act(&self.app_event_tx);
            }
            if item.dismiss_on_select {
                self.completion = Some(ViewCompletion::Accepted);
            } else if item.dismiss_parent_on_child_accept {
                self.dismiss_after_child_accept = true;
            }
        } else if selected_actual_idx.is_none() {
            if let Some(cb) = &self.on_cancel {
                cb(&self.app_event_tx);
            }
            self.completion = Some(ViewCompletion::Cancelled);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_search_query(&mut self, query: String) {
        self.search_query = query;
        self.apply_filter();
    }

    pub(crate) fn take_last_selected_index(&mut self) -> Option<usize> {
        self.last_selected_actual_idx.take()
    }

    fn rows_width(total_width: u16) -> u16 {
        total_width.saturating_sub(2)
    }

    fn clear_to_terminal_bg(buf: &mut Buffer, area: Rect) {
        let buf_area = buf.area();
        let min_x = area.x.max(buf_area.x);
        let min_y = area.y.max(buf_area.y);
        let max_x = area
            .x
            .saturating_add(area.width)
            .min(buf_area.x.saturating_add(buf_area.width));
        let max_y = area
            .y
            .saturating_add(area.height)
            .min(buf_area.y.saturating_add(buf_area.height));
        for y in min_y..max_y {
            for x in min_x..max_x {
                buf[(x, y)]
                    .set_symbol(" ")
                    .set_style(ratatui::style::Style::reset());
            }
        }
    }

    fn force_bg_to_terminal_bg(buf: &mut Buffer, area: Rect) {
        let buf_area = buf.area();
        let min_x = area.x.max(buf_area.x);
        let min_y = area.y.max(buf_area.y);
        let max_x = area
            .x
            .saturating_add(area.width)
            .min(buf_area.x.saturating_add(buf_area.width));
        let max_y = area
            .y
            .saturating_add(area.height)
            .min(buf_area.y.saturating_add(buf_area.height));
        for y in min_y..max_y {
            for x in min_x..max_x {
                buf[(x, y)].set_bg(ratatui::style::Color::Reset);
            }
        }
    }

    fn stacked_side_content(&self) -> &dyn Renderable {
        self.stacked_side_content
            .as_deref()
            .unwrap_or_else(|| self.side_content.as_ref())
    }

    /// 中文说明：Returns `Some(side_width)` when the content area is wide enough for a
    /// 中文说明：side-by-side layout (list + gap + side panel), `None` otherwise.
    fn side_layout_width(&self, content_width: u16) -> Option<u16> {
        side_by_side_layout_widths(
            content_width,
            self.side_content_width,
            self.side_content_min_width,
        )
        .map(|(_, side_width)| side_width)
    }

    fn skip_disabled_down(&mut self) {
        let len = self.visible_len();
        for _ in 0..len {
            if self.selected_visible_idx_is_disabled() {
                self.state.move_down_wrap(len);
            } else {
                break;
            }
        }
    }

    fn skip_disabled_up(&mut self) {
        let len = self.visible_len();
        for _ in 0..len {
            if self.selected_visible_idx_is_disabled() {
                self.state.move_up_wrap(len);
            } else {
                break;
            }
        }
    }

    fn skip_disabled_down_clamped(&mut self) {
        let Some(start) = self.state.selected_idx else {
            return;
        };
        if !self.visible_idx_is_disabled(start) {
            return;
        }

        let len = self.visible_len();
        self.state.selected_idx = ((start + 1)..len)
            .find(|idx| !self.visible_idx_is_disabled(*idx))
            .or_else(|| {
                (0..start)
                    .rev()
                    .find(|idx| !self.visible_idx_is_disabled(*idx))
            })
            .or(Some(start));
    }

    fn skip_disabled_up_clamped(&mut self) {
        let Some(start) = self.state.selected_idx else {
            return;
        };
        if !self.visible_idx_is_disabled(start) {
            return;
        }

        let len = self.visible_len();
        self.state.selected_idx = (0..start)
            .rev()
            .find(|idx| !self.visible_idx_is_disabled(*idx))
            .or_else(|| ((start + 1)..len).find(|idx| !self.visible_idx_is_disabled(*idx)))
            .or(Some(start));
    }

    fn selected_visible_idx_is_disabled(&self) -> bool {
        self.state
            .selected_idx
            .is_some_and(|idx| self.visible_idx_is_disabled(idx))
    }

    fn visible_idx_is_disabled(&self, idx: usize) -> bool {
        self.filtered_indices
            .get(idx)
            .and_then(|actual_idx| self.active_items().get(*actual_idx))
            .is_some_and(|item| item.disabled_reason.is_some() || item.is_disabled)
    }
}

impl BottomPaneView for ListSelectionView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        // 可搜索列表为查询输入保留可打印字符。这样可使
        // Vim 风格的普通 j/k/h/l 在非搜索列表中仍然有用，同时不会
        // 导致这些字母无法输入过滤器。
        let allow_plain_char_navigation =
            !self.is_searchable || !is_plain_text_key_event(key_event);

        match key_event {
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
            _ if allow_plain_char_navigation
                && self.tabs_enabled()
                && self.keymap.move_left.is_pressed(key_event) =>
            {
                self.switch_tab(/*step*/ -1)
            }
            _ if allow_plain_char_navigation
                && self.tabs_enabled()
                && self.keymap.move_right.is_pressed(key_event) =>
            {
                self.switch_tab(/*step*/ 1)
            }
            KeyEvent {
                code: KeyCode::Backspace,
                ..
            } if self.is_searchable => {
                self.search_query.pop();
                self.apply_filter();
            }
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            } if self.selected_item_has_toggle()
                && (!self.is_searchable || self.search_query.is_empty()) =>
            {
                self.toggle_selected()
            }
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            } if self.is_searchable
                && self.search_query.is_empty()
                && self.selected_item_has_toggle_placeholder() => {}
            _ if self.allow_cancel && self.keymap.cancel.is_pressed(key_event) => {
                self.on_ctrl_c();
            }
            _ if self.keymap.accept.is_pressed(key_event) => self.accept(),
            KeyEvent {
                code: KeyCode::Char(c),
                ..
            } if c.is_ascii_control() => {}
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if self.is_searchable
                && !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
            {
                self.search_query.push(c);
                self.apply_filter();
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if !self.is_searchable
                && !modifiers.contains(KeyModifiers::CONTROL)
                && !modifiers.contains(KeyModifiers::ALT) =>
            {
                // 注意:`state.selected_idx` 是「可见索引」(进 filtered_indices),
                // 而 `actual_idx_for_enabled_number` / `display_shortcut` 命中返回的是
                // 「实际索引」(进 active_items)。当 tabs 启用时 active_items != self.items,
                // 且即便没有 tabs,这里也必须把实际索引换算成可见索引再赋给 selected_idx,
                // 否则 `accept()` 里的 `filtered_indices.get(selected_idx)` 会解到错误条目
                // (此前直接把实际索引塞进 selected_idx —— 在 !is_searchable 下
                // filtered_indices 恰好是恒等映射而侥幸正确,但 tabs + display_shortcut
                // 组合会选中错误条目)。统一走 `select_actual_idx` 保证一致。
                if let Some(actual_idx) = self.active_items().iter().position(|item| {
                    item.display_shortcut
                        .is_some_and(|shortcut| shortcut.is_press(key_event))
                        && Self::item_is_enabled(item)
                }) {
                    if self.select_actual_idx(actual_idx) {
                        self.accept();
                    }
                    return;
                }
                if let Some(actual_idx) = c
                    .to_digit(10)
                    .map(|d| d as usize)
                    .and_then(|number| self.actual_idx_for_enabled_number(number))
                {
                    if self.select_actual_idx(actual_idx) {
                        self.accept();
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        if !self.is_searchable {
            return false;
        }
        let Some(pasted) = normalize_pasted_search_query(&pasted) else {
            return false;
        };
        self.search_query.push_str(&pasted);
        self.apply_filter();
        true
    }

    fn is_complete(&self) -> bool {
        self.completion.is_some()
    }

    fn completion(&self) -> Option<ViewCompletion> {
        self.completion
    }

    fn dismiss_after_child_accept(&self) -> bool {
        self.dismiss_after_child_accept
    }

    fn clear_dismiss_after_child_accept(&mut self) {
        self.dismiss_after_child_accept = false;
    }

    fn view_id(&self) -> Option<&'static str> {
        self.view_id
    }

    fn selected_index(&self) -> Option<usize> {
        self.selected_actual_idx()
    }

    fn active_tab_id(&self) -> Option<&str> {
        ListSelectionView::active_tab_id(self)
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        true
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        if !self.allow_cancel {
            return CancellationEvent::NotHandled;
        }
        if let Some(cb) = &self.on_cancel {
            cb(&self.app_event_tx);
        }
        self.completion = Some(ViewCompletion::Cancelled);
        CancellationEvent::Handled
    }
}

impl Renderable for ListSelectionView {
    fn desired_height(&self, width: u16) -> u16 {
        // 减去菜单表面水平内边距（每侧 2）后的内部内容宽度。
        let inner_width = popup_content_width(width);

        // 中文说明：When side-by-side is active, measure the list at the reduced width
        // 中文说明：that accounts for the gap and side panel.
        let effective_rows_width = if let Some(side_w) = self.side_layout_width(inner_width) {
            Self::rows_width(width).saturating_sub(SIDE_CONTENT_GAP + side_w)
        } else {
            Self::rows_width(width)
        };

        // 测量最多 MAX_POPUP_ROWS 个条目的换行高度。
        let rows = self.build_rows();
        let column_width = ColumnWidthConfig::new(self.col_width_mode, self.name_column_width);
        let rows_height = match self.row_display {
            SelectionRowDisplay::Wrapped => measure_rows_height_with_col_width_mode(
                &rows,
                &self.state,
                MAX_POPUP_ROWS,
                effective_rows_width.saturating_add(1),
                column_width,
            ),
            SelectionRowDisplay::SingleLine => rows.len().clamp(1, MAX_POPUP_ROWS) as u16,
        };

        let header = self.active_header();
        let tab_height = tab_bar_height(&self.tabs, self.active_tab_idx.unwrap_or(0), inner_width);
        let mut height = header.desired_height(inner_width);
        height = height.saturating_add(tab_height + u16::from(tab_height > 0));
        height = height.saturating_add(rows_height + 3);
        if self.is_searchable {
            height = height.saturating_add(1);
        }

        // 中文说明：Side content: when the terminal is wide enough the panel sits beside
        // 中文说明：the list and shares vertical space; otherwise it stacks below.
        if self.side_layout_width(inner_width).is_some() {
            // 中文说明：Side-by-side — side content shares list rows vertically so it
            // 中文说明：doesn't add to total height.
        } else {
            let side_h = self.stacked_side_content().desired_height(inner_width);
            if side_h > 0 {
                height = height.saturating_add(1 + side_h);
            }
        }

        if let Some(note) = &self.footer_note {
            let note_width = width.saturating_sub(2);
            let note_lines = wrap_styled_line(note, note_width);
            height = height.saturating_add(note_lines.len() as u16);
        }
        if self.active_footer_hint().is_some() {
            height = height.saturating_add(1);
        }
        height
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let note_width = area.width.saturating_sub(2);
        let note_lines = self
            .footer_note
            .as_ref()
            .map(|note| wrap_styled_line(note, note_width));
        let note_height = note_lines.as_ref().map_or(0, |lines| lines.len() as u16);
        let footer_rows = note_height + u16::from(self.active_footer_hint().is_some());
        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(footer_rows)]).areas(area);

        let outer_content_area = content_area;
        // 绘制共享菜单表面，然后在返回的内缩区域中布局。
        let content_area = render_menu_surface(outer_content_area, buf);

        let inner_width = popup_content_width(outer_content_area.width);
        let side_w = self.side_layout_width(inner_width);

        // 中文说明：When side-by-side is active, shrink the list to make room.
        let full_rows_width = Self::rows_width(outer_content_area.width);
        let effective_rows_width = if let Some(sw) = side_w {
            full_rows_width.saturating_sub(SIDE_CONTENT_GAP + sw)
        } else {
            full_rows_width
        };

        let header = self.active_header();
        let header_height = header.desired_height(inner_width);
        let tab_height = tab_bar_height(&self.tabs, self.active_tab_idx.unwrap_or(0), inner_width);
        let rows = self.build_rows();
        let column_width = ColumnWidthConfig::new(self.col_width_mode, self.name_column_width);
        let rows_height = match self.row_display {
            SelectionRowDisplay::Wrapped => measure_rows_height_with_col_width_mode(
                &rows,
                &self.state,
                MAX_POPUP_ROWS,
                effective_rows_width.saturating_add(1),
                column_width,
            ),
            SelectionRowDisplay::SingleLine => rows.len().clamp(1, MAX_POPUP_ROWS) as u16,
        };

        // 中文说明：Stacked (fallback) side content height — only used when not side-by-side.
        let stacked_side_h = if side_w.is_none() {
            self.stacked_side_content().desired_height(inner_width)
        } else {
            0
        };
        let stacked_gap = if stacked_side_h > 0 { 1 } else { 0 };

        let [
            header_area,
            _,
            tabs_area,
            _,
            search_area,
            list_area,
            _,
            stacked_side_area,
        ] = Layout::vertical([
            Constraint::Max(header_height),
            Constraint::Max(1),
            Constraint::Length(tab_height),
            Constraint::Length(u16::from(tab_height > 0)),
            Constraint::Length(if self.is_searchable { 1 } else { 0 }),
            Constraint::Length(rows_height),
            Constraint::Length(stacked_gap),
            Constraint::Length(stacked_side_h),
        ])
        .areas(content_area);

        // -- 标题 --
        if header_area.height < header_height {
            let [header_area, elision_area] =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(header_area);
            header.render(header_area, buf);
            Paragraph::new(vec![
                Line::from(format!("[… {header_height} lines] ctrl + a view all")).dim(),
            ])
            .render(elision_area, buf);
        } else {
            header.render(header_area, buf);
        }

        // -- 标签页 --
        if tab_height > 0 {
            render_tab_bar(&self.tabs, self.active_tab_idx.unwrap_or(0), tabs_area, buf);
        }

        // -- 搜索栏 --
        if self.is_searchable {
            Line::from(self.search_query.clone()).render(search_area, buf);
            let query_span: Span<'static> = if self.search_query.is_empty() {
                self.search_placeholder
                    .as_ref()
                    .map(|placeholder| placeholder.clone().dim())
                    .unwrap_or_else(|| "".into())
            } else {
                self.search_query.clone().into()
            };
            Line::from(query_span).render(search_area, buf);
        }

        // -- 列表行 --
        if list_area.height > 0 {
            let render_area = Rect {
                x: if rows.is_empty() {
                    list_area.x
                } else {
                    list_area.x.saturating_sub(2)
                },
                y: list_area.y,
                width: effective_rows_width.max(1),
                height: list_area.height,
            };
            match self.row_display {
                SelectionRowDisplay::Wrapped => render_rows_with_col_width_mode(
                    render_area,
                    buf,
                    &rows,
                    &self.state,
                    render_area.height as usize,
                    "no matches",
                    column_width,
                ),
                SelectionRowDisplay::SingleLine => render_rows_single_line_with_col_width_mode(
                    render_area,
                    buf,
                    &rows,
                    &self.state,
                    render_area.height as usize,
                    "no matches",
                    column_width,
                ),
            };
        }

        // -- 侧边内容（预览面板） --
        if let Some(sw) = side_w {
            // 中文说明：Side-by-side: render to the right half of the popup content
            // 中文说明：area so preview content can center vertically in that panel.
            let side_x = content_area.x + content_area.width - sw;
            let side_area = Rect::new(side_x, content_area.y, sw, content_area.height);

            // 中文说明：Clear the menu-surface background behind the side panel so the
            // 中文说明：preview appears on the terminal's own background.
            let clear_x = side_x.saturating_sub(SIDE_CONTENT_GAP);
            let clear_w = outer_content_area
                .x
                .saturating_add(outer_content_area.width)
                .saturating_sub(clear_x);
            Self::clear_to_terminal_bg(
                buf,
                Rect::new(
                    clear_x,
                    outer_content_area.y,
                    clear_w,
                    outer_content_area.height,
                ),
            );
            self.side_content.render(side_area, buf);
            if !self.preserve_side_content_bg {
                Self::force_bg_to_terminal_bg(
                    buf,
                    Rect::new(
                        clear_x,
                        outer_content_area.y,
                        clear_w,
                        outer_content_area.height,
                    ),
                );
            }
        } else if stacked_side_area.height > 0 {
            // 中文说明：Stacked fallback: render below the list (same as old footer_content).
            let clear_height = (outer_content_area.y + outer_content_area.height)
                .saturating_sub(stacked_side_area.y);
            let clear_area = Rect::new(
                outer_content_area.x,
                stacked_side_area.y,
                outer_content_area.width,
                clear_height,
            );
            Self::clear_to_terminal_bg(buf, clear_area);
            self.stacked_side_content().render(stacked_side_area, buf);
        }

        if footer_area.height > 0 {
            let [note_area, hint_area] = Layout::vertical([
                Constraint::Length(note_height),
                Constraint::Length(if self.active_footer_hint().is_some() {
                    1
                } else {
                    0
                }),
            ])
            .areas(footer_area);

            if let Some(lines) = note_lines {
                let note_area = Rect {
                    x: note_area.x + 2,
                    y: note_area.y,
                    width: note_area.width.saturating_sub(2),
                    height: note_area.height,
                };
                for (idx, line) in lines.iter().enumerate() {
                    if idx as u16 >= note_area.height {
                        break;
                    }
                    let line_area = Rect {
                        x: note_area.x,
                        y: note_area.y + idx as u16,
                        width: note_area.width,
                        height: 1,
                    };
                    line.clone().render(line_area, buf);
                }
            }

            if let Some(hint) = self.active_footer_hint() {
                let hint_area = Rect {
                    x: hint_area.x + 2,
                    y: hint_area.y,
                    width: hint_area.width.saturating_sub(2),
                    height: hint_area.height,
                };
                hint.clone().dim().render(hint_area, buf);
            }
        }
    }
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
