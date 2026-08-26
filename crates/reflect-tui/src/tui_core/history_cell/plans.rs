//! 拟议计划和计划更新历史单元。

use super::markdown_render_cache::MarkdownRenderCache;
use super::*;
use crate::tui_core::style::proposed_plan_style;

/// 拟议计划流可变尾部的临时活动单元表示。
///
/// 控制器负责准备完整的带样式计划行，因为计划尾部需要与已提交的
/// `ProposedPlanStreamCell` 使用相同的标题、内边距和背景处理，同时在流式传输期间
/// 仅作为预览。
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct StreamingPlanTailCell {
    lines: Vec<HyperlinkLine>,
    is_stream_continuation: bool,
}

impl StreamingPlanTailCell {
    pub(crate) fn new(lines: Vec<HyperlinkLine>, is_stream_continuation: bool) -> Self {
        Self {
            lines,
            is_stream_continuation,
        }
    }
}

impl HistoryCell for StreamingPlanTailCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        visible_lines(self.lines.clone())
    }

    fn display_hyperlink_lines(&self, _width: u16) -> Vec<HyperlinkLine> {
        self.lines.clone()
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(visible_lines(self.lines.clone()))
    }

    fn is_stream_continuation(&self) -> bool {
        self.is_stream_continuation
    }
}
/// 将计划更新渲染为用户友好的复选框待办列表样式。
pub(crate) fn new_plan_update(update: UpdatePlanArgs) -> PlanUpdateCell {
    let UpdatePlanArgs { explanation, plan } = update;
    PlanUpdateCell { explanation, plan }
}

/// 创建拟议计划单元，并保存会话 cwd 快照以供后续 markdown 渲染。
///
/// 计划正文以原始 markdown 存储，因此终端调整大小并重新排版时可按
/// 当前宽度重新渲染。调用方应仅对临时实时流式单元使用 `new_proposed_plan_stream`，
/// 并在计划完成后将其合并为此源数据支持的单元。
pub(crate) fn new_proposed_plan(plan_markdown: String, cwd: &Path) -> ProposedPlanCell {
    ProposedPlanCell {
        plan_markdown,
        cwd: cwd.to_path_buf(),
        rendered_lines: MarkdownRenderCache::default(),
    }
}

/// 从已渲染的行创建临时拟议计划流单元。
///
/// 流单元是显示片段，而不是源数据支持的历史记录。在依赖调整大小后的重新排版来处理最终历史记录之前，
/// 应在合并期间将其替换为 `ProposedPlanCell`。
pub(crate) fn new_proposed_plan_stream(
    lines: Vec<impl Into<HyperlinkLine>>,
    is_stream_continuation: bool,
) -> ProposedPlanStreamCell {
    ProposedPlanStreamCell {
        lines: lines.into_iter().map(Into::into).collect(),
        is_stream_continuation,
    }
}

/// 可按新宽度重新自我渲染的最终拟议计划历史记录。
///
/// 这是 `ProposedPlanStreamCell` 的源数据支持版本。它拥有原始 markdown 以及
/// 后续 transcript 重新排版时稳定渲染本地链接所需的会话 cwd。
#[derive(Debug)]
pub(crate) struct ProposedPlanCell {
    plan_markdown: String,
    /// 用于使本地文件链接显示与实时流式计划渲染保持一致的会话 cwd。
    cwd: PathBuf,
    rendered_lines: MarkdownRenderCache,
}

/// 计划仍在流式传输时发出的临时拟议计划历史记录。
///
/// 这些行已按流的当前宽度渲染。最终 transcript 不应在合并后保留这些单元，
/// 因为它们无法在后续终端调整大小时重新渲染其
/// 源数据。
#[derive(Debug)]
pub(crate) struct ProposedPlanStreamCell {
    lines: Vec<HyperlinkLine>,
    is_stream_continuation: bool,
}

