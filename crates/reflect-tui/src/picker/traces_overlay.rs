//! `/traces` overlay — 显示最近 trace span 事件（简化实现）。
//!
//! 简化为：扫描 `~/.reflect/traces/log/` 最新 JSONL 文件，提取 span 标题，
//! 用 `TranscriptPager` 渲染。

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::path::PathBuf;

/// 生成 traces overlay 内容（返回 `Vec<Line>` 供 TranscriptPager 使用）。
pub fn load_recent_traces() -> Vec<Line<'static>> {
    let trace_dir = traces_dir();
    if !trace_dir.is_dir() {
        return vec![
            Line::from(Span::styled(
                "Traces directory not found.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(format!("Expected: {}", trace_dir.display())),
        ];
    }

    // 扫描最新的 JSONL 文件
    let mut log_files: Vec<PathBuf> = std::fs::read_dir(&trace_dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd)
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "jsonl"))
        .map(|e| e.path())
        .collect();

    log_files.sort();
    log_files.reverse();

    let Some(latest) = log_files.first() else {
        return vec![Line::from(Span::styled(
            "No trace files found.",
            Style::default().fg(Color::DarkGray),
        ))];
    };

    let Ok(body) = std::fs::read_to_string(latest) else {
        return vec![Line::from(format!(
            "Cannot read trace file: {}",
            latest.display()
        ))];
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    let file_name = latest
        .file_name()
        .map_or("unknown".to_string(), |n| n.to_string_lossy().into_owned());
    lines.push(Line::from(Span::styled(
        format!("Traces: {}", file_name),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    let mut span_count = 0usize;
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            // 提取 span 的基本信息
            let op = val
                .get("op")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>");
            let dur = val
                .get("duration_ms")
                .and_then(|v| v.as_u64())
                .map(|d| format!("{}ms", d))
                .unwrap_or("-".to_string());

            lines.push(Line::from(vec![
                Span::styled(format!("▶ {} ", op), Style::default().fg(Color::Yellow)),
                Span::styled(format!("({})", dur), Style::default().fg(Color::DarkGray)),
            ]));
            span_count += 1;
        }
    }

    if span_count == 0 {
        lines.push(Line::from(Span::styled(
            "(no spans found in this trace)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  {} spans · Esc to close", span_count),
        Style::default().fg(Color::DarkGray),
    )));

    lines
}

fn traces_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".reflect").join("traces").join("log"))
        .unwrap_or_else(|| PathBuf::from("/tmp/reflect_traces"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_recent_traces_returns_lines_when_dir_missing() {
        let lines = load_recent_traces();
        assert!(!lines.is_empty());
    }
}
