use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

pub fn render_markdown(text: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    // 使用带显式前景色的 Span::styled 而非 Span::raw。
    // 复刻的 ratatui 在默认 fg/bg 与终端背景一致时,会把 Span::raw(Style::default())
    // 渲染为不可见文本。
    let style = Style::default().fg(Color::White);
    text.lines()
        .flat_map(|line| wrap_line(line, width))
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
}

fn wrap_line(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut rows = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        let next_width = current.width() + ch.to_string().width();
        if next_width > width && !current.is_empty() {
            rows.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_cjk_by_display_width() {
        let rows = render_markdown("你好世界", 4);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].to_string(), "你好");
        assert_eq!(rows[1].to_string(), "世界");
    }
}
