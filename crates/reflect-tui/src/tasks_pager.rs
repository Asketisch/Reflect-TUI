//! `/tasks` overlay:alt-screen 双栏视图。
//!
//! inline-viewport + 原生 scrollback 模型下无法做主屏常驻侧栏(终端 scrollback
//! 是整宽字符网格,TUI 无像素所有权)。故任务/agent 面板做成 alt-screen overlay
//! (镜像 `/diff`/`/transcript`):左 message_list(从 history 渲染),右 task/agent
//! 面板(plan checkbox + agent 列表)。Esc / Ctrl+T 关闭。
//!
//! overlay 模式下 ratatui 拿到全屏 buffer,可自由 `Layout::Horizontal` 分栏,
//! 零渲染障碍、不触碰 viewport/scrollback 核心管道。

use crate::events::{AgentEntry, PlanStep, UiState};
use crate::tui_core::custom_terminal::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

#[derive(Debug, Default)]
pub struct TasksPager {
    pub scroll_offset: usize,
}

impl TasksPager {
    pub fn page_up(&mut self, page: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(page);
    }
    pub fn page_down(&mut self, page: usize, max: usize) {
        self.scroll_offset = (self.scroll_offset + page).min(max);
    }
    pub fn jump_top(&mut self) {
        self.scroll_offset = 0;
    }
    pub fn jump_bottom(&mut self, max: usize) {
        self.scroll_offset = max;
    }

    /// 渲染双栏:左 message_list,右 task/agent 面板。
    pub fn draw(&self, frame: &mut Frame<'_>, state: &UiState, width: u16) {
        let area = frame.area();
        if area.height < 3 {
            return;
        }
        // 标题行。session_overlay 模式显示 session 标题,否则显示 task/agent 标题。
        let title = if state.session_overlay.is_some() {
            "Sessions (Esc / Ctrl+T to close · /resume /fork /rename)"
        } else {
            "Tasks & Agents (Esc / Ctrl+T to close · /tasks)"
        };
        let title_area = Rect::new(area.x, area.y, area.width, 1);
        frame.render_widget_ref(
            Paragraph::new(Line::from(Span::styled(
                title,
                Style::default().fg(Color::Cyan),
            ))),
            title_area,
        );
        let body = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );

