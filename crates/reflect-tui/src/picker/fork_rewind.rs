//! `/fork` / `/rewind` overlay — 从历史 user prompt 中选择 fork 点或回填输入。
//!
//! fork 模式:选中 prompt 后创建子会话;rewind 模式:回填 prompt 到 composer 并发送 Op::Rewind。

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// fork/rewind 选择器运行态。
#[derive(Debug, Clone, Default)]
pub struct ForkRewindState {
    /// user prompt 在 state.history 中的索引列表。
    pub user_prompt_indices: Vec<usize>,
    /// 当前选中位置。
    pub selected: usize,
    /// 模式:fork 或 rewind。
    pub mode: ForkRewindMode,
}

/// fork/rewind 模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForkRewindMode {
    #[default]
    Fork,
    Rewind,
}

impl ForkRewindMode {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Fork => " Fork session — select user prompt as fork point ",
            Self::Rewind => " Rewind — select user prompt to re-edit & re-send ",
        }
    }

    pub fn hint(&self) -> &'static str {
        match self {
            Self::Fork => " j/k navigate · Enter fork · Esc/q close ",
            Self::Rewind => " j/k navigate · Enter load · Esc/q close ",
        }
    }
}

/// 从 state.history 中提取 user prompt 索引并填充 ForkRewindState。
pub fn populate_from_history(
    history: &[crate::adapter::UiHistoryItem],
    mode: ForkRewindMode,
) -> Option<ForkRewindState> {
    let indices: Vec<usize> = history
        .iter()
        .enumerate()
        .filter_map(|(i, item)| matches!(item, crate::adapter::UiHistoryItem::User(_)).then_some(i))
        .collect();
    if indices.is_empty() {
        return None;
    }
    Some(ForkRewindState {
        selected: indices.len() - 1,
        user_prompt_indices: indices,
        mode,
    })
}

/// 处理按键。返回动作:
/// - `Close` 关浮层;
/// - `Confirm` Enter 确认(调用方执行 fork/rewind)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkRewindAction {
    Close,
    Confirm,
    Nop,
}

pub fn handle_key(state: &mut ForkRewindState, k: crossterm::event::KeyEvent) -> ForkRewindAction {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.kind == crossterm::event::KeyEventKind::Release {
        return ForkRewindAction::Nop;
    }
    // 关闭
    if k.code == KeyCode::Esc
        || matches!(k.code, KeyCode::Char('q'))
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
    {
        return ForkRewindAction::Close;
    }
    let last = state.user_prompt_indices.len().saturating_sub(1);
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
            ForkRewindAction::Nop
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected = (state.selected + 1).min(last);
            ForkRewindAction::Nop
        }
        KeyCode::Enter => ForkRewindAction::Confirm,
        _ => ForkRewindAction::Nop,
    }
}

/// 提取选中行对应的 user prompt 文本(若存在)。
pub fn get_selected_text(
    state: &ForkRewindState,
    history: &[crate::adapter::UiHistoryItem],
) -> Option<String> {
    let idx = state.user_prompt_indices.get(state.selected)?;
    match history.get(*idx) {
        Some(crate::adapter::UiHistoryItem::User(text)) => Some(text.clone()),
        _ => None,
    }
}

/// 取选中 user prompt 在 `user_prompt_turns` 中对应的 turn_id(供
/// `fork_with_history` 的 `up_to_turn_id` 截断)。
///
/// `user_prompt_turns` 与 history 中 `UiHistoryItem::User` 项同序(由
/// `apply_event` 在每个 `TurnStarted` 时追加)。`state.selected` 是 overlay
/// 列表里的序号,直接索引 `user_prompt_turns`。索引越界返回 `None`
/// (fallback 到全量 fork)。
pub fn get_selected_turn_id(
    state: &ForkRewindState,
    user_prompt_turns: &[reflect_protocol::TurnId],
) -> Option<reflect_protocol::TurnId> {
    user_prompt_turns.get(state.selected).copied()
}

