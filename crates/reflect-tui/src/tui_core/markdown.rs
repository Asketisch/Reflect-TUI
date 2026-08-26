//! Markdown 到 ratatui 的渲染入口。
//!
//! 本模块提供 TUI 其余部分用于把 markdown 源码转换为
//! `Vec<Line<'static>>` 的公开 API。共有两个变体：
//!
//! - [`append_markdown`] —— 通用版本，用于计划块以及已经持有
//!   预处理过 markdown 的历史单元（不做围栏解包）。
//! - [`append_markdown_agent`] —— 用于智能体回复。会先运行
//!   [`unwrap_markdown_fences`]，从而剥离包含表格的 `` ```md ``/`` ```markdown ``
//!   围栏，让 `pulldown-cmark` 看到原始表格语法而不是围栏代码块。
//!
//! ## 为什么需要围栏解包
//!
//! LLM 智能体经常把表格包在 `` ```markdown `` 围栏里，当作代码处理。
//! 如果不解包，`pulldown-cmark` 会把这些行解析为围栏代码块，并以等宽代码
//! 而非结构化表格的形式渲染。解包器有意保持保守：它会先缓冲整个围栏体再做
//! 判断，只解包 info 字符串为 `md` 或 `markdown` 且内容中包含
//! 表头+分隔行组合的围栏，并在遇到未闭合围栏时优雅降级。
use ratatui::text::Line;
use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use crate::tui_core::inline_visualization::InlineVisualizationContext;
use crate::tui_core::table_detect;
use crate::tui_core::terminal_hyperlinks::HyperlinkLine;

/// 将 markdown 源码渲染为带样式的 ratatui 行，并追加到 `lines` 中。
///
/// 已经知道会话工作目录的调用方应在此传入该目录，这样即使进程的当前工作目录不同，
/// 流式渲染与非流式渲染也会显示相同的相对路径文本。
pub(crate) fn append_markdown(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
    lines: &mut Vec<Line<'static>>,
) {
    let rendered = crate::tui_core::markdown_render::render_markdown_text_with_width_and_cwd(
        markdown_source,
        width,
        cwd,
    );
    crate::tui_core::render::line_utils::push_owned_lines(&rendered.lines, lines);
}

/// 将智能体消息渲染为带样式的 ratatui 行。
///
/// 渲染之前，源码会先经过 [`unwrap_markdown_fences`]，使包在 `` ```md `` 围栏中的
/// 表格以原生表格而非代码块的形式渲染。
/// 非 markdown 围栏（例如 `rust`、`sh`）保持原样不变。
#[cfg(test)]
pub(crate) fn append_markdown_agent(
    markdown_source: &str,
    width: Option<usize>,
    lines: &mut Vec<Line<'static>>,
) {
    let normalized = unwrap_markdown_fences(markdown_source);
    let rendered = crate::tui_core::markdown_render::render_markdown_text_with_width_and_cwd(
        &normalized,
        width,
        /*cwd*/ None,
    );
    crate::tui_core::render::line_utils::push_owned_lines(&rendered.lines, lines);
}

pub(crate) fn render_markdown_agent_with_links_and_cwd(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
) -> Vec<HyperlinkLine> {
    render_markdown_agent_with_links_cwd_and_visualizations(
        markdown_source,
        width,
        cwd,
        /*inline_visualization_context*/ None,
    )
}

pub(crate) fn render_markdown_agent_with_links_cwd_and_visualizations(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
    inline_visualization_context: Option<&InlineVisualizationContext>,
) -> Vec<HyperlinkLine> {
    let rewritten = crate::tui_core::inline_visualization::rewrite_inline_visualizations(
        markdown_source,
        inline_visualization_context,
    );
    let normalized = unwrap_markdown_fences(&rewritten.markdown);
    let is_hidden_link_destination =
        |destination: &str| rewritten.trusted_file_links.contains_key(destination);
    let mut lines =
        crate::tui_core::markdown_render::render_markdown_lines_with_width_cwd_and_hidden_link_destinations(
            &normalized,
            width,
            cwd,
            &is_hidden_link_destination,
        );
    for hyperlink in lines.iter_mut().flat_map(|line| &mut line.hyperlinks) {
        if let Some(link) = rewritten.trusted_file_links.get(&hyperlink.destination) {
            hyperlink.retarget_to_trusted_file(&link.destination);
        }
    }
    lines
}

