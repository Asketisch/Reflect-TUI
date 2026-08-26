//! `/mcp` overlay — 显示 MCP 服务器状态。
//!
//! 简化为：显示 MCP 服务器列表和工具计数。

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// 生成 MCP overlay 内容（供 TranscriptPager 使用）。
pub fn load_mcp_status() -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        "MCP Server Status",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // 当前 TUI 中集成的 MCP 服务器
    let mcp_servers = [("node_repl", "Node.js REPL environment", "connected", 3u16)];

    for (name, desc, status, tool_count) in mcp_servers {
        let color = match status {
            "connected" => Color::Green,
            "disconnected" => Color::Red,
            _ => Color::Yellow,
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", status), Style::default().fg(color)),
            Span::styled(
                format!("{} - {} ({} tools)", name, desc, tool_count),
                Style::default().fg(Color::White),
            ),
        ]));
    }

    if mcp_servers.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no MCP servers configured)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {} servers · Esc to close", mcp_servers.len()),
        Style::default().fg(Color::DarkGray),
    )));

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_mcp_status_returns_lines() {
        let lines = load_mcp_status();
        assert!(!lines.is_empty());
        let first: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(first.contains("MCP Server Status"));
    }
}
