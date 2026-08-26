//! `/copy` overlay — 列出最近 agent 回复，选中后复制到剪贴板。
//!
//! 列表是 agent reply 的快照(打开 overlay 时拷贝)，用户 ↑/↓ 选 + Enter 复制。

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::adapter::UiHistoryItem;

/// copy-history overlay 运行态(打开 overlay 时的历史快照)。
#[derive(Debug, Clone, Default)]
pub struct CopyHistoryState {
    /// agent 回复快照(最新 → 最老, 因为从 history 末尾取)。
    pub entries: Vec<String>,
    /// 当前选中位置(0 = 最新, len-1 = 最老)。
    pub selected: usize,
}

/// 处理按键。返回动作:
/// - `Close` 关浮层;
/// - `Copy` Enter 确认(调用方执行复制)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyHistoryAction {
    Close,
    Copy,
    Nop,
}

pub fn handle_key(
    state: &mut CopyHistoryState,
    k: crossterm::event::KeyEvent,
) -> CopyHistoryAction {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.kind == crossterm::event::KeyEventKind::Release {
        return CopyHistoryAction::Nop;
    }
    // 关闭
    if k.code == KeyCode::Esc
        || matches!(k.code, KeyCode::Char('q'))
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
    {
        return CopyHistoryAction::Close;
    }
    let last = state.entries.len().saturating_sub(1);
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
            CopyHistoryAction::Nop
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected = (state.selected + 1).min(last);
            CopyHistoryAction::Nop
        }
        KeyCode::Enter => CopyHistoryAction::Copy,
        _ => CopyHistoryAction::Nop,
    }
}

/// 从 state.history 中收集 agent replies 填充 CopyHistoryState(最新在前)。
pub fn populate_from_history(history: &[UiHistoryItem]) -> Option<CopyHistoryState> {
    let entries: Vec<String> = history
        .iter()
        .rev()
        .filter_map(|item| match item {
            UiHistoryItem::Agent(t) => Some(t.clone()),
            _ => None,
        })
        .take(32) // MAX_AGENT_COPY_HISTORY
        .collect();
    if entries.is_empty() {
        return None;
    }
    Some(CopyHistoryState {
        selected: 0, // 默认最新
        entries,
    })
}

/// 渲染 copy history overlay(居中覆盖层)。
pub fn draw(
    state: &CopyHistoryState,
    frame: &mut crate::tui_core::custom_terminal::Frame,
    area: Rect,
) {
    use ratatui::widgets::Clear;

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Copy agent reply — select and Enter to copy ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::DarkGray));

    let modal_area = centered_rect(80, 50, area);
    frame.render_widget_ref(Clear, modal_area);

    if state.entries.is_empty() {
        let body = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No agent replies to copy.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let para = Paragraph::new(body).block(block);
        frame.render_widget_ref(para, modal_area);
        return;
    }

    let selected = state.selected.min(state.entries.len() - 1);
    let total = state.entries.len();

    let items: Vec<Line> = state
        .entries
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let preview = if text.chars().count() > 70 {
                let mut s: String = text.chars().take(67).collect();
                s.push_str("…");
                s
            } else {
                text.clone()
            };
            let is_selected = i == selected;
            let num = i + 1;

            if is_selected {
                Line::from(vec![
                    Span::styled(
                        format!("▶ {num:>2}. "),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        preview,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("   {num:>2}. "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(preview, Style::default().fg(Color::White)),
                ])
            }
        })
        .collect();

    let hint = Line::from(Span::styled(
        format!(
            " {}/{total} · latest=1 · j/k navigate · Enter copy · Esc/q close ",
            selected + 1
        ),
        Style::default().fg(Color::DarkGray),
    ));

    let body: Vec<Line> = items
        .into_iter()
        .chain(std::iter::once(Line::from("")))
        .chain(std::iter::once(hint))
        .collect();

    let para = Paragraph::new(body).block(block);
    frame.render_widget_ref(para, modal_area);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::UiHistoryItem;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn make_history() -> Vec<UiHistoryItem> {
        vec![
            UiHistoryItem::User("first prompt".into()),
            UiHistoryItem::Agent("agent reply 1".into()),
            UiHistoryItem::User("second prompt".into()),
            UiHistoryItem::Agent("agent reply 2".into()),
            UiHistoryItem::Agent("agent reply 3".into()),
        ]
    }

    #[test]
    fn populate_from_history_collects_agent_replies() {
        let history = make_history();
        let state = populate_from_history(&history).unwrap();
        // rev 顺序,最新在前: [reply 3, reply 2, reply 1]
        assert_eq!(state.entries.len(), 3);
        assert_eq!(state.entries[0], "agent reply 3");
        assert_eq!(state.entries[1], "agent reply 2");
        assert_eq!(state.entries[2], "agent reply 1");
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn populate_from_history_empty_when_no_agents() {
        let history = vec![UiHistoryItem::User("hello".into())];
        assert!(populate_from_history(&history).is_none());
    }

    #[test]
    fn populate_from_history_limits_to_32() {
        let mut history = Vec::new();
        for i in 0..50 {
            history.push(UiHistoryItem::Agent(format!("reply {i}")));
        }
        let state = populate_from_history(&history).unwrap();
        assert_eq!(state.entries.len(), 32);
        assert_eq!(state.entries[0], "reply 49"); // 最新
    }

    #[test]
    fn handle_key_j_moves_down() {
        let mut state = CopyHistoryState {
            entries: vec!["a".into(), "b".into(), "c".into()],
            selected: 0,
        };
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn handle_key_k_moves_up() {
        let mut state = CopyHistoryState {
            entries: vec!["a".into(), "b".into(), "c".into()],
            selected: 2,
        };
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn handle_key_enter_confirms() {
        let mut state = CopyHistoryState::default();
        let act = handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(act, CopyHistoryAction::Copy));
    }

    #[test]
    fn handle_key_esc_closes() {
        let mut state = CopyHistoryState::default();
        let act = handle_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(act, CopyHistoryAction::Close));
    }
}
