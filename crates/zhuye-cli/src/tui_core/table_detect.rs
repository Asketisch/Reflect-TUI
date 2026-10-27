//! 标准管道表结构检测和原始 markdown 源码中的围栏代码块跟踪。
//!
//! 流式控制器（`streaming/controller.rs`）和 markdown 围栏解包器（`markdown.rs`）
//! 都需要识别原始 markdown 源码中的管道表结构和围栏代码块。本模块提供规范实现，
//! 这样修复只需要在一个地方进行。
//!
//! ## 概念
//!
//! GFM 管道表是行的序列，其中：
//! - **标题行**包含管道分隔的段，至少有一个非空单元格。
//! - **分隔符行**紧接标题后，仅包含对齐标记（`---`、`:---`、`---:`、`:---:`），
//!   每个至少三个短划线。
//! - **体行**跟随分隔符。
//!
//! **围栏代码块**以 3+ 反引号或波浪号开始，以匹配的关闭标记结束。[`FenceTracker`]
//! 将每行分类为 [`FenceKind::Outside`]、[`FenceKind::Markdown`] 或 [`FenceKind::Other`]，
//! 使调用者能跳过出现在非 markdown 围栏内的管道字符。
//!
//! 表函数在单行上操作，不维护跨行状态。调用者（流式控制器和围栏解包器）负责
//! 配对连续行以确认表。

/// 将管道分隔的行拆分为修剪后的段。
///
/// 如果行为空或没有未转义的分隔符标记，则返回 `None`。
/// 在拆分之前，剥离前导/尾随管道。
///
/// 这故意是结构解析器，而不是渲染器。它保留返回段内的
/// 转义管道，因为调用者只关心行是否能参与表格，
/// 而不关心单元格文本最终如何显示。
pub(crate) fn parse_table_segments(line: &str) -> Option<Vec<&str>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let has_outer_pipe = trimmed.starts_with('|') || trimmed.ends_with('|');
    let content = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let content = content.strip_suffix('|').unwrap_or(content);
    let raw_segments = split_unescaped_pipe(content);
    if !has_outer_pipe && raw_segments.len() <= 1 {
        return None;
    }

    let segments: Vec<&str> = raw_segments.into_iter().map(str::trim).collect();
    (!segments.is_empty()).then_some(segments)
}

/// 在未转义的 `|` 字符上拆分 `content`。
///
/// 由 `\` 前面的管道被视为字面文本，而不是列分隔符。
/// 反斜杠保留在段中（这是结构检测，不是渲染）。
fn split_unescaped_pipe(content: &str) -> Vec<&str> {
    let mut segments = Vec::with_capacity(8);
    let mut start = 0;
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // 跳过被转义的字符。
            i += 2;
        } else if bytes[i] == b'|' {
            segments.push(&content[start..i]);
            start = i + 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    segments.push(&content[start..]);
    segments
}

// 小表检测辅助函数内联用于流式热路径——它们在增量保持扫描期间
// 在每个源行上调用。

/// `line` 是否看起来像表标题行（具有管道分隔的
/// 段，至少有一个非空单元格）。
#[inline]
pub(crate) fn is_table_header_line(line: &str) -> bool {
    parse_table_segments(line).is_some_and(|segments| segments.iter().any(|s| !s.is_empty()))
}

/// 单个段是否匹配 markdown 表分隔符行中使用的 `---`、`:---`、`---:` 或 `:---:`
/// 对齐冒号语法。
#[inline]
fn is_table_delimiter_segment(segment: &str) -> bool {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return false;
    }
    let without_leading = trimmed.strip_prefix(':').unwrap_or(trimmed);
    let without_ends = without_leading.strip_suffix(':').unwrap_or(without_leading);
    without_ends.len() >= 3 && without_ends.chars().all(|c| c == '-')
}

/// `line` 是否是有效的表分隔符行（每个段通过 [`is_table_delimiter_segment`]）。
#[inline]
pub(crate) fn is_table_delimiter_line(line: &str) -> bool {
    parse_table_segments(line)
        .is_some_and(|segments| segments.into_iter().all(is_table_delimiter_segment))
}

