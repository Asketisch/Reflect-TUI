//! 选择弹窗行换行/缩进辅助函数簇。从 selection_popup_common.rs 抽出。

use super::*;

pub(crate) fn wrap_styled_line<'a>(line: &'a Line<'a>, width: u16) -> Vec<Line<'a>> {
    use crate::tui_core::wrapping::RtOptions;
    use crate::tui_core::wrapping::word_wrap_line;

    let width = width.max(1) as usize;
    let opts = RtOptions::new(width)
        .initial_indent(Line::from(""))
        .subsequent_indent(Line::from(""));
    word_wrap_line(line, opts)
}

pub(super) fn line_to_owned(line: Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .into_iter()
            .map(|span| Span {
                style: span.style,
                content: Cow::Owned(span.content.into_owned()),
            })
            .collect(),
    }
}

pub(super) fn compute_desc_col(
    rows_all: &[GenericDisplayRow],
    start_idx: usize,
    visible_items: usize,
    content_width: u16,
    column_width: ColumnWidthConfig,
) -> usize {
    if content_width <= 1 {
        return 0;
    }

    let max_desc_col = content_width.saturating_sub(1) as usize;
    // 中文说明：Reuse the existing fixed split constants to derive the auto cap:
    // 中文说明：if fixed mode is 30/70 (label/description), auto mode caps label width
    // 中文说明：at 70% to keep at least 30% available for descriptions.
    let max_auto_desc_col = max_desc_col.min(
        ((content_width as usize * (FIXED_LEFT_COLUMN_DENOMINATOR - FIXED_LEFT_COLUMN_NUMERATOR))
            / FIXED_LEFT_COLUMN_DENOMINATOR)
            .max(1),
    );
    match column_width.mode {
        ColumnWidthMode::Fixed => ((content_width as usize * FIXED_LEFT_COLUMN_NUMERATOR)
            / FIXED_LEFT_COLUMN_DENOMINATOR)
            .clamp(1, max_desc_col),
        ColumnWidthMode::AutoVisible | ColumnWidthMode::AutoAllRows => {
            let max_name_width = match column_width.mode {
                ColumnWidthMode::AutoVisible => rows_all
                    .iter()
                    .enumerate()
                    .skip(start_idx)
                    .take(visible_items)
                    .map(|(_, row)| {
                        let mut spans = row.name_prefix_spans.clone();
                        spans.push(row.name.clone().into());
                        if row.disabled_reason.is_some() {
                            spans.push(" (disabled)".dim());
                        }
                        Line::from(spans).width()
                    })
                    .max()
                    .unwrap_or(0),
                ColumnWidthMode::AutoAllRows => rows_all
                    .iter()
                    .map(|row| {
                        let mut spans = row.name_prefix_spans.clone();
                        spans.push(row.name.clone().into());
                        if row.disabled_reason.is_some() {
                            spans.push(" (disabled)".dim());
                        }
                        Line::from(spans).width()
                    })
                    .max()
                    .unwrap_or(0),
                ColumnWidthMode::Fixed => 0,
            };

            column_width
                .name_column_width
                .map(|width| width.max(max_name_width))
                .unwrap_or(max_name_width)
                .saturating_add(2)
                .min(max_auto_desc_col)
        }
    }
}

/// 确定某行的换行续行应缩进多少空格。
pub(super) fn wrap_indent(row: &GenericDisplayRow, desc_col: usize, max_width: u16) -> usize {
    let max_indent = max_width.saturating_sub(1) as usize;
    let indent = row.wrap_indent.unwrap_or_else(|| {
        if row.description.is_some() || row.disabled_reason.is_some() {
            desc_col
        } else {
            0
        }
    });
    indent.min(max_indent)
}

