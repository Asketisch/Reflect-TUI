//! 用户、助手、推理以及流式消息的历史单元。

use super::markdown_render_cache::MarkdownRenderCache;
use super::*;

#[derive(Debug)]
pub(crate) struct UserHistoryCell {
    pub message: String,
    pub text_elements: Vec<TextElement>,
    #[allow(dead_code)]
    pub local_image_paths: Vec<PathBuf>,
    pub remote_image_urls: Vec<String>,
}

/// 移除 CSI 转义序列和控制字符，保留制表符和换行符。
pub(crate) fn sanitize_user_text(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.next_if_eq(&'[').is_some() {
            let _ = chars.find(|ch| ('@'..='~').contains(ch));
        } else if matches!(ch, '\n' | '\t') || !ch.is_control() {
            sanitized.push(ch);
        }
    }
    sanitized
}

/// 为带样式文本元素的用户消息构建逻辑行。
///
/// 此函数保留显式换行符的同时交错元素片段，并在历史渲染时跳过格式错误的字节范围，
/// 而不是直接报错。
fn build_user_message_lines_with_elements(
    message: &str,
    elements: &[TextElement],
    style: Style,
    element_style: Style,
) -> Vec<Line<'static>> {
    let mut elements = elements.to_vec();
    elements.sort_by_key(|e| e.byte_range.start);
    let mut offset = 0usize;
    let mut raw_lines: Vec<Line<'static>> = Vec::new();
    for line_text in message.split('\n') {
        let line_start = offset;
        let line_end = line_start + line_text.len();
        let mut spans: Vec<Span<'static>> = Vec::new();
        // 跟踪当前行已输出的字符量，以便交错排版纯文本片段和带样式的片段。
        let mut cursor = line_start;
        for elem in &elements {
            let start = elem.byte_range.start.max(line_start);
            let end = elem.byte_range.end.min(line_end);
            if start >= end {
                continue;
            }
            let rel_start = start - line_start;
            let rel_end = end - line_start;
            // 防止来自上游数据的格式错误的 UTF-8 字节范围；在渲染历史时跳过无效元素，
            // 而不是直接触发 panic。
            if !line_text.is_char_boundary(rel_start) || !line_text.is_char_boundary(rel_end) {
                continue;
            }
            let rel_cursor = cursor - line_start;
            if cursor < start
                && line_text.is_char_boundary(rel_cursor)
                && let Some(segment) = line_text.get(rel_cursor..rel_start)
            {
                spans.push(Span::from(segment.to_string()));
            }
            if let Some(segment) = line_text.get(rel_start..rel_end) {
                spans.push(Span::styled(segment.to_string(), element_style));
                cursor = end;
            }
        }
        let rel_cursor = cursor - line_start;
        if cursor < line_end
            && line_text.is_char_boundary(rel_cursor)
            && let Some(segment) = line_text.get(rel_cursor..)
        {
            spans.push(Span::from(segment.to_string()));
        }
        let line = if spans.is_empty() {
            Line::from(line_text.to_string()).style(style)
        } else {
            Line::from(spans).style(style)
        };
        raw_lines.push(line);
        // 按 '\n' 切分，以便让任何 '\r' 保留在该行中；前进 1 是为了跳过分隔字节。
        offset = line_end + 1;
    }

    raw_lines
}

fn remote_image_display_line(style: Style, index: usize) -> Line<'static> {
    Line::from(local_image_label_text(index)).style(style)
}

fn trim_trailing_blank_lines(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while lines
        .last()
        .is_some_and(|line| line.spans.iter().all(|span| span.content.trim().is_empty()))
    {
        lines.pop();
    }
    lines
}

