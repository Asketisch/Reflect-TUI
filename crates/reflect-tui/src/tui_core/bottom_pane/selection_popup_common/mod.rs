use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
// 注：基于表的布局以前使用 Constraint；下面的手动渲染器不再需要它。
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Widget;
use std::borrow::Cow;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::tui_core::render::Insets;
use crate::tui_core::render::RectExt as _;
use crate::tui_core::style::accent_style;
use crate::tui_core::style::user_message_style;

use super::scroll_state::ScrollState;

/// 选择弹出框中单行的可渲染表示。
///
/// 此类型包含呈现导向的字段，故意比源域模型更具体。
/// `match_indices` 是 `name` 中的字符偏移量，`wrap_indent` 以终端单元格列解释。
#[derive(Default)]
pub(crate) struct GenericDisplayRow {
    pub name: String,
    pub name_prefix_spans: Vec<Span<'static>>,
    pub display_shortcut: Option<KeyBinding>,
    pub match_indices: Option<Vec<usize>>, // indices to bold (char positions)
    pub description: Option<String>,       // optional grey text after the name
    pub category_tag: Option<String>,      // optional right-side category label
    pub disabled_reason: Option<String>,   // optional disabled message
    pub is_disabled: bool,
    pub wrap_indent: Option<usize>, // optional indent for wrapped lines
}

/// 控制选择行如何选择左/右名称/描述列之间的分割。
///
/// 调用者应在测量和渲染时使用相同的模式，否则
/// 弹出框可能会保留错误的行数并裁剪内容。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum ColumnWidthMode {
    /// 仅从可见视口行派生列位置。
    #[default]
    AutoVisible,
    /// 从所有行派生列位置，因此滚动不会移动列。
    AutoAllRows,
    /// 使用固定的两列分割：30% 左（名称），70% 右（描述）。
    Fixed,
}

/// 列宽度行为加上可选的共享左列宽度覆盖。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ColumnWidthConfig {
    pub mode: ColumnWidthMode,
    pub name_column_width: Option<usize>,
}

impl ColumnWidthConfig {
    pub(crate) const fn new(mode: ColumnWidthMode, name_column_width: Option<usize>) -> Self {
        Self {
            mode,
            name_column_width,
        }
    }
}

// 显式固定列模式使用的固定分割：30% 标签，70% 描述。
const FIXED_LEFT_COLUMN_NUMERATOR: usize = 3;
const FIXED_LEFT_COLUMN_DENOMINATOR: usize = 10;

const MENU_SURFACE_INSET_V: u16 = 1;
const MENU_SURFACE_INSET_H: u16 = 2;

/// 应用底部窗格覆盖共享的"菜单表面"内边距。
///
/// 渲染代码应调用 [`render_menu_surface`]，然后在返回的内缩矩形内
/// 布局内容。
pub(crate) fn menu_surface_inset(area: Rect) -> Rect {
    area.inset(Insets::vh(MENU_SURFACE_INSET_V, MENU_SURFACE_INSET_H))
}

/// 菜单表面处理引入的总垂直内边距。
pub(crate) const fn menu_surface_padding_height() -> u16 {
    MENU_SURFACE_INSET_V * 2
}

/// 绘制共享菜单背景并返回内缩后的内容区域。
///
/// 这使选择式覆盖层（例如 `/model`、审批和 request-user-input）的表面处理保持一致。调用方应在返回的矩形而非原始区域中渲染所有内部内容。
pub(crate) fn render_menu_surface(area: Rect, buf: &mut Buffer) -> Rect {
    if area.is_empty() {
        return area;
    }
    Block::default()
        .style(user_message_style())
        .render(area, buf);
    menu_surface_inset(area)
}

/// 在保留 span 样式的同时对样式化行进行换行。
///
/// 此函数将 `width` 限制为至少一个终端单元格，使调用方能在窄布局中安全使用。
// ── 行换行/缩进辅助（外移子模块） ──
mod wrap;
pub(crate) use wrap::*;

