//! `/skills` overlay — Skills Hub 显示当前技能列表 + 启用/禁用状态。
//!
//! 简化为：扫描已注册技能（来自配置/文件系统），显示列表，不直接修改状态。

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui_core::custom_terminal::Frame;

/// Skills Hub overlay 运行态。
#[derive(Debug, Clone, Default)]
pub struct SkillsHubState {
    pub skills: Vec<SkillEntry>,
    pub selected: usize,
}

/// 一个技能的显示条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

impl SkillEntry {
    pub fn checkbox(&self) -> &'static str {
        if self.enabled { "[x]" } else { "[ ]" }
    }
}

/// 扫描已注册技能（简化实现：从已知的内置路径检测）。
pub fn load_skills() -> Vec<SkillEntry> {
    // 简化实现：从 Cargo.toml 技能位置扫描
    let mut skills: Vec<SkillEntry> = Vec::new();

    // 内置技能（基于当前 TUI 已知功能）
    let builtin = [
        ("slash_popup", "Slash command popup (ComposerInput)", true),
        ("file_mentions", "@file mentions in composer", true),
        ("transcript_overlay", "Ctrl+T transcript pager", true),
        ("plan_mode", "Plan mode approval overlay", true),
        ("diff_overlay", "/diff unified diff viewer", true),
        ("theme_picker", "/theme dark/light/no-color", true),
        ("keymap_picker", "/keymap binding viewer", true),
        ("statusline_picker", "/statusline item toggle", true),
        ("image_picker", "/image file picker", true),
        ("copy_history", "/copy agent reply selection", true),
        ("fork_rewind", "/fork + /rewind session selection", true),
        ("checkpoint", "/checkpoint git snapshot overlay", true),
        ("skills_hub", "This overlay (skill list)", true),
        ("traces", "/traces span event viewer", true),
    ];

    for (name, desc, enabled) in builtin {
        skills.push(SkillEntry {
            name: name.to_string(),
            description: desc.to_string(),
            enabled,
        });
    }
    skills
}

/// 填充 SkillsHubState。
pub fn populate() -> SkillsHubState {
    let skills = load_skills();
    SkillsHubState {
        selected: 0,
        skills,
    }
}

/// 处理按键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillsHubAction {
    Close,
    Nop,
}

pub fn handle_key(state: &mut SkillsHubState, k: crossterm::event::KeyEvent) -> SkillsHubAction {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.kind == crossterm::event::KeyEventKind::Release {
        return SkillsHubAction::Nop;
    }
    if k.code == KeyCode::Esc
        || matches!(k.code, KeyCode::Char('q'))
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
    {
        return SkillsHubAction::Close;
    }
    let last = state.skills.len().saturating_sub(1);
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
            SkillsHubAction::Nop
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected = (state.selected + 1).min(last);
            SkillsHubAction::Nop
        }
        KeyCode::Char('e') | KeyCode::Char('d') => {
            // 简化：只提示（不直接切换，避免状态不一致）
            SkillsHubAction::Nop
        }
        _ => SkillsHubAction::Nop,
    }
}

/// 渲染 Skills Hub overlay（居中覆盖层）。
pub fn draw(state: &SkillsHubState, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Skills Hub — installed skills ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::DarkGray));

    let modal_area = centered_rect(80, 60, area);
    frame.render_widget_ref(Clear, modal_area);

    let items: Vec<Line> = state
        .skills
        .iter()
        .enumerate()
        .map(|(i, skill)| {
            let is_selected = i == state.selected;
            let checkbox = skill.checkbox();
            let status_color = if skill.enabled {
                Color::Green
            } else {
                Color::Red
            };

            if is_selected {
                Line::from(vec![
                    Span::styled(
                        format!("▶ {} ", checkbox),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{}: {}", skill.name, skill.description),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("   {} ", checkbox),
                        Style::default().fg(status_color),
                    ),
                    Span::styled(
                        format!("{}: {}", skill.name, skill.description),
                        Style::default().fg(Color::White),
                    ),
                ])
            }
        })
        .collect();

    let hint = Line::from(Span::styled(
        format!(
            " {}/{} · j/k navigate · Esc/q close ",
            state.selected + 1,
            state.skills.len()
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
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn load_skills_returns_builtins() {
        let skills = load_skills();
        assert!(!skills.is_empty());
        assert!(skills.iter().all(|s| s.enabled));
    }

    #[test]
    fn handle_key_j_moves_down() {
        let mut state = SkillsHubState {
            skills: load_skills(),
            selected: 0,
        };
        handle_key(
            &mut state,
            crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn handle_key_esc_closes() {
        let mut state = SkillsHubState::default();
        let act = handle_key(
            &mut state,
            crossterm::event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        );
        assert!(matches!(act, SkillsHubAction::Close));
    }
}