impl HistoryCell for UserHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let message = sanitize_user_text(&self.message);
        let text_elements = if message == self.message {
            self.text_elements.as_slice()
        } else {
            &[]
        };
        let wrap_width = width
            .saturating_sub(
                LIVE_PREFIX_COLS + 1, /* 保留一列右侧边距用于自动换行 */
            )
            .max(1);

        let style = user_message_style();
        let element_style = style.fg(Color::Cyan);

        let wrapped_remote_images = if self.remote_image_urls.is_empty() {
            None
        } else {
            Some(adaptive_wrap_lines(
                self.remote_image_urls
                    .iter()
                    .enumerate()
                    .map(|(idx, _url)| {
                        remote_image_display_line(element_style, idx.saturating_add(1))
                    }),
                RtOptions::new(usize::from(wrap_width))
                    .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit),
            ))
        };

        let wrapped_message = if message.is_empty() && text_elements.is_empty() {
            None
        } else if text_elements.is_empty() {
            let message_without_trailing_newlines = message.trim_end_matches(['\r', '\n']);
            let wrapped = adaptive_wrap_lines(
                message_without_trailing_newlines
                    .split('\n')
                    .map(|line| Line::from(line).style(style)),
                // 换行算法与 textarea.rs 保持一致。
                RtOptions::new(usize::from(wrap_width))
                    .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit),
            );
            let wrapped = trim_trailing_blank_lines(wrapped);
            (!wrapped.is_empty()).then_some(wrapped)
        } else {
            let raw_lines = build_user_message_lines_with_elements(
                &message,
                text_elements,
                style,
                element_style,
            );
            let wrapped = adaptive_wrap_lines(
                raw_lines,
                RtOptions::new(usize::from(wrap_width))
                    .wrap_algorithm(textwrap::WrapAlgorithm::FirstFit),
            );
            let wrapped = trim_trailing_blank_lines(wrapped);
            (!wrapped.is_empty()).then_some(wrapped)
        };

        if wrapped_remote_images.is_none() && wrapped_message.is_none() {
            return Vec::new();
        }

        let mut lines: Vec<Line<'static>> = vec![Line::from("").style(style)];

        if let Some(wrapped_remote_images) = wrapped_remote_images {
            lines.extend(prefix_lines(
                wrapped_remote_images,
                "  ".into(),
                "  ".into(),
            ));
            if wrapped_message.is_some() {
                lines.push(Line::from("").style(style));
            }
        }

        if let Some(wrapped_message) = wrapped_message {
            lines.extend(prefix_lines(
                wrapped_message,
                "› ".bold().dim(),
                "  ".into(),
            ));
        }

        lines.push(Line::from("").style(style));
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        let message = sanitize_user_text(&self.message);
        let mut lines = raw_lines_from_source(message.trim_end_matches(['\r', '\n']));
        if !self.remote_image_urls.is_empty() {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.extend(
                self.remote_image_urls
                    .iter()
                    .enumerate()
                    .map(|(idx, _url)| Line::from(local_image_label_text(idx.saturating_add(1)))),
            );
        }
        lines
    }
}

#[derive(Debug)]
pub(crate) struct ReasoningSummaryCell {
    _header: String,
    content: String,
    /// 用于在推理正文中渲染本地文件链接的会话工作目录。
    cwd: PathBuf,
    transcript_only: bool,
}

impl ReasoningSummaryCell {
    /// 创建一个推理摘要单元，其本地文件链接将相对于记录该摘要时生效的会话工作目录进行渲染。
    pub(crate) fn new(header: String, content: String, cwd: &Path, transcript_only: bool) -> Self {
        Self {
            _header: header,
            content,
            cwd: cwd.to_path_buf(),
            transcript_only,
        }
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        append_markdown(
            &self.content,
            crate::tui_core::width::usable_content_width_u16(width, /*reserved_cols*/ 2),
            Some(self.cwd.as_path()),
            &mut lines,
        );
        let summary_style = Style::default().dim().italic();
        let summary_lines = lines
            .into_iter()
            .map(|mut line| {
                line.spans = line
                    .spans
                    .into_iter()
                    .map(|span| span.patch_style(summary_style))
                    .collect();
                line
            })
            .collect::<Vec<_>>();

        adaptive_wrap_lines(
            &summary_lines,
            RtOptions::new(width as usize)
                .initial_indent("• ".dim().into())
                .subsequent_indent("  ".into()),
        )
    }
}

impl HistoryCell for ReasoningSummaryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.transcript_only {
            Vec::new()
        } else {
            self.lines(width)
        }
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        if self.transcript_only {
            Vec::new()
        } else {
            raw_lines_from_source(self.content.trim())
        }
    }
}

#[derive(Debug)]
pub(crate) struct AgentMessageCell {
    lines: Vec<HyperlinkLine>,
    is_first_line: bool,
}

impl AgentMessageCell {
    #[cfg(test)]
    pub(crate) fn new(lines: Vec<Line<'static>>, is_first_line: bool) -> Self {
        Self {
            lines: plain_hyperlink_lines(lines),
            is_first_line,
        }
    }

