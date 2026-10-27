//! `/checkpoint` overlay — git 文件级快照列表。
//!
//! 稳定 TUI 版简化了 git 操作路径——
//! 创建/回退通过 emit_op 触发,overlay 只负责列表展示与交互。

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// 一个 checkpoint 条目。
#[derive(Debug, Clone, Default)]
pub struct CheckpointEntry {
    /// Git sha(40 字符完整 SHA)。
    pub sha: String,
    /// 可选 label(创建时提供)。
    pub label: Option<String>,
    /// 创建时间(ISO 8601 或 "just now" / "5m ago" 等)。
    pub created_at: String,
}

/// checkpoint overlay 运行态。
#[derive(Debug, Clone, Default)]
pub struct CheckpointOverlayState {
    pub entries: Vec<CheckpointEntry>,
    pub selected: usize,
}

/// `handle_key` 返回的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointAction {
    Close,
    Create,
    Rewind(String),
    Copy(String),
    Refresh,
    Nop,
}

/// 渲染 checkpoint overlay(居中覆盖层)。由 `tui/mod.rs::draw` 在主布局之上调用。
pub fn draw(
    overlay: &CheckpointOverlayState,
    frame: &mut crate::tui_core::custom_terminal::Frame,
    area: Rect,
) {
    use ratatui::widgets::Clear;

    let total = overlay.entries.len();
    let title = if total == 0 {
        " Checkpoints (none yet) — c create · q/Esc close ".to_string()
    } else {
        format!(
            " Checkpoints [{}] — c create · r/Enter rewind · y copy sha · R refresh · q/Esc close ",
            total
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::DarkGray));

    let modal_area = centered_rect(80, 70, area);
    frame.render_widget_ref(Clear, modal_area);

    if overlay.entries.is_empty() {
        let body = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No checkpoints in this session yet.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  Press c to snapshot the workspace now (git auto-commit).",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  rewind restores files via git reset --hard.",
                Style::default().fg(Color::Yellow),
            )),
        ];
        let para = Paragraph::new(body).block(block);
        frame.render_widget_ref(para, modal_area);
        return;
    }

    let selected = overlay.selected.min(overlay.entries.len() - 1);

    // 构建列表项
    let items: Vec<Line> = overlay
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let short_sha: String = e.sha.chars().take(8).collect();
            let label_part: String = e
                .label
                .clone()
                .unwrap_or_else(|| format!("turn {}", short_turn(&e.sha)));
            let is_selected = i == selected;

            if is_selected {
                Line::from(vec![
                    Span::styled(
                        "▶ ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{short_sha} "),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{label_part} "), Style::default().fg(Color::White)),
                    Span::styled(&e.created_at, Style::default().fg(Color::DarkGray)),
                ])
            } else {
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(format!("{short_sha} "), Style::default().fg(Color::Cyan)),
                    Span::styled(format!("{label_part} "), Style::default().fg(Color::White)),
                    Span::styled(&e.created_at, Style::default().fg(Color::DarkGray)),
                ])
            }
        })
        .collect();

    let para = Paragraph::new(items).block(block);
    frame.render_widget_ref(para, modal_area);
}

/// 渲染 rewind 确认 modal(居中,红色警告)。
pub fn draw_confirm(
    target_sha: &str,
    frame: &mut crate::tui_core::custom_terminal::Frame,
    area: Rect,
) {
    use ratatui::widgets::Clear;

    let modal = centered_rect(60, 30, area);
    frame.render_widget_ref(Clear, modal);

    let short: String = target_sha.chars().take(8).collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Rewind workspace? ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Red));

    let body = vec![
        Line::from(vec![
            Span::raw("Restore files to checkpoint "),
            Span::styled(
                short.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ?"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "git reset --hard + clean -fd will DISCARD all uncommitted",
            Style::default().fg(Color::Red),
        )),
        Line::from(Span::styled(
            "changes and untracked files. This cannot be undone.",
            Style::default().fg(Color::Red),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "[Y]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" confirm rewind   "),
            Span::styled(
                "[N] / Esc",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" cancel"),
        ]),
    ];

    let para = Paragraph::new(body).block(block);
    frame.render_widget_ref(para, modal);
}