impl HistoryCell for ProposedPlanCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }

    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.rendered_lines.render(width, || {
            let mut lines = vec![
                HyperlinkLine::new(vec!["• ".dim(), "Proposed Plan".bold()].into()),
                HyperlinkLine::new(Line::from(" ")),
            ];

            let mut plan_lines: Vec<HyperlinkLine> = Vec::new();
            let wrap_width = width.saturating_sub(4).max(1) as usize;
            let mut body = crate::tui_core::markdown::render_markdown_agent_with_links_and_cwd(
                &self.plan_markdown,
                Some(wrap_width),
                Some(self.cwd.as_path()),
            );
            if body.is_empty() {
                body.push(HyperlinkLine::new(Line::from("(empty)".dim().italic())));
            }
            plan_lines.extend(prefix_hyperlink_lines(body, "  ".into(), "  ".into()));
            plan_lines.push(HyperlinkLine::new(Line::from(" ")));

            // 为 plan body 应用淡背景色，让 plan 在 transcript 中作为一个可辨识的色块，
            // 方便快速定位 plan 位置。`proposed_plan_style()` 只设置 `bg`，行级背景与
            // span 级标题彩色叠加后二者均可生效（span 的 fg 覆盖行 fg，bg 保留）。
            // 与流式预览路径（`PlanStreamController::render_display_lines`）保持一致。
            let plan_style = proposed_plan_style();
            lines.extend(plan_lines.into_iter().map(|line| line.style(plan_style)));
            lines
        })
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        raw_lines_from_source(&self.plan_markdown)
    }
}

#[cfg(test)]
#[path = "plans_tests.rs"]
mod tests;

impl HistoryCell for ProposedPlanStreamCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        visible_lines(self.lines.clone())
    }

    fn display_hyperlink_lines(&self, _width: u16) -> Vec<HyperlinkLine> {
        self.lines.clone()
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(visible_lines(self.lines.clone()))
    }

    fn is_stream_continuation(&self) -> bool {
        self.is_stream_continuation
    }
}

#[derive(Debug)]
pub(crate) struct PlanUpdateCell {
    explanation: Option<String>,
    plan: Vec<PlanItemArg>,
}

impl HistoryCell for PlanUpdateCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let render_note = |text: &str| -> Vec<Line<'static>> {
            let wrap_width = width.saturating_sub(4).max(1) as usize;
            let note = Line::from(text.to_string().dim().italic());
            let wrapped = adaptive_wrap_line(&note, RtOptions::new(wrap_width));
            let mut out = Vec::new();
            push_owned_lines(&wrapped, &mut out);
            out
        };

        let render_step = |status: &StepStatus, text: &str| -> Vec<Line<'static>> {
            let (box_str, step_style) = match status {
                StepStatus::Completed => ("✔ ", Style::default().crossed_out().dim()),
                StepStatus::InProgress => ("□ ", Style::default().cyan().bold()),
                StepStatus::Pending => ("□ ", Style::default().dim()),
            };

            let opts = RtOptions::new(width.saturating_sub(4).max(1) as usize)
                .initial_indent(box_str.into())
                .subsequent_indent("  ".into());
            let step = Line::from(text.to_string().set_style(step_style));
            let wrapped = adaptive_wrap_line(&step, opts);
            let mut out = Vec::new();
            push_owned_lines(&wrapped, &mut out);
            out
        };

        let mut lines: Vec<Line<'static>> = vec![];
        lines.push(vec!["• ".dim(), "Updated Plan".bold()].into());

        let mut indented_lines = vec![];
        let note = self
            .explanation
            .as_ref()
            .map(|s| s.trim())
            .filter(|t| !t.is_empty());
        if let Some(expl) = note {
            indented_lines.extend(render_note(expl));
        };

        if self.plan.is_empty() {
            indented_lines.push(Line::from("(no steps provided)".dim().italic()));
        } else {
            for PlanItemArg { step, status } in self.plan.iter() {
                indented_lines.extend(render_step(status, step));
            }
        }
        lines.extend(prefix_lines(indented_lines, "  └ ".dim(), "    ".into()));

        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from("Updated Plan")];
        if let Some(explanation) = self
            .explanation
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            lines.extend(raw_lines_from_source(explanation));
        }
        if self.plan.is_empty() {
            lines.push(Line::from("(no steps provided)"));
        } else {
            for PlanItemArg { step, status } in &self.plan {
                lines.push(Line::from(format!("{status:?}: {step}")));
            }
        }
        lines
    }
}
