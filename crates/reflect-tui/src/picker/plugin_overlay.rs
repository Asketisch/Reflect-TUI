//! `/plugin` overlay — 显示已启用插件列表。
//!
//! 简化为：显示当前已识别的插件/功能模块列表。

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// 生成插件 overlay 内容（供 TranscriptPager 使用）。
pub fn load_plugins() -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        "Installed Plugins",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // 当前 TUI 中集成的功能模块（等效插件）
    let plugins: [(&str, &str, bool); 9] = [
        ("core_tui", "Reflect TUI core", true),
        ("slash_commands", "Slash command parser & popup", true),
        ("streaming_render", "Live markdown streaming", true),
        ("plan_approval", "Plan mode approval overlay", true),
        ("diff_viewer", "Unified diff overlay", true),
        ("transcript", "Full transcript pager (Ctrl+T)", true),
        ("clipboard", "Copy to clipboard (OSC52/native)", true),
        ("notifications", "Terminal notifications (OSC9/BEL)", true),
        ("external_editor", "External editor (Ctrl+G)", true),
    ];

    for (name, desc, enabled) in plugins {
        let status = if enabled { "enabled" } else { "disabled" };
        let status_color = if enabled { Color::Green } else { Color::Red };
        lines.push(Line::from(vec![
            Span::styled(format!("• {} ", name), Style::default().fg(Color::Yellow)),
            Span::styled(desc.to_string(), Style::default().fg(Color::White)),
            Span::styled(format!(" ({})", status), Style::default().fg(status_color)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {} plugins · Esc to close", plugins.len()),
        Style::default().fg(Color::DarkGray),
    )));

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_plugins_returns_lines() {
        let lines = load_plugins();
        assert!(!lines.is_empty());
        let first: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(first.contains("Installed Plugins"));
    }
}