// ---------------------------------------------------------------------------
// 围栏代码块跟踪
// ---------------------------------------------------------------------------

/// 源行相对于围栏代码块的位置。
///
/// 表保持仅适用于 `Outside` 或 `Markdown` 围栏内的行。`Other` 围栏（例如 `sh`、`rust`）内的行
/// 被表扫描器忽略，因为它们的管道字符是代码，而不是
/// 表语法。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FenceKind {
    /// 不在任何围栏代码块内。
    Outside,
    /// 在 `` ```md `` 或 `` ```markdown `` 围栏内。
    Markdown,
    /// 在具有非 markdown 信息字符串的围栏内。
    Other,
}

/// 围栏代码块打开/关闭转换的增量跟踪器。
///
/// 通过 [`advance`](Self::advance) 逐行馈送；使用 [`kind`](Self::kind) 查询当前
/// 上下文。跟踪器处理前导空白
/// 限制（>3 空格 → 不是围栏）、引用块前缀剥离和
/// 反引号/波浪号标记匹配。
///
/// 跟踪器报告适用于当前行的围栏上下文
/// 在该行改变状态之前。调用者在决定
/// 当前原始行是否可以打开或继续表时依赖于此。
pub(crate) struct FenceTracker {
    state: Option<(char, usize, FenceKind)>,
}

impl FenceTracker {
    #[inline]
    pub(crate) fn new() -> Self {
        Self { state: None }
    }

    /// 处理一行原始源码并更新围栏状态。
    ///
    /// 缩进超过 3 个空格的行会被忽略（属于缩进式代码块而非围栏）。
    /// 扫描前会先剥离块引用前缀（`>`）。
    pub(crate) fn advance(&mut self, raw_line: &str) {
        let leading_spaces = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| **byte == b' ')
            .count();
        if leading_spaces > 3 {
            return;
        }

        let trimmed = &raw_line[leading_spaces..];
        let fence_scan_text = strip_blockquote_prefix(trimmed);
        if let Some((marker, len)) = parse_fence_marker(fence_scan_text) {
            if let Some((open_char, open_len, _)) = self.state {
                // 标记匹配则关闭当前围栏。
                if marker == open_char
                    && len >= open_len
                    && fence_scan_text[len..].trim().is_empty()
                {
                    self.state = None;
                }
            } else {
                // 打开新的围栏。
                let kind = if is_markdown_fence_info(fence_scan_text, len) {
                    FenceKind::Markdown
                } else {
                    FenceKind::Other
                };
                self.state = Some((marker, len, kind));
            }
        }
    }

    /// 最近一次 `advance` 处理行的围栏上下文。
    #[inline]
    pub(crate) fn kind(&self) -> FenceKind {
        self.state.map_or(FenceKind::Outside, |(_, _, k)| k)
    }
}

/// 返回潜在围栏行的围栏标记字符和运行长度。
///
/// 识别反引号和波浪号围栏，最小运行长度为 3。
/// 输入应该已经剥离了前导空白和块引用前缀。
#[inline]
pub(crate) fn parse_fence_marker(line: &str) -> Option<(char, usize)> {
    let first = line.as_bytes().first().copied()?;
    if first != b'`' && first != b'~' {
        return None;
    }
    let len = line.bytes().take_while(|&b| b == first).count();
    if len < 3 {
        return None;
    }
    Some((first as char, len))
}

/// 围栏标记后的信息字符串是否指示 markdown 内容。
///
/// 匹配 `md` 和 `markdown`（不区分大小写）。
#[inline]
pub(crate) fn is_markdown_fence_info(trimmed_line: &str, marker_len: usize) -> bool {
    let info = trimmed_line[marker_len..]
        .split_whitespace()
        .next()
        .unwrap_or_default();
    info.eq_ignore_ascii_case("md") || info.eq_ignore_ascii_case("markdown")
}