/// 渲染 fork/rewind overlay(居中覆盖层)。
pub fn draw(
    state: &ForkRewindState,
    history: &[crate::adapter::UiHistoryItem],
    frame: &mut crate::tui_core::custom_terminal::Frame,
    area: Rect,
) {
    use ratatui::widgets::Clear;

    let title = format!("{}{}", " ", state.mode.title());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::DarkGray));

    let modal_area = centered_rect(80, 60, area);
    frame.render_widget_ref(Clear, modal_area);

    if state.user_prompt_indices.is_empty() {
        let body = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No user prompts in history.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                state.mode.hint(),
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let para = Paragraph::new(body).block(block);
        frame.render_widget_ref(para, modal_area);
        return;
    }

    let selected = state.selected.min(state.user_prompt_indices.len() - 1);
    let total = state.user_prompt_indices.len();

    // 构建列表项:每个 prompt 显示行号 + 预览前 60 字符
    let items: Vec<Line> = state
        .user_prompt_indices
        .iter()
        .enumerate()
        .map(|(i, &hist_idx)| {
            let text = match history.get(hist_idx) {
                Some(crate::adapter::UiHistoryItem::User(t)) => t.clone(),
                _ => String::new(),
            };
            // 按 char 截断预览,避免在多字节边界字节切片 panic。
            let preview: String = if text.chars().count() > 60 {
                let mut s: String = text.chars().take(57).collect();
                s.push('…');
                s
            } else {
                text
            };
            let is_selected = i == selected;
            let num = i + 1;

            if is_selected {
                Line::from(vec![
                    Span::styled(
                        format!("▶ {num:>3}. "),
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
                        format!("   {num:>3}. "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(preview, Style::default().fg(Color::White)),
                ])
            }
        })
        .collect();

    let hint = Line::from(Span::styled(
        format!(" {}/{total} ·{}", selected + 1, state.mode.hint()),
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
            UiHistoryItem::Agent("agent reply".into()),
            UiHistoryItem::User("second prompt".into()),
            UiHistoryItem::Notice("notification".into()),
            UiHistoryItem::User("third prompt".into()),
        ]
    }

    #[test]
    fn populate_from_history_finds_user_prompts() {
        let history = make_history();
        let state = populate_from_history(&history, ForkRewindMode::Fork).unwrap();
        assert_eq!(state.user_prompt_indices, vec![0, 2, 4]);
        assert_eq!(state.selected, 2); // 默认最后一条
    }

    #[test]
    fn populate_from_history_empty_when_no_users() {
        let history = vec![UiHistoryItem::Agent("reply".into())];
        assert!(populate_from_history(&history, ForkRewindMode::Rewind).is_none());
    }

    #[test]
    fn handle_key_j_moves_down() {
        let mut state = ForkRewindState {
            user_prompt_indices: vec![0, 2, 4],
            selected: 0,
            mode: ForkRewindMode::Fork,
        };
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn handle_key_k_moves_up() {
        let mut state = ForkRewindState {
            user_prompt_indices: vec![0, 2, 4],
            selected: 2,
            mode: ForkRewindMode::Rewind,
        };
        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE),
        );
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn handle_key_enter_confirms() {
        let mut state = ForkRewindState::default();
        let act = handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(matches!(act, ForkRewindAction::Confirm));
    }

    #[test]
    fn handle_key_esc_closes() {
        let mut state = ForkRewindState::default();
        let act = handle_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(act, ForkRewindAction::Close));
    }

    #[test]
    fn get_selected_text_returns_prompt() {
        let history = make_history();
        let state = populate_from_history(&history, ForkRewindMode::Fork).unwrap();
        // selected=2 → 索引 4 → "third prompt"
        let text = get_selected_text(&state, &history).unwrap();
        assert_eq!(text, "third prompt");
    }

    #[test]
    fn get_selected_text_none_for_invalid() {
        let state = ForkRewindState::default();
        assert!(get_selected_text(&state, &make_history()).is_none());
    }

    #[test]
    fn preview_does_not_panic_on_multibyte_boundary() {
        // 回归:旧的 &text[..60] 字节切片会在多字节(中文/emoji)边界 panic。
        // 这里构造一条长度>60 字节、且第 60 字节正好落在多字节序列中间的 user prompt。
        let emoji_prompt: String = "😀".repeat(40); // 每个 emoji 4 字节,160 字节
        let history = vec![UiHistoryItem::User(emoji_prompt.clone())];
        let state = populate_from_history(&history, ForkRewindMode::Rewind).unwrap();
        // draw 不 panic 即通过(用 VT100 backend 渲染)。
        let backend = crate::tui_core::test_backend::VT100Backend::new(100, 20);
        let mut terminal =
            crate::tui_core::custom_terminal::Terminal::with_options(backend).unwrap();
        terminal.set_viewport_area(ratatui::layout::Rect::new(0, 0, 100, 20));
        terminal
            .draw(|frame: &mut crate::tui_core::custom_terminal::Frame| {
                draw(&state, &history, frame, frame.area());
            })
            .unwrap();
    }
}
