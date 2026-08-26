//! v1.x Tier 4.5:`/statusline` picker overlay widget。
//!
//! 居中覆盖层,复用 `plan_approval_block` 的布局。展示 HUD identity / metrics
//! 两段可配置项,允许启用/禁用(`Enter` toggle)。
//!
//! 渲染管线:`tui/mod.rs::draw` 调用 `statusline_picker::draw(frame, state, area)`。

use crate::events::{StatuslineItem, StatuslinePickerState, StatuslineSection, UiState};
use crate::tui_core::custom_terminal::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// `handle_key` 返回的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatuslinePickerAction {
    Close,
    Nop,
}

/// 处理按键。`Tab` 切段(Identity ↔ Metrics);`↑/k` `↓/j` 移动;
/// `Enter`/Space toggle 当前项 enabled;`Esc/q/Ctrl+C` 关闭。
pub fn handle_key(
    picker: &mut StatuslinePickerState,
    k: crossterm::event::KeyEvent,
) -> StatuslinePickerAction {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.kind == crossterm::event::KeyEventKind::Release {
        return StatuslinePickerAction::Nop;
    }
    if k.code == KeyCode::Esc
        || matches!(k.code, KeyCode::Char('q'))
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
    {
        return StatuslinePickerAction::Close;
    }

    let identity_len = picker.identity_items.len();
    let metrics_len = picker.metrics_items.len();

    // Tab:在两段之间切换,保持 selected_index 落在新段的合法范围。
    if matches!(k.code, KeyCode::Tab | KeyCode::BackTab) {
        match picker.selected_section {
            StatuslineSection::Identity => {
                picker.selected_section = StatuslineSection::Metrics;
                picker.selected_index = picker.selected_index.min(metrics_len.saturating_sub(1));
            }
            StatuslineSection::Metrics => {
                picker.selected_section = StatuslineSection::Identity;
                picker.selected_index = picker.selected_index.min(identity_len.saturating_sub(1));
            }
        }
        return StatuslinePickerAction::Nop;
    }

    // 当前段的项数与全局 selected_index(段内偏移 = selected_index - section_base)。
    let (section_len, section_base): (usize, usize) = match picker.selected_section {
        StatuslineSection::Identity => (identity_len, 0),
        StatuslineSection::Metrics => (metrics_len, identity_len),
    };
    let last = section_base + section_len.saturating_sub(1);
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if picker.selected_index > section_base {
                picker.selected_index -= 1;
            }
            StatuslinePickerAction::Nop
        }
        KeyCode::Down | KeyCode::Char('j') => {
            picker.selected_index = (picker.selected_index + 1).min(last);
            StatuslinePickerAction::Nop
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            let items = match picker.selected_section {
                StatuslineSection::Identity => &mut picker.identity_items,
                StatuslineSection::Metrics => &mut picker.metrics_items,
            };
            let local = picker.selected_index.saturating_sub(section_base);
            if let Some(item) = items.get_mut(local) {
                item.enabled = !item.enabled;
            }
            StatuslinePickerAction::Nop
        }
        _ => StatuslinePickerAction::Nop,
    }
}

/// 渲染 statusline picker(居中覆盖层)。
pub fn draw(frame: &mut Frame<'_>, state: &UiState, area: Rect) {
    let picker = match &state.statusline_picker {
        Some(p) => p,
        None => return,
    };
    let modal_area = centered_rect(65, 60, area);
    frame.render_widget_ref(Clear, modal_area);
    frame.render_widget_ref(build_paragraph(picker), modal_area);
}