        // 宽屏才分栏;窄屏只画右栏(task/agent 是本 overlay 的主体)。
        let show_left = area.width >= 60;
        let split = if show_left {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(body)
        } else {
            // 只用右半(整宽)。
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(100)])
                .split(body)
        };

        if show_left {
            self.draw_message_list(frame, split[0], state, width);
            self.draw_side_panel(frame, split[1], state);
        } else {
            self.draw_side_panel(frame, split[0], state);
        }
    }

    fn draw_message_list(&self, frame: &mut Frame<'_>, area: Rect, state: &UiState, width: u16) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for item in &state.history {
            lines.extend(crate::history_render::history_item_to_lines(
                item, width, &state.cwd,
            ));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "(empty transcript)",
                Style::default().fg(Color::DarkGray),
            )));
        }
        let page_h = area.height as usize;
        let max = lines.len().saturating_sub(page_h);
        let start = self.scroll_offset.min(max);
        let end = (start + page_h).min(lines.len());
        let slice: Vec<Line<'static>> = lines[start..end].to_vec();
        frame.render_widget_ref(
            Paragraph::new(slice).wrap(Wrap { trim: false }).block(
                Block::default().borders(Borders::TOP).title(Span::styled(
                    "Transcript",
                    Style::default().fg(Color::DarkGray),
                )),
            ),
            area,
        );
    }

    fn draw_side_panel(&self, frame: &mut Frame<'_>, area: Rect, state: &UiState) {
        // v1.x Tier 4.5:session_overlay 模式 → 渲染 session list(优先于 plan/agent)。
        if state.session_overlay.is_some() {
            self.draw_session_panel(frame, area, state);
            return;
        }
        let mut lines: Vec<Line<'static>> = Vec::new();

        // 计划步骤。
        lines.push(Line::from(Span::styled(
            "Plan",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        if state.plan_steps.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no plan steps)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for s in &state.plan_steps {
                lines.push(plan_step_line(s));
            }
        }

        lines.push(Line::from(""));
        // Agents。
        lines.push(Line::from(Span::styled(
            "Agents",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        if state.agents.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no agents)",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for a in &state.agents {
                lines.push(agent_line(a));
            }
        }

        frame.render_widget_ref(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::TOP)
                    .title(Span::styled("Tasks", Style::default().fg(Color::DarkGray))),
            ),
            area,
        );
    }

    /// v1.x Tier 4.5:session_overlay 模式的右栏:列出 session + 操作 hint。
    fn draw_session_panel(&self, frame: &mut Frame<'_>, area: Rect, state: &UiState) {
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(Span::styled(
            "Sessions",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        if state.session_entries.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no saved sessions)",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  /new         start new session",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "  /archive     archive current",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "  /delete      delete current",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "  /fork [n]    fork current",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(Span::styled(
                "  /rename [n]  rename current",
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            for s in &state.session_entries {
                let mark = s.checkbox();
                let color = if s.is_current {
                    Color::Yellow
                } else {
                    Color::White
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{mark} "),
                        Style::default().fg(if s.is_current {
                            Color::Green
                        } else {
                            Color::DarkGray
                        }),
                    ),
                    Span::styled(
                        s.name.clone(),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  ({})", s.last_active),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                lines.push(Line::from(Span::styled(
                    format!("    id: {}", s.id),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        frame.render_widget_ref(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                Block::default().borders(Borders::TOP).title(Span::styled(
                    "Session",
                    Style::default().fg(Color::DarkGray),
                )),
            ),
            area,
        );
    }
}

fn plan_step_line(s: &PlanStep) -> Line<'static> {
    let (color, glyph) = match s.status {
        crate::events::PlanStepStatus::Done => (Color::Green, s.status.checkbox()),
        crate::events::PlanStepStatus::Skipped => (Color::DarkGray, s.status.checkbox()),
        crate::events::PlanStepStatus::InProgress => (Color::Yellow, s.status.checkbox()),
        crate::events::PlanStepStatus::Pending => (Color::White, s.status.checkbox()),
    };
    Line::from(vec![
        Span::styled(format!("{glyph} "), Style::default().fg(color)),
        Span::raw(s.title.clone()),
    ])
}

fn agent_line(a: &AgentEntry) -> Line<'static> {
    let color = match a.status {
        crate::events::AgentStatus::Running => Color::Yellow,
        crate::events::AgentStatus::Done => Color::Green,
        crate::events::AgentStatus::Idle => Color::DarkGray,
    };
    Line::from(vec![
        Span::styled(format!("{} ", a.status.glyph()), Style::default().fg(color)),
        Span::raw(a.label.clone()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkbox_glyphs() {
        use crate::events::PlanStepStatus::*;
        assert_eq!(Pending.checkbox(), "[ ]");
        assert_eq!(InProgress.checkbox(), "[~]");
        assert_eq!(Done.checkbox(), "[x]");
        assert_eq!(Skipped.checkbox(), "[-]");
    }

    #[test]
    fn agent_glyphs() {
        use crate::events::AgentStatus::*;
        assert_eq!(Idle.glyph(), "○");
        assert_eq!(Running.glyph(), "◐");
        assert_eq!(Done.glyph(), "●");
    }

    #[test]
    fn plan_step_line_shows_title() {
        let s = PlanStep {
            index: 0,
            status: crate::events::PlanStepStatus::Done,
            title: "write tests".into(),
        };
        let l = plan_step_line(&s);
        let text: String = l.spans.iter().map(|s| s.content.clone()).collect();
        assert!(text.contains("[x]"), "checkbox: {text}");
        assert!(text.contains("write tests"), "title: {text}");
    }
}