fn compute_item_window_start(
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_items: usize,
) -> usize {
    if rows_all.is_empty() || max_items == 0 {
        return 0;
    }

    let mut start_idx = state.scroll_top.min(rows_all.len().saturating_sub(1));
    if let Some(sel) = state.selected_idx {
        if sel < start_idx {
            start_idx = sel;
        } else {
            let bottom = start_idx.saturating_add(max_items.saturating_sub(1));
            if sel > bottom {
                start_idx = sel + 1 - max_items;
            }
        }
    }
    start_idx
}

fn is_selected_visible_in_wrapped_viewport(
    rows_all: &[GenericDisplayRow],
    start_idx: usize,
    max_items: usize,
    selected_idx: usize,
    desc_col: usize,
    width: u16,
    viewport_height: u16,
) -> bool {
    if viewport_height == 0 {
        return false;
    }

    let mut used_lines = 0usize;
    let viewport_height = viewport_height as usize;
    for (idx, row) in rows_all.iter().enumerate().skip(start_idx).take(max_items) {
        let row_lines = wrap_row_lines(row, desc_col, width).len().max(1);
        // 中文说明：Keep rendering semantics in sync: always show the first row, even if
        // 中文说明：it overflows the viewport.
        if used_lines > 0 && used_lines.saturating_add(row_lines) > viewport_height {
            break;
        }
        if idx == selected_idx {
            return true;
        }
        used_lines = used_lines.saturating_add(row_lines);
        if used_lines >= viewport_height {
            break;
        }
    }
    false
}

fn adjust_start_for_wrapped_selection_visibility(
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_items: usize,
    desc_measure_items: usize,
    width: u16,
    viewport_height: u16,
    column_width: ColumnWidthConfig,
) -> usize {
    let mut start_idx = compute_item_window_start(rows_all, state, max_items);
    let Some(sel) = state.selected_idx else {
        return start_idx;
    };
    if viewport_height == 0 {
        return start_idx;
    }

    // 中文说明：If wrapped row heights push the selected item out of view, advance the
    // 中文说明：item window until the selected row is visible.
    while start_idx < sel {
        let desc_col =
            compute_desc_col(rows_all, start_idx, desc_measure_items, width, column_width);
        if is_selected_visible_in_wrapped_viewport(
            rows_all,
            start_idx,
            max_items,
            sel,
            desc_col,
            width,
            viewport_height,
        ) {
            break;
        }
        start_idx = start_idx.saturating_add(1);
    }
    start_idx
}

/// 中文说明：Build the full display line for a row with the description padded to start
/// 中文说明：at `desc_col`. Applies fuzzy-match bolding when indices are present and
/// 中文说明：dims the description.
fn build_full_line(row: &GenericDisplayRow, desc_col: usize) -> Line<'static> {
    let combined_description = match (&row.description, &row.disabled_reason) {
        (Some(desc), Some(reason)) => Some(format!("{desc} (disabled: {reason})")),
        (Some(desc), None) => Some(desc.clone()),
        (None, Some(reason)) => Some(format!("disabled: {reason}")),
        (None, None) => None,
    };

    // 中文说明：Enforce single-line name: allow at most desc_col - 2 cells for name,
    // 中文说明：reserving two spaces before the description column.
    let name_prefix_width = Line::from(row.name_prefix_spans.clone()).width();
    let name_limit = combined_description
        .as_ref()
        .map(|_| desc_col.saturating_sub(2).saturating_sub(name_prefix_width))
        .unwrap_or(usize::MAX);

    let mut name_spans: Vec<Span> = Vec::with_capacity(row.name.len());
    let mut used_width = 0usize;
    let mut truncated = false;

    if let Some(idxs) = row.match_indices.as_ref() {
        let mut idx_iter = idxs.iter().peekable();
        for (char_idx, ch) in row.name.chars().enumerate() {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
            let next_width = used_width.saturating_add(ch_w);
            if next_width > name_limit {
                truncated = true;
                break;
            }
            used_width = next_width;

            if idx_iter.peek().is_some_and(|next| **next == char_idx) {
                idx_iter.next();
                name_spans.push(ch.to_string().bold());
            } else {
                name_spans.push(ch.to_string().into());
            }
        }
    } else {
        for ch in row.name.chars() {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);
            let next_width = used_width.saturating_add(ch_w);
            if next_width > name_limit {
                truncated = true;
                break;
            }
            used_width = next_width;
            name_spans.push(ch.to_string().into());
        }
    }

    if truncated {
        // 中文说明：If there is at least one cell available, add an ellipsis.
        // 中文说明：When name_limit is 0, we still show an ellipsis to indicate truncation.
        name_spans.push("…".into());
    }

    if row.disabled_reason.is_some() {
        name_spans.push(" (disabled)".dim());
    }

    let this_name_width = name_prefix_width + Line::from(name_spans.clone()).width();
    let mut full_spans: Vec<Span> = row.name_prefix_spans.clone();
    full_spans.extend(name_spans);
    if let Some(display_shortcut) = row.display_shortcut {
        full_spans.push(" (".into());
        full_spans.push(display_shortcut.into());
        full_spans.push(")".into());
    }
    if let Some(desc) = combined_description.as_ref() {
        let gap = desc_col.saturating_sub(this_name_width);
        if gap > 0 {
            full_spans.push(" ".repeat(gap).into());
        }
        full_spans.push(desc.clone().dim());
    }
    if let Some(tag) = row.category_tag.as_deref().filter(|tag| !tag.is_empty()) {
        full_spans.push("  ".into());
        full_spans.push(tag.to_string().dim());
    }
    Line::from(full_spans)
}

