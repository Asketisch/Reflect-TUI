//! 简易 transcript overlay（对齐 Reflect Ctrl+T）。
//!
//! 进入备用屏后用滚动偏移渲染已提交历史行；不依赖完整 stub `Tui`/`Overlay`
//! 接线。滚轮/方向键翻页，Esc 或 Ctrl+T 关闭。

use crate::tui_core::custom_terminal::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

#[derive(Debug, Default)]
pub struct TranscriptPager {
    pub lines: Vec<Line<'static>>,
    pub scroll_offset: usize,
    pub title: String,
}

impl TranscriptPager {
    pub fn new(lines: Vec<Line<'static>>) -> Self {
        let scroll_offset = lines.len().saturating_sub(1);
        Self {
            lines,
            scroll_offset,
            title: "Transcript (Esc / Ctrl+T to close)".into(),
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize, page_height: usize) {
        let max = self.lines.len().saturating_sub(page_height.max(1));
        self.scroll_offset = (self.scroll_offset + n).min(max);
    }

    pub fn page_up(&mut self, page_height: usize) {
        self.scroll_up(page_height.max(1));
    }

    pub fn page_down(&mut self, page_height: usize) {
        self.scroll_down(page_height.max(1), page_height);
    }

    pub fn jump_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn jump_bottom(&mut self, page_height: usize) {
        let max = self.lines.len().saturating_sub(page_height.max(1));
        self.scroll_offset = max;
    }

    pub fn draw(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        if area.height < 2 {
            return;
        }
        let title_area = Rect::new(area.x, area.y, area.width, 1);
        let body = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );
        frame.render_widget_ref(
            Paragraph::new(Line::from(Span::styled(
                self.title.as_str(),
                Style::default().fg(Color::Cyan),
            ))),
            title_area,
        );
        let page_h = body.height as usize;
        let start = self.scroll_offset.min(self.lines.len());
        let end = (start + page_h).min(self.lines.len());
        let slice: Vec<Line<'static>> = self.lines[start..end].to_vec();
        frame.render_widget_ref(Paragraph::new(slice).wrap(Wrap { trim: false }), body);
    }
}
