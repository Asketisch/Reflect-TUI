//! 针对进行中的转录流做增量 markdown 渲染。
//!
//! 已完成的顶层块会被保留，而最后一个块保持可变，从而避免在携带换行的增量到达时
//! 反复渲染稳定前缀。

use crate::tui_core::history_cell::HistoryRenderMode;
use crate::tui_core::history_cell::raw_lines_from_source;
use crate::tui_core::inline_visualization::DIRECTIVE_PREFIX;
use crate::tui_core::inline_visualization::InlineVisualizationContext;
use crate::tui_core::markdown::render_markdown_agent_with_links_cwd_and_visualizations;
use crate::tui_core::markdown::render_streaming_markdown_agent_with_links_and_cwd;
use crate::tui_core::terminal_hyperlinks::HyperlinkLine;
use crate::tui_core::terminal_hyperlinks::plain_hyperlink_lines;
use ratatui::text::Line;
use std::path::Path;

/// 在源码边界与渲染行边界处拆分的增量渲染状态。
///
/// 两个边界之前的前缀不可变；随着已提交源码的到来，仅重新渲染最后一个顶层 Markdown 块。
pub(super) struct StreamingRender {
    pub(super) lines: Vec<HyperlinkLine>,
    /// 仅包含已完成顶层 markdown 块的源码前缀。
    stable_source_len: usize,
    /// 与 `stable_source_len` 对应的渲染行边界。
    stable_rendered_len: usize,
    /// 参考式链接定义可能影响前后任意 markdown 块。
    has_reference_link_definition: bool,
    /// 一旦出现行内可视化指令被提交，就需要对源码做全局重写。
    has_inline_visualization_directive: bool,
}

impl StreamingRender {
    pub(super) fn new() -> Self {
        Self {
            lines: Vec::with_capacity(64),
            stable_source_len: 0,
            stable_rendered_len: 0,
            has_reference_link_definition: false,
            has_inline_visualization_directive: false,
        }
    }

    pub(super) fn clear(&mut self) {
        self.lines.clear();
        self.stable_source_len = 0;
        self.stable_rendered_len = 0;
        self.has_reference_link_definition = false;
        self.has_inline_visualization_directive = false;
    }

    /// 重新渲染完整源码，并重置两个稳定前缀边界。
    ///
    /// 在宽度或渲染模式变化时，以及当源码级渲染状态导致保留先前渲染的块不再安全时使用。
    pub(super) fn recompute(
        &mut self,
        source: &str,
        width: Option<usize>,
        cwd: &Path,
        render_mode: HistoryRenderMode,
        inline_visualization_context: Option<&InlineVisualizationContext>,
    ) {
        self.has_inline_visualization_directive = source.contains(DIRECTIVE_PREFIX);
        self.lines = match (render_mode, inline_visualization_context) {
            (HistoryRenderMode::Rich, None) if !self.has_inline_visualization_directive => {
                let rendered =
                    render_streaming_markdown_agent_with_links_and_cwd(source, width, Some(cwd));
                self.has_reference_link_definition = rendered.has_reference_link_definition;
                rendered.lines
            }
            _ => {
                self.has_reference_link_definition = false;
                render_source(
                    source,
                    width,
                    cwd,
                    render_mode,
                    inline_visualization_context,
                )
            }
        };
        self.stable_source_len = 0;
        self.stable_rendered_len = 0;
    }

    /// 追加新提交的源码，同时仅保留最后一个 markdown 块为可变状态。
    ///
    /// 当另一行到达时，最后一个顶层块的含义仍可能改变（例如列表的紧凑/松散、setext 标题、
    /// 围栏代码块或表格）。更早的顶层块只渲染一次并保留。参考式链接定义与行内可视化
    /// 重写会回退到完整渲染，因为它们可能影响源码级渲染状态。
    pub(super) fn append(
        &mut self,
        raw_source: &str,
        committed_source: &str,
        width: Option<usize>,
        cwd: &Path,
        render_mode: HistoryRenderMode,
        inline_visualization_context: Option<&InlineVisualizationContext>,
    ) {
        if render_mode == HistoryRenderMode::Raw {
            self.lines
                .extend(plain_hyperlink_lines(raw_lines_from_source(
                    committed_source,
                )));
            return;
        }

        self.has_inline_visualization_directive |= committed_source.contains(DIRECTIVE_PREFIX);
        if self.has_inline_visualization_directive {
            self.recompute(
                raw_source,
                width,
                cwd,
                render_mode,
                inline_visualization_context,
            );
            return;
        }

        if self.has_reference_link_definition {
            self.recompute(
                raw_source,
                width,
                cwd,
                render_mode,
                inline_visualization_context,
            );
            return;
        }

        let pending_source = &raw_source[self.stable_source_len..];
        let pending =
            render_streaming_markdown_agent_with_links_and_cwd(pending_source, width, Some(cwd));
        if pending.has_reference_link_definition {
            self.has_reference_link_definition = true;
            self.recompute(
                raw_source,
                width,
                cwd,
                render_mode,
                inline_visualization_context,
            );
            return;
        }

        let mut newly_stable_rendered_len = None;
        if let Some(boundary) = pending.last_top_level_block_start {
            let newly_stable_source = &pending_source[..boundary];
            let newly_stable = render_source(
                newly_stable_source,
                width,
                cwd,
                render_mode,
                inline_visualization_context,
            );
            self.stable_source_len += boundary;
            newly_stable_rendered_len = Some(newly_stable.len());
        }

        self.lines.truncate(self.stable_rendered_len);
        if !self.lines.is_empty()
            && (!pending.lines.is_empty() || !pending_source.trim().is_empty())
            && !pending.first_top_level_block_is_html
        {
            self.lines.push(HyperlinkLine::new(Line::default()));
        }
        let pending_render_start = self.lines.len();
        self.lines.extend(pending.lines);
        if let Some(newly_stable_rendered_len) = newly_stable_rendered_len {
            self.stable_rendered_len = pending_render_start + newly_stable_rendered_len;
        }
    }
}

pub(super) fn render_source(
    source: &str,
    width: Option<usize>,
    cwd: &Path,
    render_mode: HistoryRenderMode,
    inline_visualization_context: Option<&InlineVisualizationContext>,
) -> Vec<HyperlinkLine> {
    match render_mode {
        HistoryRenderMode::Rich => render_markdown_agent_with_links_cwd_and_visualizations(
            source,
            width,
            Some(cwd),
            inline_visualization_context,
        ),
        HistoryRenderMode::Raw => plain_hyperlink_lines(raw_lines_from_source(source)),
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