    pub(crate) fn new_hyperlink_lines(lines: Vec<HyperlinkLine>, is_first_line: bool) -> Self {
        Self {
            lines,
            is_first_line,
        }
    }
}

impl HistoryCell for AgentMessageCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }

    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        let mut wrapped = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let initial_indent = if index == 0 && self.is_first_line {
                "• ".dim().into()
            } else {
                "  ".into()
            };
            let mut subsequent_indent = Line::from("  ");
            subsequent_indent.spans.extend(
                crate::tui_core::insert_history::leading_whitespace_prefix(&line.line).spans,
            );
            wrapped.extend(
                crate::tui_core::terminal_hyperlinks::adaptive_wrap_hyperlink_lines(
                    std::slice::from_ref(line),
                    RtOptions::new(width as usize)
                        .initial_indent(initial_indent)
                        .subsequent_indent(subsequent_indent),
                ),
            );
        }
        wrapped
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(visible_lines(self.lines.clone()))
    }

    fn is_stream_continuation(&self) -> bool {
        !self.is_first_line
    }
}

/// 一个整合的助手消息单元，存储原始 markdown 源并从中重新渲染。
///
/// 流结束后，`App` 中的 `ConsolidateAgentMessage` 处理器会将连续的 `AgentMessageCell` 序列
/// 替换为单个 `AgentMarkdownCell`。在终端调整大小时，`display_lines(width)` 通过
/// `append_markdown_agent` 从源重新渲染，从而生成尺寸正确的、带有盒绘边框的表格。
///
/// 该单元在构造时对 `cwd` 进行快照，以便本地文件链接的显示与生成该消息的会话保持一致。
/// 在重排时复用当前进程的工作目录，会使旧的转录内容在后续的 `/cd` 或恢复会话之后语义发生变化。
///
/// 普通 markdown 会缓存其最新的富文本渲染结果。可视化指令会绕过该缓存，
/// 因为解析这些本地文件链接依赖于稍后可能发生变化的文件系统状态。
#[derive(Debug)]
pub(crate) struct AgentMarkdownCell {
    markdown_source: String,
    cwd: PathBuf,
    inline_visualization_context:
        Option<crate::tui_core::inline_visualization::InlineVisualizationContext>,
    rendered_lines: Option<MarkdownRenderCache>,
}

impl AgentMarkdownCell {
    /// 创建一个由源支持的已完成助手消息单元。
    ///
    /// `markdown_source` 必须是流控制器累积的原始源，而不是已经包装好的终端行。
    /// 若传入已渲染的行，将导致后续调整大小时的重排保留过时的折行，而不是修复它。
    #[cfg(test)]
    pub(crate) fn new(markdown_source: String, cwd: &Path) -> Self {
        Self::new_with_inline_visualizations(
            markdown_source,
            cwd,
            /*inline_visualization_context*/ None,
        )
    }

    pub(crate) fn new_with_inline_visualizations(
        markdown_source: String,
        cwd: &Path,
        inline_visualization_context: Option<
            crate::tui_core::inline_visualization::InlineVisualizationContext,
        >,
    ) -> Self {
        let rendered_lines = (!markdown_source
            .contains(crate::tui_core::inline_visualization::DIRECTIVE_PREFIX))
        .then(MarkdownRenderCache::default);
        Self {
            markdown_source,
            cwd: cwd.to_path_buf(),
            inline_visualization_context,
            rendered_lines,
        }
    }
}

impl HistoryCell for AgentMarkdownCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }

    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        let render = || {
            let Some(wrap_width) =
                crate::tui_core::width::usable_content_width_u16(width, /*reserved_cols*/ 2)
            else {
                return prefix_hyperlink_lines(
                    vec![HyperlinkLine::new(Line::default())],
                    "• ".dim(),
                    "  ".into(),
                );
            };

            // 按当前宽度从源重新渲染 markdown。预留 2 列用于下方添加的 "• " / " " 前缀。
            let lines =
                crate::tui_core::markdown::render_markdown_agent_with_links_cwd_and_visualizations(
                    &self.markdown_source,
                    Some(wrap_width),
                    Some(self.cwd.as_path()),
                    self.inline_visualization_context.as_ref(),
                );
            prefix_hyperlink_lines(lines, "• ".dim(), "  ".into())
        };

        if let Some(rendered_lines) = &self.rendered_lines {
            rendered_lines.render(width, render)
        } else {
            render()
        }
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        raw_lines_from_source(&self.markdown_source)
    }

    fn has_stable_transcript_height(&self) -> bool {
        self.rendered_lines.is_some()
    }
}

