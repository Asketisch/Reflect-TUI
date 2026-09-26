//! `/plugin` overlay — 显示真实已安装插件列表。
//!
//! 数据源:`reflect_plugin::PluginManager::load(default_plugins_root())`
//! 读取 `installed_plugins.json`,enabled 标志来自 config
//! `[plugins].enabled_plugins`(TUI 当前不接 `[plugins]` 热重载,
//! config 的 enabled 集合与 bootstrap 挂载的运行时状态一致)。

use std::collections::HashSet;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

fn scope_str(scope: reflect_plugin::PluginScope) -> &'static str {
    match scope {
        reflect_plugin::PluginScope::Managed => "managed",
        reflect_plugin::PluginScope::User => "user",
        reflect_plugin::PluginScope::Project => "project",
        reflect_plugin::PluginScope::Local => "local",
    }
}

/// 生成插件 overlay 内容(供 TranscriptPager 使用)。
///
/// `enabled_plugins` 是 config `[plugins].enabled_plugins`(或空切片)。
pub fn load_plugins(enabled_plugins: &[String]) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(Line::from(Span::styled(
        "Installed Plugins",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    let enabled_set: HashSet<String> = enabled_plugins.iter().cloned().collect();

    let installed = reflect_plugin::default_plugins_root()
        .and_then(|root| reflect_plugin::PluginManager::load(root).ok())
        .map(|manager| manager.list_with_enabled(&enabled_set));

    let Some(installed) = installed else {
        lines.push(Line::from(Span::styled(
            "  plugins root unavailable (HOME 未设置或目录不存在)",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    };

    if installed.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No plugins installed.",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  安装:把插件目录放进 plugins root,并把插件 id 加入",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  ~/.reflect/config.toml 的 [plugins].enabled_plugins。",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(Span::styled(
            "  命令能力重新挂载需重启 TUI。",
            Style::default().fg(Color::DarkGray),
        )));
        return lines;
    }

    let enabled_count = installed.iter().filter(|(.., en)| *en).count();
    for (id, entry, is_enabled) in &installed {
        let status = if *is_enabled { "enabled" } else { "disabled" };
        let status_color = if *is_enabled {
            Color::Green
        } else {
            Color::Red
        };
        lines.push(Line::from(vec![
            Span::styled(
                if *is_enabled { "● " } else { "○ " },
                Style::default().fg(status_color),
            ),
            Span::styled(format!("{id} "), Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("v{} ", entry.version),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("[{}] ", scope_str(entry.scope)),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(status.to_string(), Style::default().fg(status_color)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("    {}", entry.install_path.display()),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "  {} plugins ({enabled_count} enabled) · Esc to close",
            installed.len()
        ),
        Style::default().fg(Color::DarkGray),
    )));

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_plugins_returns_header_lines() {
        let lines = load_plugins(&[]);
        assert!(!lines.is_empty());
        let first: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(first.contains("Installed Plugins"));
    }

    #[test]
    fn load_plugins_with_unknown_enabled_ids_still_renders() {
        // enabled 列表里的未知 id 不应导致 panic / 渲染失败;
        // 空环境(HOME 指向临时目录)下走 root unavailable 或空列表分支。
        let lines = load_plugins(&["nonexistent:plugin".to_string()]);
        assert!(!lines.is_empty());
    }
}
