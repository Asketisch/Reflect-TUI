pub fn ansi_escape_line(_text: &str) -> ratatui::prelude::Line<'static> {
    ratatui::prelude::Line::from(_text.to_string())
}