#[cfg(test)]
#[path = "messages_tests.rs"]
mod tests;

/// 智能体流可变尾部的临时活动单元表示。
///
/// 在流式传输期间，由于尚未提交到回滚区（因为属于进行中的表格）的行，
/// 会通过该单元显示在 `active_cell` 槽中。当增量改变可见尾部时会替换它，
/// 而在流结束时清除。
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct StreamingAgentTailCell {
    lines: Vec<HyperlinkLine>,
    is_first_line: bool,
}

impl StreamingAgentTailCell {
    pub(crate) fn new(lines: Vec<HyperlinkLine>, is_first_line: bool) -> Self {
        Self {
            lines,
            is_first_line,
        }
    }
}

impl HistoryCell for StreamingAgentTailCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }

    fn display_hyperlink_lines(&self, _width: u16) -> Vec<HyperlinkLine> {
        // 尾部行已经按控制器当前的流宽度渲染完毕。
        // 在此处重新折行可能会打断表格边框，并产生格式错误的进行中行。
        let mut lines = prefix_hyperlink_lines(
            self.lines.clone(),
            if self.is_first_line {
                "• ".dim()
            } else {
                "  ".into()
            },
            "  ".into(),
        );
        for line in &mut lines {
            if line
                .line
                .spans
                .iter()
                .all(|span| span.content.chars().all(char::is_whitespace))
            {
                line.line = Line::default().style(line.line.style);
                line.hyperlinks.clear();
            }
        }
        lines
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.display_lines(/*width*/ u16::MAX))
    }

    fn is_stream_continuation(&self) -> bool {
        !self.is_first_line
    }
}
pub(crate) fn new_user_prompt(
    message: String,
    text_elements: Vec<TextElement>,
    local_image_paths: Vec<PathBuf>,
    remote_image_urls: Vec<String>,
) -> UserHistoryCell {
    UserHistoryCell {
        message,
        text_elements,
        local_image_paths,
        remote_image_urls,
    }
}
/// 在推理块末尾创建发出的推理历史单元。
///
/// 该辅助函数会将 `cwd` 快照到返回的单元中，以便本地文件链接按照轮次活跃时的方式渲染，
/// 即便渲染发生在其他应用状态推进之后也是如此。保留分块边界，使得独立空占位符可以
/// 被移除，而不会改变字面 HTML 注释或仅有加粗样式的摘要内容。
pub(crate) fn new_reasoning_summary_block(
    reasoning_parts: Vec<String>,
    cwd: &Path,
) -> Box<dyn HistoryCell> {
    let (header, content) = split_reasoning_summary_parts(&reasoning_parts);
    let transcript_only = header.is_empty();
    Box::new(ReasoningSummaryCell::new(
        header,
        content,
        cwd,
        transcript_only,
    ))
}

/// 将结构化的推理摘要部分拆分为状态头和可渲染的内容。
pub(crate) fn split_reasoning_summary_parts(reasoning_parts: &[String]) -> (String, String) {
    let mut leading_empty_part_header = None;
    let mut content_parts = Vec::with_capacity(reasoning_parts.len());

    for part in reasoning_parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let header_end = part.strip_prefix("**").and_then(|after_open| {
            after_open
                .find("**")
                .and_then(|close| (close > 0).then_some(close + 4))
        });
        let body = header_end.map_or(part, |header_end| &part[header_end..]);
        if body.trim() == "<!-- -->" {
            if content_parts.is_empty()
                && leading_empty_part_header.is_none()
                && let Some(header_end) = header_end
            {
                leading_empty_part_header = Some(part[..header_end].to_string());
            }
            continue;
        }

        content_parts.push(part);
    }

    let content = content_parts.join("\n\n");
    if content.is_empty() {
        return (leading_empty_part_header.unwrap_or_default(), content);
    }

    if let Some(after_open) = content.strip_prefix("**")
        && let Some(close) = after_open.find("**")
    {
        let after_close_idx = 2 + close + 2;
        let after_close = &content[after_close_idx..];
        if after_close.starts_with('\n') || after_close.starts_with('\r') {
            return (
                content[..after_close_idx].to_string(),
                after_close.to_string(),
            );
        }
    }

    (leading_empty_part_header.unwrap_or_default(), content)
}
