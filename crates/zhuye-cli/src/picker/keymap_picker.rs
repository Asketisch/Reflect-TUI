//! v1.x Tier 4.5:`/keymap` picker overlay widget。
//!
//! 居中覆盖层,复用 `plan_approval_block` 的布局。显示当前 `KeyAction` → 按键
//! 的可读映射,允许用户查看(未来:重绑定,本阶段仅展示)。
//!
//! 渲染管线:`tui/mod.rs::draw` 调用 `keymap_picker::draw(frame, state, area)`。

use crate::events::{KeymapBinding, KeymapPickerState, UiState};
use crate::keymap::KeyAction;
use crate::tui_core::custom_terminal::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// 默认 `KeyAction` → 按键 的可读映射(用于 picker 展示)。
/// 顺序即为 picker 列表的展示顺序。
pub fn default_bindings() -> Vec<KeymapBinding> {
    use KeyAction::*;
    vec![
        ("App", "Exit", "Ctrl+C", Exit),
        ("App", "OpenTranscript", "Ctrl+T", OpenTranscript),
        ("App", "CycleMode", "Shift+Tab", CycleMode),
        ("App", "CopyLastReply", "Ctrl+O", CopyLastReply),
        ("App", "OpenExternalEditor", "Ctrl+G", OpenExternalEditor),
        ("App", "ForceRedraw", "Ctrl+L", ForceRedraw),
        ("App", "OpenDiff", "Ctrl+Shift+D", OpenDiff),
    ]
    .into_iter()
    .map(|(ctx, action, key, _)| KeymapBinding {
        context: ctx.to_string(),
        action: action.to_string(),
        key: key.to_string(),
    })
    .collect()
}

/// 用 `default_bindings()` 填充一个空 picker。
pub fn fill_defaults(picker: &mut KeymapPickerState) {
    picker.bindings = default_bindings();
    picker.selected = 0;
}

/// `handle_key` 返回的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapPickerAction {
    Close,
    Nop,
}

/// 处理按键。`↑/k` `↓/j` 移动;`Esc/q/Ctrl+C` 关闭(rebind 未实现)。
pub fn handle_key(
    picker: &mut KeymapPickerState,
    k: crossterm::event::KeyEvent,
) -> KeymapPickerAction {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.kind == crossterm::event::KeyEventKind::Release {
        return KeymapPickerAction::Nop;
    }
    if k.code == KeyCode::Esc
        || matches!(k.code, KeyCode::Char('q'))
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
    {
        return KeymapPickerAction::Close;
    }
    let last = picker.bindings.len().saturating_sub(1);
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            picker.selected = picker.selected.saturating_sub(1);
            KeymapPickerAction::Nop
        }
        KeyCode::Down | KeyCode::Char('j') => {
            picker.selected = (picker.selected + 1).min(last);
            KeymapPickerAction::Nop
        }
        _ => KeymapPickerAction::Nop,
    }
}

/// 渲染 keymap picker(居中覆盖层)。
pub fn draw(frame: &mut Frame<'_>, state: &UiState, area: Rect) {
    let picker = match &state.keymap_picker {
        Some(p) => p,
        None => return,
    };
    let modal_area = centered_rect(70, 60, area);
    frame.render_widget_ref(Clear, modal_area);
    frame.render_widget_ref(build_paragraph(picker), modal_area);
}

fn build_paragraph(picker: &KeymapPickerState) -> Paragraph<'static> {
    let title = Line::from(Span::styled(
        " ⌨  Keymap ",
        Style::default()
            .fg(Color::Cyan)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
    ));

    let mut body: Vec<Line<'static>> = Vec::new();
    body.push(Line::from(Span::styled(
        "  Context  Action            Key",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )));
    body.push(Line::from(Span::styled(
        "  ───────  ──────────────    ──────────",
        Style::default().fg(Color::DarkGray),
    )));

    for (i, b) in picker.bindings.iter().enumerate() {
        let is_sel = i == picker.selected;
        let marker = if is_sel { "▶" } else { " " };
        let color = if is_sel { Color::Yellow } else { Color::White };
        body.push(Line::from(vec![
            Span::styled(format!(" {marker} "), Style::default().fg(color)),
            Span::styled(
                format!("{:<9}", b.context),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(format!("{:<18}", b.action), Style::default().fg(color)),
            Span::styled(b.key.clone(), Style::default().fg(Color::Green)),
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
            "Esc ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("close · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "(rebind not yet wired)",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let mut all = vec![title];
    all.extend(body);
    all.push(Line::from(""));
    all.push(hint);
    Paragraph::new(all).block(block).wrap(Wrap { trim: false })
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let pop_w = area.width.saturating_mul(percent_x) / 100;
    let pop_h = area.height.saturating_mul(percent_y) / 100;
    let pop_w = pop_w.max(40).min(area.width);
    let pop_h = pop_h.max(8).min(area.height);
    let x = area.x + (area.width.saturating_sub(pop_w)) / 2;
    let y = area.y + (area.height.saturating_sub(pop_h)) / 2;
    Rect::new(x, y, pop_w, pop_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bindings_includes_all_actions() {
        let bs = default_bindings();
        let actions: Vec<&str> = bs.iter().map(|b| b.action.as_str()).collect();
        for a in &[
            "Exit",
            "OpenTranscript",
            "CycleMode",
            "CopyLastReply",
            "OpenExternalEditor",
            "ForceRedraw",
            "OpenDiff",
        ] {
            assert!(actions.contains(a), "应含 {a}: {actions:?}");
        }
    }

    #[test]
    fn fill_defaults_replaces_bindings() {
        let mut p = KeymapPickerState::default();
        p.bindings.push(KeymapBinding {
            context: "X".into(),
            action: "Y".into(),
            key: "Z".into(),
        });
        fill_defaults(&mut p);
        assert_eq!(p.bindings.len(), 7);
        assert_eq!(p.selected, 0);
    }

    fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn handle_key_j_k_navigate() {
        let mut p = KeymapPickerState::default();
        fill_defaults(&mut p);
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(p.selected, 1);
        // 越界钳到最后一个。
        let last = p.bindings.len() - 1;
        p.selected = last;
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(p.selected, last);
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('k')));
        assert_eq!(p.selected, last - 1);
    }

    #[test]
    fn handle_key_esc_closes() {
        let mut p = KeymapPickerState::default();
        let act = handle_key(&mut p, key(crossterm::event::KeyCode::Esc));
        assert!(matches!(act, KeymapPickerAction::Close));
    }
}
