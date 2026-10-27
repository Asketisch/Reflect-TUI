//! 渲染并格式化 unified-exec 后台会话摘要文本。
//!
//! 本模块提供唯一规范的摘要字符串，使底部窗格可以
//! 渲染专用页脚行，或在状态行中内联复用同一文本，
//! 而无需重复文案/语法逻辑。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::tui_core::live_wrap::take_prefix_by_width;
use crate::tui_core::render::renderable::Renderable;

/// 跟踪活跃的 unified-exec 进程并渲染紧凑摘要。
pub(crate) struct UnifiedExecFooter {
    processes: Vec<String>,
}

impl UnifiedExecFooter {
    pub(crate) fn new() -> Self {
        Self {
            processes: Vec::new(),
        }
    }

    pub(crate) fn set_processes(&mut self, processes: Vec<String>) -> bool {
        if self.processes == processes {
            return false;
        }
        self.processes = processes;
        true
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.processes.is_empty()
    }

    /// 返回页脚和状态行渲染共同使用的无缩进摘要文本。
    ///
    /// 中文说明：The returned string intentionally omits leading spaces and separators so
    /// 中文说明：callers can choose layout-specific framing (inline separator vs. row
    /// 中文说明：indentation). Returning `None` means there is nothing to surface.
    pub(crate) fn summary_text(&self) -> Option<String> {
        if self.processes.is_empty() {
            return None;
        }

        let count = self.processes.len();
        let plural = if count == 1 { "" } else { "s" };
        Some(format!(
            "{count} background terminal{plural} running · /ps to view · /stop to close"
        ))
    }

    fn render_lines(&self, width: u16) -> Vec<Line<'static>> {
        if width < 4 {
            return Vec::new();
        }
        let Some(summary) = self.summary_text() else {
            return Vec::new();
        };
        let message = format!("  {summary}");
        let (truncated, _, _) = take_prefix_by_width(&message, width as usize);
        vec![Line::from(truncated.dim())]
    }
}

impl Renderable for UnifiedExecFooter {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        Paragraph::new(self.render_lines(area.width)).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.render_lines(width).len() as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;

    #[test]
    fn desired_height_empty() {
        let footer = UnifiedExecFooter::new();
        assert_eq!(footer.desired_height(/*width*/ 40), 0);
    }

    #[test]
    fn render_more_sessions() {
        let mut footer = UnifiedExecFooter::new();
        footer.set_processes(vec!["rg \"foo\" src".to_string()]);
        let width = 50;
        let height = footer.desired_height(width);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        footer.render(Rect::new(0, 0, width, height), &mut buf);
        assert_snapshot!("render_more_sessions", format!("{buf:?}"));
    }

    #[test]
    fn render_many_sessions() {
        let mut footer = UnifiedExecFooter::new();
        footer.set_processes((0..123).map(|idx| format!("cmd {idx}")).collect());
        let width = 50;
        let height = footer.desired_height(width);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        footer.render(Rect::new(0, 0, width, height), &mut buf);
        assert_snapshot!("render_many_sessions", format!("{buf:?}"));
    }
}
