//! diff 渲染的换行辅助：把语法高亮 span 按终端列宽硬换行，
//! 跨行保留样式信息。从 diff_render.rs 抽出。

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn push_wrapped_diff_line_inner_with_theme_and_color_level(
    line_number: usize,
    kind: DiffLineType,
    text: &str,
    width: usize,
    line_number_width: usize,
    syntax_spans: Option<&[RtSpan<'static>]>,
    theme: DiffTheme,
    color_level: DiffColorLevel,
    diff_backgrounds: ResolvedDiffBackgrounds,
) -> Vec<RtLine<'static>> {
    let ln_str = line_number.to_string();

    // 预留固定数量的空格（等于最宽的行号加上一个尾随间隔），
    // 使符号列在整个 diff 块中保持对齐。
    let gutter_width = line_number_width.max(1);
    let prefix_cols = gutter_width + 1;

    let (sign_char, sign_style, content_style) = match kind {
        DiffLineType::Insert => (
            '+',
            style_sign_add(theme, color_level, diff_backgrounds),
            style_add(theme, color_level, diff_backgrounds),
        ),
        DiffLineType::Delete => (
            '-',
            style_sign_del(theme, color_level, diff_backgrounds),
            style_del(theme, color_level, diff_backgrounds),
        ),
        DiffLineType::Context => (' ', style_context(), style_context()),
    };

    let line_bg = style_line_bg_for(kind, diff_backgrounds);
    let gutter_style = style_gutter_for(kind, theme, color_level);

    // 当我们有语法 spans 时，将其与 diff 样式组合以获得更丰富的
    // 视图。符号字符保留 diff 颜色；内容获得语法颜色
    // 以及删除行的叠加修饰符（暗淡）。
    if let Some(syn_spans) = syntax_spans {
        let gutter = format!("{ln_str:>gutter_width$} ");
        let sign = format!("{sign_char}");
        let styled: Vec<RtSpan<'static>> = syn_spans
            .iter()
            .map(|sp| {
                let style = if matches!(kind, DiffLineType::Delete) {
                    sp.style.add_modifier(Modifier::DIM)
                } else {
                    sp.style
                };
                RtSpan::styled(sp.content.clone().into_owned(), style)
            })
            .collect();

        // 确定在 gutter 和符号字符之后还剩余多少显示列给内容。
        let available_content_cols = width.saturating_sub(prefix_cols + 1).max(1);

        // 换行带样式的内容 spans 以适应可用列数。
        let wrapped_chunks = wrap_styled_spans(&styled, available_content_cols);

        let mut lines: Vec<RtLine<'static>> = Vec::new();
        for (i, chunk) in wrapped_chunks.into_iter().enumerate() {
            let mut row_spans: Vec<RtSpan<'static>> = Vec::new();
            if i == 0 {
                // 第一行：gutter + 符号 + 内容
                row_spans.push(RtSpan::styled(gutter.clone(), gutter_style));
                row_spans.push(RtSpan::styled(sign.clone(), sign_style));
            } else {
                // 续行：空 gutter + 两个空格的缩进（与
                // 纯文本换行续行样式保持一致）。
                let cont_gutter = format!("{:gutter_width$}  ", "");
                row_spans.push(RtSpan::styled(cont_gutter, gutter_style));
            }
            row_spans.extend(chunk);
            lines.push(RtLine::from(row_spans).style(line_bg));
        }
        return lines;
    }

    let available_content_cols = width.saturating_sub(prefix_cols + 1).max(1);
    let styled = vec![RtSpan::styled(text.to_string(), content_style)];
    let wrapped_chunks = wrap_styled_spans(&styled, available_content_cols);

    let mut lines: Vec<RtLine<'static>> = Vec::new();
    for (i, chunk) in wrapped_chunks.into_iter().enumerate() {
        let mut row_spans: Vec<RtSpan<'static>> = Vec::new();
        if i == 0 {
            let gutter = format!("{ln_str:>gutter_width$} ");
            let sign = format!("{sign_char}");
            row_spans.push(RtSpan::styled(gutter, gutter_style));
            row_spans.push(RtSpan::styled(sign, sign_style));
        } else {
            let cont_gutter = format!("{:gutter_width$}  ", "");
            row_spans.push(RtSpan::styled(cont_gutter, gutter_style));
        }
        row_spans.extend(chunk);
        lines.push(RtLine::from(row_spans).style(line_bg));
    }

    lines
}

