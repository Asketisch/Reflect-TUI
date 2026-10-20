//! `/mcp` overlay — 显示 MCP 服务器连接态。
//!
//! 数据来自 `state.mcp_servers`:由 bootstrap 连接管理器发出的
//! `McpServerStarted` / `McpServerFailed` 生命周期事件 upsert 而成。
//! **不在此处编造服务器列表** —— 没连上就是没连上,overlay 如实说「无」。

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::adapter::McpServerView;

/// 由连接态快照生成 MCP overlay 内容(供 TranscriptPager 使用)。
///
/// 快照为空(尚未收到任何生命周期事件)时明确告知「未连接任何 MCP 服务器」,
/// 而不是显示一份虚构的服务器清单。
pub fn load_mcp_status(servers: &[McpServerView]) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        "MCP Server Status",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    if servers.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No MCP servers connected.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  Configure servers in .mcp.json / reflect config, then /mcp again.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  0 servers · Esc to close",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    for s in servers {
        let (status, color) = if s.connected {
            ("connected", Color::Green)
        } else if s.will_retry {
            ("retrying", Color::Yellow)
        } else {
            ("failed", Color::Red)
        };
        let mut row = vec![
            Span::styled(format!("{} ", status), Style::default().fg(color)),
            Span::styled(s.name.clone(), Style::default().fg(Color::White)),
        ];
        if s.connected {
            row.push(Span::styled(
                format!(" ({} tools)", s.tool_names.len()),
                Style::default().fg(Color::DarkGray),
            ));
        } else if let Some(err) = &s.error {
            row.push(Span::styled(
                format!(" — {err}"),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(row));

        // 连接成功的 server 展开工具清单(协议事件本来就带 tool_names)。
        for tool in &s.tool_names {
            lines.push(Line::from(Span::styled(
                format!("    · {tool}"),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines.push(Line::from(""));
    let connected = servers.iter().filter(|s| s.connected).count();
    let tools: usize = servers
        .iter()
        .filter(|s| s.connected)
        .map(|s| s.tool_names.len())
        .sum();
    lines.push(Line::from(Span::styled(
        format!(
            "  {connected}/{} servers · {tools} tools · Esc to close",
            servers.len()
        ),
        Style::default().fg(Color::DarkGray),
    )));

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected(name: &str, tools: &[&str]) -> McpServerView {
        McpServerView {
            name: name.into(),
            connected: true,
            tool_names: tools.iter().map(|s| (*s).to_string()).collect(),
            error: None,
            will_retry: false,
        }
    }

    fn failed(name: &str, err: &str, will_retry: bool) -> McpServerView {
        McpServerView {
            name: name.into(),
            connected: false,
            tool_names: Vec::new(),
            error: Some(err.into()),
            will_retry,
        }
    }

    fn render(servers: &[McpServerView]) -> String {
        load_mcp_status(servers)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn load_mcp_status_returns_lines() {
        let lines = load_mcp_status(&[]);
        assert!(!lines.is_empty());
        let first: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(first.contains("MCP Server Status"));
    }

    #[test]
    fn empty_snapshot_reports_no_servers_not_fake_data() {
        // 回归:旧实现无条件编造一份 ["node_repl", ..., "connected", 3] 列表。
        let text = render(&[]);
        assert!(text.contains("No MCP servers connected"));
        assert!(!text.contains("node_repl"), "不得出现虚构服务器");
    }

    #[test]
    fn connected_server_lists_tools() {
        let text = render(&[connected("ctx7", &["ctx7__search", "ctx7__fetch"])]);
        assert!(text.contains("connected ctx7 (2 tools)"));
        assert!(text.contains("ctx7__search"));
        assert!(text.contains("ctx7__fetch"));
        assert!(text.contains("1/1 servers · 2 tools"));
    }

    #[test]
    fn failed_server_shows_error_and_retry_state() {
        let text = render(&[failed("broken", "spawn failed: ENOENT", false)]);
        assert!(text.contains("failed broken — spawn failed: ENOENT"));
        assert!(text.contains("0/1 servers · 0 tools"));

        let retrying = render(&[failed("http-srv", "connection refused", true)]);
        assert!(retrying.contains("retrying http-srv"));
    }

    #[test]
    fn upsert_replaces_same_named_server() {
        // 重连成功必须清掉此前的 failed 状态。
        let mut list = vec![failed("srv", "boom", false)];
        McpServerView::upsert(&mut list, connected("srv", &["srv__t"]));
        assert_eq!(list.len(), 1);
        assert!(list[0].connected);
        assert_eq!(list[0].tool_names, vec!["srv__t".to_string()]);
    }
}