/// 从行中剥离所有前导 `>` 块引用标记。
///
/// 表可以出现在块引用内（`> | A | B |`），因此保持
/// 扫描器在检查表语法之前必须剥离这些标记。
#[inline]
pub(crate) fn strip_blockquote_prefix(line: &str) -> &str {
    let mut rest = line.trim_start();
    loop {
        let Some(stripped) = rest.strip_prefix('>') else {
            return rest;
        };
        rest = stripped.strip_prefix(' ').unwrap_or(stripped).trim_start();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_table_segments_basic() {
        assert_eq!(
            parse_table_segments("| A | B | C |"),
            Some(vec!["A", "B", "C"])
        );
    }

    #[test]
    fn parse_table_segments_no_outer_pipes() {
        assert_eq!(parse_table_segments("A | B | C"), Some(vec!["A", "B", "C"]));
    }

    #[test]
    fn parse_table_segments_no_leading_pipe() {
        assert_eq!(
            parse_table_segments("A | B | C |"),
            Some(vec!["A", "B", "C"])
        );
    }

    #[test]
    fn parse_table_segments_no_trailing_pipe() {
        assert_eq!(
            parse_table_segments("| A | B | C"),
            Some(vec!["A", "B", "C"])
        );
    }

    #[test]
    fn parse_table_segments_single_segment_is_allowed() {
        assert_eq!(parse_table_segments("| only |"), Some(vec!["only"]));
    }

    #[test]
    fn parse_table_segments_without_pipe_returns_none() {
        assert_eq!(parse_table_segments("just text"), None);
    }

    #[test]
    fn parse_table_segments_empty_returns_none() {
        assert_eq!(parse_table_segments(""), None);
        assert_eq!(parse_table_segments("   "), None);
    }

    #[test]
    fn parse_table_segments_escaped_pipe() {
        // 转义的管道不应拆分 —— 保留在同一段内。
        assert_eq!(
            parse_table_segments(r"| A \| B | C |"),
            Some(vec![r"A \| B", "C"])
        );
    }

    #[test]
    fn is_table_delimiter_segment_valid() {
        assert!(is_table_delimiter_segment("---"));
        assert!(is_table_delimiter_segment(":---"));
        assert!(is_table_delimiter_segment("---:"));
        assert!(is_table_delimiter_segment(":---:"));
        assert!(is_table_delimiter_segment(":-------:"));
    }

    #[test]
    fn is_table_delimiter_segment_invalid() {
        assert!(!is_table_delimiter_segment(""));
        assert!(!is_table_delimiter_segment("--"));
        assert!(!is_table_delimiter_segment("abc"));
        assert!(!is_table_delimiter_segment(":--"));
    }

    #[test]
    fn is_table_delimiter_line_valid() {
        assert!(is_table_delimiter_line("| --- | --- |"));
        assert!(is_table_delimiter_line("|:---:|---:|"));
        assert!(is_table_delimiter_line("--- | --- | ---"));
    }

    #[test]
    fn is_table_delimiter_line_invalid() {
        assert!(!is_table_delimiter_line("| A | B |"));
        assert!(!is_table_delimiter_line("| -- | -- |"));
    }

    #[test]
    fn is_table_header_line_valid() {
        assert!(is_table_header_line("| A | B |"));
        assert!(is_table_header_line("Name | Value"));
    }

    #[test]
    fn is_table_header_line_all_empty_segments() {
        assert!(!is_table_header_line("| | |"));
    }

    // -----------------------------------------------------------------------
    // FenceTracker 测试
    // -----------------------------------------------------------------------

    #[test]
    fn fence_tracker_outside_by_default() {
        let tracker = FenceTracker::new();
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_opens_and_closes_backtick_fence() {
        let mut tracker = FenceTracker::new();
        tracker.advance("```rust");
        assert_eq!(tracker.kind(), FenceKind::Other);

        tracker.advance("let x = 1;");
        assert_eq!(tracker.kind(), FenceKind::Other);

        tracker.advance("```");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_opens_and_closes_tilde_fence() {
        let mut tracker = FenceTracker::new();
        tracker.advance("~~~python");
        assert_eq!(tracker.kind(), FenceKind::Other);
        tracker.advance("~~~");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_markdown_fence() {
        let mut tracker = FenceTracker::new();
        tracker.advance("```md");
        assert_eq!(tracker.kind(), FenceKind::Markdown);
        tracker.advance("| A | B |");
        assert_eq!(tracker.kind(), FenceKind::Markdown);
        tracker.advance("```");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_markdown_case_insensitive() {
        let mut tracker = FenceTracker::new();
        tracker.advance("```Markdown");
        assert_eq!(tracker.kind(), FenceKind::Markdown);
        tracker.advance("```");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_nested_shorter_marker_does_not_close() {
        let mut tracker = FenceTracker::new();
        tracker.advance("````sh");
        assert_eq!(tracker.kind(), FenceKind::Other);
        // 内部更短的标记不能关闭外层围栏。
        tracker.advance("```");
        assert_eq!(tracker.kind(), FenceKind::Other);
        // 长度匹配的标记才能关闭。
        tracker.advance("````");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_mismatched_char_does_not_close() {
        let mut tracker = FenceTracker::new();
        tracker.advance("```sh");
        assert_eq!(tracker.kind(), FenceKind::Other);
        // 波浪号标记不应关闭反引号围栏。
        tracker.advance("~~~");
        assert_eq!(tracker.kind(), FenceKind::Other);
        tracker.advance("```");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_indented_4_spaces_ignored() {
        let mut tracker = FenceTracker::new();
        tracker.advance("    ```sh");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_blockquote_prefix_stripped() {
        let mut tracker = FenceTracker::new();
        tracker.advance("> ```sh");
        assert_eq!(tracker.kind(), FenceKind::Other);
        tracker.advance("> ```");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    #[test]
    fn fence_tracker_close_with_trailing_content_does_not_close() {
        let mut tracker = FenceTracker::new();
        tracker.advance("```sh");
        assert_eq!(tracker.kind(), FenceKind::Other);
        // 尾部内容会阻止关闭。
        tracker.advance("``` extra");
        assert_eq!(tracker.kind(), FenceKind::Other);
        tracker.advance("```");
        assert_eq!(tracker.kind(), FenceKind::Outside);
    }

    // -----------------------------------------------------------------------
    // 围栏辅助函数测试
    // -----------------------------------------------------------------------

    #[test]
    fn parse_fence_marker_backtick() {
        assert_eq!(parse_fence_marker("```rust"), Some(('`', 3)));
        assert_eq!(parse_fence_marker("````"), Some(('`', 4)));
    }

    #[test]
    fn parse_fence_marker_tilde() {
        assert_eq!(parse_fence_marker("~~~python"), Some(('~', 3)));
    }

    #[test]
    fn parse_fence_marker_too_short() {
        assert_eq!(parse_fence_marker("``"), None);
        assert_eq!(parse_fence_marker("~~"), None);
    }

    #[test]
    fn parse_fence_marker_not_fence() {
        assert_eq!(parse_fence_marker("hello"), None);
        assert_eq!(parse_fence_marker(""), None);
    }

    #[test]
    fn is_markdown_fence_info_basic() {
        assert!(is_markdown_fence_info("```md", /*marker_len*/ 3));
        assert!(is_markdown_fence_info("```markdown", /*marker_len*/ 3));
        assert!(is_markdown_fence_info("```MD", /*marker_len*/ 3));
        assert!(!is_markdown_fence_info("```rust", /*marker_len*/ 3));
        assert!(!is_markdown_fence_info("```", /*marker_len*/ 3));
    }

    #[test]
    fn strip_blockquote_prefix_basic() {
        assert_eq!(strip_blockquote_prefix("> hello"), "hello");
        assert_eq!(strip_blockquote_prefix("> > nested"), "nested");
        assert_eq!(strip_blockquote_prefix("no prefix"), "no prefix");
    }
}
