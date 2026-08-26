//! v1.x Tier 4.5:`/theme` picker overlay widget。
//!
//! 居中覆盖层,复用 `plan_approval_block` 的布局。展示 dark / light / no-color
//! 三个主题,选中后回写到 `state.permission_mode`(只用于 picker 自身;真正的
//! 主题应用留待后续把 `palette` 接到 `tui_core::style`)。

use crate::events::{ThemePickerState, UiState};
use crate::tui_core::custom_terminal::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// 渲染 theme picker(居中覆盖层)。
pub fn draw(frame: &mut Frame<'_>, state: &UiState, area: Rect) {
    let picker = match &state.theme_picker {
        Some(p) => p,
        None => return,
    };
    let modal_area = centered_rect(50, 40, area);
    frame.render_widget_ref(Clear, modal_area);
    frame.render_widget_ref(build_paragraph(picker), modal_area);
}

fn build_paragraph(picker: &ThemePickerState) -> Paragraph<'static> {
    let title = Line::from(Span::styled(
        " 🎨  Theme ",
        Style::default()
            .fg(Color::Magenta)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
    ));

    let mut body: Vec<Line<'static>> = Vec::new();
    body.push(Line::from(Span::styled(
        format!(" Current: {}", picker.current),
        Style::default().fg(Color::DarkGray),
    )));
    body.push(Line::from(""));

    for (i, t) in picker.themes.iter().enumerate() {
        let is_sel = i == picker.selected;
        let marker = if is_sel { "▶" } else { " " };
        let color = if is_sel { Color::Yellow } else { Color::White };
        body.push(Line::from(vec![
            Span::styled(format!(" {marker} "), Style::default().fg(color)),
            Span::styled(t.label.clone(), Style::default().fg(color)),
        ]));
    }

    let hint = Line::from(vec![
        Span::styled(
            " ↑↓ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("navigate · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Enter ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("apply · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Esc ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("close", Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let mut all = vec![title];
    all.extend(body);
    all.push(Line::from(""));
    all.push(hint);
    Paragraph::new(all).block(block).wrap(Wrap { trim: false })
}

/// `handle_key` 返回的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemePickerAction {
    Close,
    /// Enter 应用(已回写 `picker.current`,但调色板未真正切换)。
    Apply,
    Nop,
}

/// 处理按键。`↑/k` `↓/j` 移动;`Enter` 应用(回写 `current`);`Esc/q/Ctrl+C` 关闭。
pub fn handle_key(
    picker: &mut ThemePickerState,
    k: crossterm::event::KeyEvent,
) -> ThemePickerAction {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.kind == crossterm::event::KeyEventKind::Release {
        return ThemePickerAction::Nop;
    }
    if k.code == KeyCode::Esc
        || matches!(k.code, KeyCode::Char('q'))
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
    {
        return ThemePickerAction::Close;
    }
    let last = picker.themes.len().saturating_sub(1);
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            picker.selected = picker.selected.saturating_sub(1);
            ThemePickerAction::Nop
        }
        KeyCode::Down | KeyCode::Char('j') => {
            picker.selected = (picker.selected + 1).min(last);
            ThemePickerAction::Nop
        }
        KeyCode::Enter => {
            if let Some(t) = picker.themes.get(picker.selected) {
                picker.current = t.id.clone();
            }
            ThemePickerAction::Apply
        }
        _ => ThemePickerAction::Nop,
    }
}

/// 应用 picker 的选择（仅回写 `state.theme_picker.current`，不重启 TUI 调色板——
///// 真正的 theme 切换留待后续把 `palette` 接到 `tui_core::style`）。
pub fn apply(state: &mut UiState) {
    // 取出选中的 id 后再 mut borrow state。
    let selected_id = state
        .theme_picker
        .as_ref()
        .and_then(|p| p.themes.get(p.selected))
        .map(|t| t.id.clone());
    if let (Some(id), Some(p)) = (selected_id, state.theme_picker.as_mut()) {
        p.current = id.clone();
        state
            .history
            .push(crate::adapter::UiHistoryItem::Notice(format!(
                "/theme: switched to '{id}' (display only — palette apply not yet wired)."
            )));
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let pop_w = area.width.saturating_mul(percent_x) / 100;
    let pop_h = area.height.saturating_mul(percent_y) / 100;
    let pop_w = pop_w.max(30).min(area.width);
    let pop_h = pop_h.max(8).min(area.height);
    let x = area.x + (area.width.saturating_sub(pop_w)) / 2;
    let y = area.y + (area.height.saturating_sub(pop_h)) / 2;
    Rect::new(x, y, pop_w, pop_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_picker_has_three_themes() {
        let p = ThemePickerState::default();
        assert_eq!(p.themes.len(), 3);
        assert_eq!(p.themes[0].id, "dark");
        assert_eq!(p.themes[1].id, "light");
        assert_eq!(p.themes[2].id, "no-color");
    }

    #[test]
    fn apply_sets_current_to_selected() {
        let mut s = UiState::default();
        let mut p = ThemePickerState::default();
        p.selected = 1; // light
        s.theme_picker = Some(p);
        apply(&mut s);
        let p = s.theme_picker.as_ref().unwrap();
        assert_eq!(p.current, "light");
        // 推了一条 Notice
        assert!(
            s.history.iter().any(
                |h| matches!(h, crate::adapter::UiHistoryItem::Notice(n) if n.contains("light"))
            )
        );
    }

    fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn handle_key_j_moves_down_clamped() {
        let mut p = ThemePickerState::default();
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(p.selected, 1);
        // 越界钳到最后一个。
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('j')));
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(p.selected, 2);
    }

    #[test]
    fn handle_key_k_moves_up_clamped() {
        let mut p = ThemePickerState::default();
        p.selected = 2;
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('k')));
        assert_eq!(p.selected, 1);
        // 不会下溢。
        p.selected = 0;
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('k')));
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn handle_key_esc_closes() {
        let mut p = ThemePickerState::default();
        let act = handle_key(&mut p, key(crossterm::event::KeyCode::Esc));
        assert!(matches!(act, ThemePickerAction::Close));
    }

    #[test]
    fn handle_key_enter_applies_current() {
        let mut p = ThemePickerState::default();
        p.selected = 2; // no-color
        let act = handle_key(&mut p, key(crossterm::event::KeyCode::Enter));
        assert!(matches!(act, ThemePickerAction::Apply));
        assert_eq!(p.current, "no-color");
    }
}