fn build_paragraph(picker: &StatuslinePickerState) -> Paragraph<'static> {
    let title = Line::from(Span::styled(
        " 📊  Statusline ",
        Style::default()
            .fg(Color::Green)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
    ));

    let mut body: Vec<Line<'static>> = Vec::new();
    body.push(Line::from(""));
    body.push(Line::from(Span::styled(
        " Identity (top line)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    body.extend(render_items(
        &picker.identity_items,
        picker.selected_section == StatuslineSection::Identity,
        picker.selected_index,
    ));
    body.push(Line::from(""));
    body.push(Line::from(Span::styled(
        " Metrics (bottom line)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    body.extend(render_items(
        &picker.metrics_items,
        picker.selected_section == StatuslineSection::Metrics,
        picker.selected_index - picker.identity_items.len().min(picker.selected_index),
    ));

    let hint = Line::from(vec![
        Span::styled(
            " Tab ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("section · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "↑↓ ",
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
        Span::styled("toggle · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Esc ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("close", Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let mut all = vec![title];
    all.extend(body);
    all.push(Line::from(""));
    all.push(hint);
    Paragraph::new(all).block(block).wrap(Wrap { trim: false })
}

fn render_items(
    items: &[StatuslineItem],
    is_active_section: bool,
    active_index: usize,
) -> Vec<Line<'static>> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_sel = is_active_section && i == active_index;
            let marker = if is_sel { "▶" } else { " " };
            let color = if is_sel { Color::Yellow } else { Color::White };
            let check = if item.enabled { "☑" } else { "☐" };
            let check_color = if item.enabled {
                Color::Green
            } else {
                Color::DarkGray
            };
            Line::from(vec![
                Span::styled(format!(" {marker} "), Style::default().fg(color)),
                Span::styled(format!("{check} "), Style::default().fg(check_color)),
                Span::styled(item.name.clone(), Style::default().fg(color)),
            ])
        })
        .collect()
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let pop_w = area.width.saturating_mul(percent_x) / 100;
    let pop_h = area.height.saturating_mul(percent_y) / 100;
    let pop_w = pop_w.max(40).min(area.width);
    let pop_h = pop_h.max(10).min(area.height);
    let x = area.x + (area.width.saturating_sub(pop_w)) / 2;
    let y = area.y + (area.height.saturating_sub(pop_h)) / 2;
    Rect::new(x, y, pop_w, pop_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_items_marks_selected() {
        let items = vec![
            StatuslineItem {
                name: "model".into(),
                enabled: true,
            },
            StatuslineItem {
                name: "branch".into(),
                enabled: false,
            },
        ];
        let lines = render_items(&items, true, 1);
        assert_eq!(lines.len(), 2);
        // 第一行(未选中):启用项 ☑
        let text0: String = lines[0].spans.iter().map(|s| s.content.clone()).collect();
        assert!(text0.contains("☑"), "first enabled: {text0}");
        // 第二行(选中):禁用项 ☐,前缀 ▶
        let text1: String = lines[1].spans.iter().map(|s| s.content.clone()).collect();
        assert!(text1.starts_with(" ▶"), "selected: {text1}");
        assert!(text1.contains("☐"), "second disabled: {text1}");
    }

    fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn handle_key_tab_switches_section() {
        let mut p = StatuslinePickerState::default();
        assert_eq!(p.selected_section, StatuslineSection::Identity);
        handle_key(&mut p, key(crossterm::event::KeyCode::Tab));
        assert_eq!(p.selected_section, StatuslineSection::Metrics);
        // BackTab 也应切回(对称)。
        handle_key(&mut p, key(crossterm::event::KeyCode::BackTab));
        assert_eq!(p.selected_section, StatuslineSection::Identity);
    }

    #[test]
    fn handle_key_j_k_within_identity_section() {
        let mut p = StatuslinePickerState::default();
        // identity 段默认 5 项。
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(p.selected_index, 1);
        handle_key(&mut p, key(crossterm::event::KeyCode::Char('k')));
        assert_eq!(p.selected_index, 0);
    }

    #[test]
    fn handle_key_enter_toggles_selected_item() {
        let mut p = StatuslinePickerState::default();
        // identity[0] = "model" 默认已启用。
        assert!(p.identity_items[0].enabled);
        handle_key(&mut p, key(crossterm::event::KeyCode::Enter));
        assert!(!p.identity_items[0].enabled);
        handle_key(&mut p, key(crossterm::event::KeyCode::Enter));
        assert!(p.identity_items[0].enabled);
    }

    #[test]
    fn handle_key_space_also_toggles() {
        let mut p = StatuslinePickerState::default();
        let before = p.identity_items[0].enabled;
        handle_key(&mut p, key(crossterm::event::KeyCode::Char(' ')));
        assert_eq!(p.identity_items[0].enabled, !before);
    }

    #[test]
    fn handle_key_esc_closes() {
        let mut p = StatuslinePickerState::default();
        let act = handle_key(&mut p, key(crossterm::event::KeyCode::Esc));
        assert!(matches!(act, StatuslinePickerAction::Close));
    }

    #[test]
    fn handle_key_enter_toggles_in_metrics_section() {
        let mut p = StatuslinePickerState::default();
        // 切到 metrics 段,选中第一项(context-bar,默认已启用)。
        handle_key(&mut p, key(crossterm::event::KeyCode::Tab));
        p.selected_index = p.identity_items.len(); // metrics[0]
        assert!(p.metrics_items[0].enabled);
        handle_key(&mut p, key(crossterm::event::KeyCode::Enter));
        assert!(!p.metrics_items[0].enabled);
    }
}