pub(super) fn should_wrap_name_in_column(row: &GenericDisplayRow) -> bool {
    // 中文说明：This path intentionally targets plain option rows that opt into wrapped
    // 中文说明：labels. Styled/fuzzy-matched rows keep the legacy combined-line path.
    row.wrap_indent.is_some()
        && row.description.is_some()
        && row.disabled_reason.is_none()
        && row.match_indices.is_none()
        && row.display_shortcut.is_none()
        && row.category_tag.is_none()
        && row.name_prefix_spans.is_empty()
}

pub(super) fn wrap_two_column_row(
    row: &GenericDisplayRow,
    desc_col: usize,
    width: u16,
) -> Vec<Line<'static>> {
    let Some(description) = row.description.as_deref() else {
        return Vec::new();
    };

    let width = width.max(1);
    let max_desc_col = width.saturating_sub(1) as usize;
    if max_desc_col == 0 {
        // 中文说明：No valid description column exists at this width; let callers fall
        // 中文说明：back to single-line wrapping path.
        return Vec::new();
    }

    let desc_col = desc_col.clamp(1, max_desc_col);
    let left_width = desc_col.saturating_sub(2).max(1);
    let right_width = width.saturating_sub(desc_col as u16).max(1) as usize;
    let name_wrap_indent = row
        .wrap_indent
        .unwrap_or(0)
        .min(left_width.saturating_sub(1));

    let name_subsequent_indent = " ".repeat(name_wrap_indent);
    let name_options = textwrap::Options::new(left_width)
        .initial_indent("")
        .subsequent_indent(name_subsequent_indent.as_str());
    let name_lines = textwrap::wrap(row.name.as_str(), name_options);

    let desc_options = textwrap::Options::new(right_width).initial_indent("");
    let desc_lines = textwrap::wrap(description, desc_options);

    let rows = name_lines.len().max(desc_lines.len()).max(1);
    let mut out = Vec::with_capacity(rows);
    for idx in 0..rows {
        let mut spans: Vec<Span<'static>> = Vec::new();
        if let Some(name) = name_lines.get(idx) {
            spans.push(name.to_string().into());
        }

        if let Some(desc) = desc_lines.get(idx) {
            let left_used = spans
                .iter()
                .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
                .sum::<usize>();
            let gap = if left_used == 0 {
                desc_col
            } else {
                desc_col.saturating_sub(left_used).max(2)
            };
            if gap > 0 {
                spans.push(" ".repeat(gap).into());
            }
            spans.push(desc.to_string().dim());
        }

        out.push(Line::from(spans));
    }

    out
}

pub(super) fn wrap_standard_row(
    row: &GenericDisplayRow,
    desc_col: usize,
    width: u16,
) -> Vec<Line<'static>> {
    use crate::tui_core::wrapping::RtOptions;
    use crate::tui_core::wrapping::word_wrap_line;

    let full_line = build_full_line(row, desc_col);
    let continuation_indent = wrap_indent(row, desc_col, width);
    let options = RtOptions::new(width.max(1) as usize)
        .initial_indent(Line::from(""))
        .subsequent_indent(Line::from(" ".repeat(continuation_indent)));
    word_wrap_line(&full_line, options)
        .into_iter()
        .map(line_to_owned)
        .collect()
}

pub(super) fn wrap_row_lines(
    row: &GenericDisplayRow,
    desc_col: usize,
    width: u16,
) -> Vec<Line<'static>> {
    if should_wrap_name_in_column(row) {
        let wrapped = wrap_two_column_row(row, desc_col, width);
        if !wrapped.is_empty() {
            return wrapped;
        }
    }

    wrap_standard_row(row, desc_col, width)
}

pub(super) fn apply_row_state_style(
    lines: &mut [Line<'static>],
    selected: bool,
    is_disabled: bool,
) {
    if selected {
        for line in lines.iter_mut() {
            line.spans.iter_mut().for_each(|span| {
                span.style = accent_style();
            });
        }
    }
    if is_disabled {
        for line in lines.iter_mut() {
            line.spans.iter_mut().for_each(|span| {
                span.style = span.style.dim();
            });
        }
    }
}