/// 渲染智能体消息，并收集增量渲染所需的块元数据。
///
/// 块偏移量会在 Markdown 表格围栏解包之后映射回 `markdown_source`。
/// 如果某个归一化后的边界无法表示为原始源码的后缀，则会被丢弃，从而让
/// 变换后的块保持可变。
pub(crate) fn render_streaming_markdown_agent_with_links_and_cwd(
    markdown_source: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
) -> crate::tui_core::markdown_render::StreamingMarkdownRender {
    let normalized = unwrap_markdown_fences(markdown_source);
    let mut rendered =
        crate::tui_core::markdown_render::render_streaming_markdown_lines_with_width_and_cwd(
            &normalized,
            width,
            cwd,
        );
    if normalized != markdown_source {
        // 围栏解包会移除起始/结束行。归一化后的尾部若仍是原始源码的后缀，
        // 就必然从这些被移除的行之后开始，因此其边界可以安全地映射回原始源码；
        // 否则就让变换后的块保持可变。
        rendered.last_top_level_block_start = rendered
            .last_top_level_block_start
            .and_then(|boundary| markdown_source.strip_suffix(&normalized[boundary..]))
            .map(str::len);
    }
    rendered
}

/// 剥离包含表格的 `` ```md ``/`` ```markdown `` 围栏，把其内容作为裸 markdown 输出，
/// 使 `pulldown-cmark` 能原生解析这些表格。
///
/// info 字符串不是 `md` 或 `markdown` 的围栏会原样透传。*不*包含表格的 markdown
/// 围栏（通过检查是否存在表头行 + 分隔行来判断）同样会透传，从而让围栏内的非表格
/// markdown 仍然渲染为代码块。
///
/// 围栏解包有意保持保守：它会先缓冲整个围栏体再做判断；对输入末尾未闭合的围栏，
/// 会连同其起始行一起重新输出，使不完整的流式内容降级为代码显示。
pub(crate) fn unwrap_markdown_fences<'a>(markdown_source: &'a str) -> Cow<'a, str> {
    // 零拷贝快速路径：大多数消息完全不含围栏。
    if !markdown_source.contains("```") && !markdown_source.contains("~~~") {
        return Cow::Borrowed(markdown_source);
    }

    #[derive(Clone, Copy)]
    struct Fence {
        marker: u8,
        len: usize,
        is_blockquoted: bool,
    }

    // 去掉行尾换行符以及最多 3 个前导空格，返回修剪后的切片。
    // 当该行有 4 个及以上前导空格时返回 `None`
    // （按 CommonMark 规范这会使其成为缩进代码行）。
    fn strip_line_indent(line: &str) -> Option<&str> {
        let without_newline = line.strip_suffix('\n').unwrap_or(line);
        let mut byte_idx = 0usize;
        let mut column = 0usize;
        for b in without_newline.as_bytes() {
            match b {
                b' ' => {
                    byte_idx += 1;
                    column += 1;
                }
                b'\t' => {
                    byte_idx += 1;
                    column += 4;
                }
                _ => break,
            }
            if column >= 4 {
                return None;
            }
        }
        Some(&without_newline[byte_idx..])
    }

    // 解析围栏起始行，返回围栏元数据以及其 info 字符串是否表示 markdown 内容。
    fn parse_open_fence(line: &str) -> Option<(Fence, bool)> {
        let trimmed = strip_line_indent(line)?;
        let is_blockquoted = trimmed.trim_start().starts_with('>');
        let fence_scan_text = table_detect::strip_blockquote_prefix(trimmed);
        let (marker, len) = table_detect::parse_fence_marker(fence_scan_text)?;
        let is_markdown = table_detect::is_markdown_fence_info(fence_scan_text, len);
        Some((
            Fence {
                marker: marker as u8,
                len,
                is_blockquoted,
            },
            is_markdown,
        ))
    }

    fn is_close_fence(line: &str, fence: Fence) -> bool {
        let Some(trimmed) = strip_line_indent(line) else {
            return false;
        };
        let fence_scan_text = if fence.is_blockquoted {
            if !trimmed.trim_start().starts_with('>') {
                return false;
            }
            table_detect::strip_blockquote_prefix(trimmed)
        } else {
            trimmed
        };
        if let Some((marker, len)) = table_detect::parse_fence_marker(fence_scan_text) {
            marker as u8 == fence.marker
                && len >= fence.len
                && fence_scan_text[len..].trim().is_empty()
        } else {
            false
        }
    }

    fn markdown_fence_contains_table(content: &str, is_blockquoted_fence: bool) -> bool {
        let mut previous_line: Option<&str> = None;
        for line in content.lines() {
            let text = if is_blockquoted_fence {
                table_detect::strip_blockquote_prefix(line)
            } else {
                line
            };
            let trimmed = text.trim();
            if trimmed.is_empty() {
                previous_line = None;
                continue;
            }

            if let Some(previous) = previous_line
                && table_detect::is_table_header_line(previous)
                && !table_detect::is_table_delimiter_line(previous)
                && table_detect::is_table_delimiter_line(trimmed)
            {
                return true;
            }

            previous_line = Some(trimmed);
        }
        false
    }

    fn content_from_ranges(source: &str, ranges: &[Range<usize>]) -> String {
        let total_len: usize = ranges.iter().map(ExactSizeIterator::len).sum();
        let mut content = String::with_capacity(total_len);
        for range in ranges {
            content.push_str(&source[range.start..range.end]);
        }
        content
    }

    struct MarkdownCandidateData {
        fence: Fence,
        opening_range: Range<usize>,
        content_ranges: Vec<Range<usize>>,
    }

    // 把较大的变体装箱，以保持 ActiveFence 体积很小（约一个指针大小）。
    enum ActiveFence {
        Passthrough(Fence),
        MarkdownCandidate(Box<MarkdownCandidateData>),
    }

    let mut out = String::with_capacity(markdown_source.len());
    let mut active_fence: Option<ActiveFence> = None;
    let mut source_offset = 0usize;

    let mut push_source_range = |range: Range<usize>| {
        if !range.is_empty() {
            out.push_str(&markdown_source[range]);
        }
    };

    for line in markdown_source.split_inclusive('\n') {
        let line_start = source_offset;
        source_offset += line.len();
        let line_range = line_start..source_offset;

        if let Some(active) = active_fence.take() {
            match active {
                ActiveFence::Passthrough(fence) => {
                    push_source_range(line_range);
                    if !is_close_fence(line, fence) {
                        active_fence = Some(ActiveFence::Passthrough(fence));
                    }
                }
                ActiveFence::MarkdownCandidate(mut data) => {
                    if is_close_fence(line, data.fence) {
                        if markdown_fence_contains_table(
                            &content_from_ranges(markdown_source, &data.content_ranges),
                            data.fence.is_blockquoted,
                        ) {
                            for range in data.content_ranges {
                                push_source_range(range);
                            }
                        } else {
                            push_source_range(data.opening_range);
                            for range in data.content_ranges {
                                push_source_range(range);
                            }
                            push_source_range(line_range);
                        }
                    } else {
                        data.content_ranges.push(line_range);
                        active_fence = Some(ActiveFence::MarkdownCandidate(data));
                    }
                }
            }
            continue;
        }

        if let Some((fence, is_markdown)) = parse_open_fence(line) {
            if is_markdown {
                active_fence = Some(ActiveFence::MarkdownCandidate(Box::new(
                    MarkdownCandidateData {
                        fence,
                        opening_range: line_range,
                        content_ranges: Vec::new(),
                    },
                )));
            } else {
                push_source_range(line_range);
                active_fence = Some(ActiveFence::Passthrough(fence));
            }
            continue;
        }

        push_source_range(line_range);
    }

    if let Some(active) = active_fence {
        match active {
            ActiveFence::Passthrough(_) => {}
            ActiveFence::MarkdownCandidate(data) => {
                push_source_range(data.opening_range);
                for range in data.content_ranges {
                    push_source_range(range);
                }
            }
        }
    }

    Cow::Owned(out)
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::text::Line;

    fn lines_to_strings(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.clone())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn citations_render_as_plain_text() {
        let src = "Before 【F:/x.rs†L1】\nAfter 【F:/x.rs†L3】\n";
        let mut out = Vec::new();
        append_markdown(src, /*width*/ None, /*cwd*/ None, &mut out);
        let rendered = lines_to_strings(&out);
        assert_eq!(rendered.len(), 2);
        assert_eq!(rendered[0], "Before 【F:/x.rs†L1】");
        assert_eq!(rendered[1], "After 【F:/x.rs†L3】");
    }

    #[ignore = "Phase 1: requires full Reflect markdown implementation"]
    fn indented_code_blocks_preserve_leading_whitespace() {
        // 基本合理性检查：带前后空行的缩进代码应产出该缩进行。
        let src = "Before\n\n    code 1\n\nAfter\n";
        let mut out = Vec::new();
        append_markdown(src, /*width*/ None, /*cwd*/ None, &mut out);
        let lines = lines_to_strings(&out);
        // Phase 0 存根：空段落与多行拆分被简化处理。
        assert_eq!(lines, vec!["Before", "    code 1", "After"]);
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_preserves_full_text_line() {
        let src = "Hi! How can I help with reflect-agent today? Want me to explore the repo, run tests, or work on a specific change?\n";
        let mut out = Vec::new();
        append_markdown(src, /*width*/ None, /*cwd*/ None, &mut out);
        assert_eq!(
            out.len(),
            1,
            "expected a single rendered line for plain text"
        );
        let rendered: String = out
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(
            rendered,
            "Hi! How can I help with reflect-agent today? Want me to explore the repo, run tests, or work on a specific change?"
        );
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_matches_tui_markdown_for_ordered_item() {
        let mut out = Vec::new();
        append_markdown(
            "1. Tight item\n",
            /*width*/ None,
            /*cwd*/ None,
            &mut out,
        );
        let lines = lines_to_strings(&out);
        assert_eq!(lines, vec!["1. Tight item".to_string()]);
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_keeps_ordered_list_line_unsplit_in_context() {
        let src = "Loose vs. tight list items:\n1. Tight item\n";
        let mut out = Vec::new();
        append_markdown(src, /*width*/ None, /*cwd*/ None, &mut out);

        let lines = lines_to_strings(&out);

        // 期望有序列表行渲染为单独一行，
        // 而不是拆成只有标记的一行后面再跟文本。
        assert!(
            lines.iter().any(|s| s == "1. Tight item"),
            "expected '1. Tight item' rendered as a single line; got: {lines:?}"
        );
        assert!(
            !lines
                .windows(2)
                .any(|w| w[0].trim_end() == "1." && w[1] == "Tight item"),
            "did not expect a split into ['1.', 'Tight item']; got: {lines:?}"
        );
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_unwraps_markdown_fences_for_table_rendering() {
        let src = "```markdown\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n";
        let mut out = Vec::new();
        append_markdown_agent(src, /*width*/ None, &mut out);
        let rendered = lines_to_strings(&out);
        assert!(rendered.iter().any(|line| line.contains('━')));
        assert!(rendered.iter().any(|line| line.contains(" 1      2")));
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_unwraps_markdown_fences_for_no_outer_table_rendering() {
        let src = "```md\nCol A | Col B | Col C\n--- | --- | ---\nx | y | z\n10 | 20 | 30\n```\n";
        let mut out = Vec::new();
        append_markdown_agent(src, /*width*/ None, &mut out);
        let rendered = lines_to_strings(&out);
        assert!(rendered.iter().any(|line| line.contains('━')));
        assert!(
            rendered
                .iter()
                .any(|line| line.contains(" Col A    Col B    Col C"))
        );
        assert!(
            !rendered
                .iter()
                .any(|line| line.trim() == "Col A | Col B | Col C")
        );
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_unwraps_markdown_fences_for_two_column_no_outer_table() {
        let src = "```md\nA | B\n--- | ---\nleft | right\n```\n";
        let mut out = Vec::new();
        append_markdown_agent(src, /*width*/ None, &mut out);
        let rendered = lines_to_strings(&out);
        assert!(rendered.iter().any(|line| line.contains('━')));
        assert!(rendered.iter().any(|line| line.contains(" left    right")));
        assert!(!rendered.iter().any(|line| line.trim() == "A | B"));
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_unwraps_markdown_fences_for_single_column_table() {
        let src = "```md\n| Only |\n|---|\n| value |\n```\n";
        let mut out = Vec::new();
        append_markdown_agent(src, /*width*/ None, &mut out);
        let rendered = lines_to_strings(&out);
        assert!(rendered.iter().any(|line| line.contains('━')));
        assert!(!rendered.iter().any(|line| line.trim() == "| Only |"));
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_keeps_non_markdown_fences_as_code() {
        let src = "```rust\n| A | B |\n|---|---|\n| 1 | 2 |\n```\n";
        let mut out = Vec::new();
        append_markdown_agent(src, /*width*/ None, &mut out);
        let rendered = lines_to_strings(&out);
        assert_eq!(
            rendered,
            vec![
                "| A | B |".to_string(),
                "|---|---|".to_string(),
                "| 1 | 2 |".to_string(),
            ]
        );
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_unwraps_blockquoted_markdown_fence_table() {
        let src = "> ```markdown\n> | A | B |\n> |---|---|\n> | 1 | 2 |\n> ```\n";
        let rendered = unwrap_markdown_fences(src);
        assert!(
            !rendered.contains("```"),
            "expected markdown fence markers to be removed: {rendered:?}"
        );
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_keeps_non_blockquoted_markdown_fence_with_blockquote_table_example() {
        let src = "```markdown\n> | A | B |\n> |---|---|\n> | 1 | 2 |\n```\n";
        let normalized = unwrap_markdown_fences(src);
        assert_eq!(normalized, src);
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_keeps_markdown_fence_when_content_is_not_table() {
        let src = "```markdown\n**bold**\n```\n";
        let mut out = Vec::new();
        append_markdown_agent(src, /*width*/ None, &mut out);
        let rendered = lines_to_strings(&out);
        assert_eq!(rendered, vec!["**bold**".to_string()]);
    }

    #[test]
    fn unwrap_markdown_fences_repro_keeps_fence_without_header_delimiter_pair() {
        let src = "```markdown\n| A | B |\nnot a delimiter row\n| --- | --- |\n# Heading\n```\n";
        let normalized = unwrap_markdown_fences(src);
        assert_eq!(normalized, src);
    }

    #[test]
    #[ignore = "Phase 1: requires full Reflect implementation"]
    fn append_markdown_agent_keeps_markdown_fence_with_blank_line_between_header_and_delimiter() {
        let src = "```markdown\n| A | B |\n\n|---|---|\n| 1 | 2 |\n```\n";
        let rendered = unwrap_markdown_fences(src);
        assert_eq!(rendered, src);
    }
}