/// 处理按键。`q`/`Esc`/`Ctrl+C` → Close; `j`/`↓`/`k`/`↑` 移动;
/// `c` → Create; `r`/`Enter` → Rewind(选中行 sha); `y` → Copy(sha);
/// `R` → Refresh。
pub fn handle_key(
    overlay: &mut CheckpointOverlayState,
    k: crossterm::event::KeyEvent,
) -> CheckpointAction {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.kind == crossterm::event::KeyEventKind::Release {
        return CheckpointAction::Nop;
    }

    // 关闭
    let close = matches!(k.code, KeyCode::Char('q'))
        || k.code == KeyCode::Esc
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL));
    if close {
        return CheckpointAction::Close;
    }

    // 刷新
    if matches!(k.code, KeyCode::Char('R')) {
        return CheckpointAction::Refresh;
    }

    let last = overlay.entries.len().saturating_sub(1);
    match k.code {
        KeyCode::Down | KeyCode::Char('j') => {
            overlay.selected = (overlay.selected + 1).min(last);
            CheckpointAction::Nop
        }
        KeyCode::Up | KeyCode::Char('k') => {
            overlay.selected = overlay.selected.saturating_sub(1);
            CheckpointAction::Nop
        }
        KeyCode::Char('c') => CheckpointAction::Create,
        KeyCode::Char('r') | KeyCode::Enter => overlay
            .entries
            .get(overlay.selected)
            .map(|e| CheckpointAction::Rewind(e.sha.clone()))
            .unwrap_or(CheckpointAction::Nop),
        KeyCode::Char('y') => overlay
            .entries
            .get(overlay.selected)
            .map(|e| CheckpointAction::Copy(e.sha.clone()))
            .unwrap_or(CheckpointAction::Nop),
        _ => CheckpointAction::Nop,
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    use ratatui::layout::{Constraint, Direction, Layout};
    let popup = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup[1])[1]
}

/// 取 sha 的前 6 字符作为短形(作为 turn id 代理)。
fn short_turn(sha: &str) -> String {
    sha.chars().take(6).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn entry(sha: &str, label: Option<&str>) -> CheckpointEntry {
        CheckpointEntry {
            sha: sha.to_string(),
            label: label.map(|s| s.to_string()),
            created_at: "5m ago".to_string(),
        }
    }

    #[test]
    fn handle_key_c_creates() {
        let mut o = CheckpointOverlayState {
            entries: vec![entry("deadbeef", None)],
            selected: 0,
        };
        let act = handle_key(
            &mut o,
            crossterm::event::KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        );
        assert!(matches!(act, CheckpointAction::Create));
    }

    #[test]
    fn handle_key_r_rewinds_selected_sha() {
        let mut o = CheckpointOverlayState {
            entries: vec![entry("cafebabe", None)],
            selected: 0,
        };
        let act = handle_key(
            &mut o,
            crossterm::event::KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        );
        match act {
            CheckpointAction::Rewind(sha) => assert_eq!(sha, "cafebabe"),
            other => panic!("expected Rewind, got {other:?}"),
        }
    }

    #[test]
    fn handle_key_y_copies_selected_sha() {
        let mut o = CheckpointOverlayState {
            entries: vec![entry("12345678", None)],
            selected: 0,
        };
        let act = handle_key(
            &mut o,
            crossterm::event::KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        );
        match act {
            CheckpointAction::Copy(sha) => assert_eq!(sha, "12345678"),
            other => panic!("expected Copy, got {other:?}"),
        }
    }

    #[test]
    fn handle_key_uppercase_r_refreshes() {
        let mut o = CheckpointOverlayState::default();
        let act = handle_key(
            &mut o,
            crossterm::event::KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
        );
        assert!(matches!(act, CheckpointAction::Refresh));
    }

    #[test]
    fn handle_key_q_closes() {
        let mut o = CheckpointOverlayState::default();
        let act = handle_key(
            &mut o,
            crossterm::event::KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        );
        assert!(matches!(act, CheckpointAction::Close));
    }

    #[test]
    fn handle_key_j_moves_down_clamped() {
        let mut o = CheckpointOverlayState {
            entries: vec![entry("a", None), entry("b", None)],
            selected: 0,
        };
        handle_key(
            &mut o,
            crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(o.selected, 1);
        handle_key(
            &mut o,
            crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(o.selected, 1, "不应越界");
    }

    #[test]
    fn handle_key_enter_on_empty_is_nop() {
        let mut o = CheckpointOverlayState::default();
        let act = handle_key(
            &mut o,
            crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(act, CheckpointAction::Nop));
    }
}