/// 中文说明：Render a list of rows using the provided ScrollState, with shared styling
/// 中文说明：and behavior for selection popups.
/// 中文说明：Returns the number of terminal lines actually rendered (including the
/// 中文说明：single-line empty placeholder when shown).
fn render_rows_inner(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
    column_width: ColumnWidthConfig,
) -> u16 {
    if rows_all.is_empty() {
        if area.height > 0 {
            Line::from(empty_message.dim().italic()).render(area, buf);
        }
        // 中文说明：Count the placeholder line only when there is vertical space to draw it.
        return u16::from(area.height > 0);
    }

    let max_items = max_results.min(rows_all.len());
    if max_items == 0 {
        return 0;
    }
    let desc_measure_items = max_items.min(area.height.max(1) as usize);

    // 中文说明：Keep item-window semantics, then correct for wrapped row heights so the
    // 中文说明：selected row remains visible in a line-based viewport.
    let start_idx = adjust_start_for_wrapped_selection_visibility(
        rows_all,
        state,
        max_items,
        desc_measure_items,
        area.width,
        area.height,
        column_width,
    );

    let desc_col = compute_desc_col(
        rows_all,
        start_idx,
        desc_measure_items,
        area.width,
        column_width,
    );

    // 中文说明：Render items, wrapping descriptions and aligning wrapped lines under the
    // 中文说明：shared description column. Stop when we run out of vertical space.
    let mut cur_y = area.y;
    let mut rendered_lines: u16 = 0;
    for (i, row) in rows_all.iter().enumerate().skip(start_idx).take(max_items) {
        if cur_y >= area.y + area.height {
            break;
        }

        let mut wrapped = wrap_row_lines(row, desc_col, area.width);
        apply_row_state_style(
            &mut wrapped,
            Some(i) == state.selected_idx && !row.is_disabled,
            row.is_disabled,
        );

        // 中文说明：Render the wrapped lines.
        for line in wrapped {
            if cur_y >= area.y + area.height {
                break;
            }
            line.render(
                Rect {
                    x: area.x,
                    y: cur_y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
            cur_y = cur_y.saturating_add(1);
            rendered_lines = rendered_lines.saturating_add(1);
        }
    }

    rendered_lines
}

/// 中文说明：Render a list of rows using the provided ScrollState, with shared styling
/// 中文说明：and behavior for selection popups.
/// 中文说明：Description alignment is computed from visible rows only, which allows the
/// 中文说明：layout to adapt tightly to the current viewport.
///
/// 中文说明：This function should be paired with [`measure_rows_height`] when reserving
/// 中文说明：space; pairing it with a different measurement mode can cause clipping.
/// 中文说明：Returns the number of terminal lines actually rendered.
pub(crate) fn render_rows(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
) -> u16 {
    render_rows_inner(
        area,
        buf,
        rows_all,
        state,
        max_results,
        empty_message,
        ColumnWidthConfig::default(),
    )
}

/// 中文说明：Render a list of rows using the provided ScrollState and explicit
/// 中文说明：[`ColumnWidthMode`] behavior.
///
/// 中文说明：This is the low-level entry point for callers that need to thread a mode
/// 中文说明：through higher-level configuration.
/// 中文说明：Returns the number of terminal lines actually rendered.
pub(crate) fn render_rows_with_col_width_mode(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
    column_width: ColumnWidthConfig,
) -> u16 {
    render_rows_inner(
        area,
        buf,
        rows_all,
        state,
        max_results,
        empty_message,
        column_width,
    )
}

/// 中文说明：Render rows as a single line each (no wrapping), truncating overflow with an ellipsis.
///
/// 中文说明：This path always uses viewport-local width alignment and is best for dense
/// 中文说明：list UIs where multi-line descriptions would add too much vertical churn.
/// 中文说明：Returns the number of terminal lines actually rendered.
pub(crate) fn render_rows_single_line(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
) -> u16 {
    render_rows_single_line_with_col_width_mode(
        area,
        buf,
        rows_all,
        state,
        max_results,
        empty_message,
        ColumnWidthConfig::default(),
    )
}

/// 中文说明：Render a list of rows as a single line each (no wrapping), truncating overflow with an
/// 中文说明：ellipsis while honoring the configured column width behavior.
pub(crate) fn render_rows_single_line_with_col_width_mode(
    area: Rect,
    buf: &mut Buffer,
    rows_all: &[GenericDisplayRow],
    state: &ScrollState,
    max_results: usize,
    empty_message: &str,
    column_width: ColumnWidthConfig,
) -> u16 {
    if rows_all.is_empty() {
        if area.height > 0 {
            Line::from(empty_message.dim().italic()).render(area, buf);
        }
        // 中文说明：Count the placeholder line only when there is vertical space to draw it.
        return u16::from(area.height > 0);
    }

    let visible_items = max_results
        .min(rows_all.len())
        .min(area.height.max(1) as usize);

    let mut start_idx = state.scroll_top.min(rows_all.len().saturating_sub(1));
    if let Some(sel) = state.selected_idx {
        if sel < start_idx {
            start_idx = sel;
        } else if visible_items > 0 {
            let bottom = start_idx + visible_items - 1;
            if sel > bottom {
                start_idx = sel + 1 - visible_items;
            }
        }
    }

    let desc_col = compute_desc_col(rows_all, start_idx, visible_items, area.width, column_width);

    let mut cur_y = area.y;
    let mut rendered_lines: u16 = 0;
    for (i, row) in rows_all
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(visible_items)
    {
        if cur_y >= area.y + area.height {
            break;
        }

        let mut full_line = build_full_line(row, desc_col);
        if Some(i) == state.selected_idx && !row.is_disabled {
            full_line.spans.iter_mut().for_each(|span| {
                span.style = accent_style();
            });
        }
        if row.is_disabled {
            full_line.spans.iter_mut().for_each(|span| {
                span.style = span.style.dim();
            });
        }

        let full_line = truncate_line_with_ellipsis_if_overflow(full_line, area.width as usize);
        full_line.render(
            Rect {
                x: area.x,
                y: cur_y,
                width: area.width,
                height: 1,
            },
            buf,
        );
        cur_y = cur_y.saturating_add(1);
        rendered_lines = rendered_lines.saturating_add(1);
    }

    rendered_lines
}

/// 中文说明：Compute the number of terminal rows required to render up to `max_results`
/// 中文说明：items from `rows_all` given the current scroll/selection state and the
/// 中文说明：available `width`. Accounts for description wrapping and alignment so the
/// 中文说明：caller can allocate sufficient vertical space.
///
/// 中文说明：This function matches [`render_rows`] semantics (`AutoVisible` column
/// 中文说明：sizing). Mixing it with stable or fixed render modes can under- or
/// 中文说明：over-estimate required height.
// ── 高度测量辅助（外移子模块） ──
mod measure;
pub(crate) use measure::*;

#[cfg(test)]
#[cfg(test)]
mod tests;