/// 将带样式的 spans 拆分为适合 `max_cols` 显示列的块。
///
/// 每行输出返回一个 `Vec<RtSpan>`。样式在
/// 分割边界处保持不变，因此换行永远不会丢失语法着色。
///
/// 该算法使用 Unicode 显示宽度（制表符展开为
/// [`TAB_WIDTH`] 列）逐字符遍历。当某个字符超出当前行时，
/// 累积的文本会被刷新并开始新行。当单个字符宽度超过剩余空间时，
/// 在该字符*之前*强制换行，从而始终取得进展（避免在行末的
/// CJK 字符或制表符上出现无限循环）。
pub(super) fn wrap_styled_spans(
    spans: &[RtSpan<'static>],
    max_cols: usize,
) -> Vec<Vec<RtSpan<'static>>> {
    let mut result: Vec<Vec<RtSpan<'static>>> = Vec::new();
    let mut current_line: Vec<RtSpan<'static>> = Vec::new();
    let mut col: usize = 0;

    for span in spans {
        let style = span.style;
        let text = span.content.as_ref();
        let mut remaining = text;

        while !remaining.is_empty() {
            // 累积字符直到填满一行。
            let mut byte_end = 0;
            let mut chars_col = 0;

            for ch in remaining.chars() {
                // 制表符没有 Unicode 宽度；将它们视为 TAB_WIDTH 列。
                let w = ch.width().unwrap_or(if ch == '\t' { TAB_WIDTH } else { 0 });
                if col + chars_col + w > max_cols {
                    // 添加此字符会超出行的宽度。
                    // 在此处中断；如果是 `remaining` 中的第一个字符，
                    // 我们将在下面的 `byte_end == 0` 分支中
                    // 在消费它之前刷新/开始新行。
                    break;
                }
                byte_end += ch.len_utf8();
                chars_col += w;
            }

            if byte_end == 0 {
                // 单个字符宽度超过剩余空间——强制换到
                // 新行以便取得进展。
                if !current_line.is_empty() {
                    result.push(std::mem::take(&mut current_line));
                }
                // 至少取一个字符以避免无限循环。
                let Some(ch) = remaining.chars().next() else {
                    break;
                };
                let ch_len = ch.len_utf8();
                current_line.push(RtSpan::styled(
                    remaining[..ch_len].replace('\t', TAB_REPLACEMENT),
                    style,
                ));
                // 使用回退宽度 1（而不是 0），以便此分支始终推进
                // 即使 `ch` 具有未知/零显示宽度。
                col = ch.width().unwrap_or(if ch == '\t' { TAB_WIDTH } else { 1 });
                remaining = &remaining[ch_len..];
                continue;
            }

            let (chunk, rest) = remaining.split_at(byte_end);
            current_line.push(RtSpan::styled(chunk.replace('\t', TAB_REPLACEMENT), style));
            col += chars_col;
            remaining = rest;

            // 如果正好填满或超出当前行，则开始新行。
            // 不要根据 !remaining.is_empty() 进行门控——外层循环中
            // 的下一个 span 可能仍有必须在新行开始的内容。
            if col >= max_cols {
                result.push(std::mem::take(&mut current_line));
                col = 0;
            }
        }
    }

    // 推送最后一行（始终至少一行，即使为空）。
    if !current_line.is_empty() || result.is_empty() {
        result.push(current_line);
    }

    result
}
