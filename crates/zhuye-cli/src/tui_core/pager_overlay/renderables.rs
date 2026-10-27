//! pager overlay 渲染适配器。从 pager_overlay.rs 抽出。

use super::*;

/// 缓存自身期望高度的可渲染对象。
pub(super) struct CachedRenderable {
    renderable: Box<dyn Renderable>,
    height: std::cell::Cell<Option<u16>>,
    last_width: std::cell::Cell<Option<u16>>,
}

impl CachedRenderable {
    pub(super) fn new(renderable: impl Into<Box<dyn Renderable>>) -> Self {
        Self {
            renderable: renderable.into(),
            height: std::cell::Cell::new(None),
            last_width: std::cell::Cell::new(None),
        }
    }
}

impl Renderable for CachedRenderable {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.renderable.render(area, buf);
    }
    fn desired_height(&self, width: u16) -> u16 {
        if self.last_width.get() != Some(width) {
            let height = self.renderable.desired_height(width);
            self.height.set(Some(height));
            self.last_width.set(Some(width));
        }
        self.height.get().unwrap_or(0)
    }
}

pub(super) struct CellRenderable {
    pub(super) cell: Arc<dyn HistoryCell>,
    pub(super) highlighted: bool,
}

impl Renderable for CellRenderable {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let hyperlink_lines = self.cell.transcript_hyperlink_lines(area.width);
        let style = if self.cell.as_any().is::<UserHistoryCell>() {
            if self.highlighted {
                user_message_style().reversed()
            } else {
                user_message_style()
            }
        } else {
            Style::default()
        };
        let p = Paragraph::new(Text::from(visible_lines_ref(&hyperlink_lines)))
            .style(style)
            .wrap(Wrap { trim: false });
        p.render(area, buf);
        mark_buffer_hyperlinks(buf, area, &hyperlink_lines, /*scroll_rows*/ 0);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.cell.desired_transcript_height(width)
    }
}

pub(super) struct HyperlinkLinesRenderable {
    pub(super) lines: Vec<HyperlinkLine>,
}

impl Renderable for HyperlinkLinesRenderable {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(Text::from(visible_lines_ref(&self.lines)))
            .wrap(Wrap { trim: false })
            .render(area, buf);
        mark_buffer_hyperlinks(buf, area, &self.lines, /*scroll_rows*/ 0);
    }

    fn desired_height(&self, width: u16) -> u16 {
        Paragraph::new(Text::from(visible_lines_ref(&self.lines)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(/*default*/ 0)
    }
}
