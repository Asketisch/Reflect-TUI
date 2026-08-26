use crate::app_server_protocol::HookEventName;
use crate::app_server_protocol::HookMetadata;
use crate::app_server_protocol::HookSource;
use crate::app_server_protocol::HookTrustStatus;
use crate::app_server_protocol::HooksListEntry;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Styled;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

use super::CancellationEvent;
use super::bottom_pane_view::BottomPaneView;
use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::render_menu_surface;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::hooks_rpc::HookTrustUpdate;
use crate::tui_core::hooks_rpc::hook_needs_review;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::keymap::ListKeymap;
use crate::tui_core::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::status::format_directory_display;
use crate::tui_core::style::accent_style;

const EVENT_COLUMN_WIDTH: usize = 22;
const COUNT_COLUMN_WIDTH: usize = 12;
const MAX_COMMAND_DETAIL_LINES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HooksBrowserPage {
    Events,
    Handlers(HookEventName),
}

pub(crate) struct HooksBrowserView {
    entry: HooksListEntry,
    page: HooksBrowserPage,
    state: ScrollState,
    complete: bool,
    app_event_tx: AppEventSender,
    keymap: ListKeymap,
}

impl HooksBrowserView {
    #[cfg(test)]
    pub(crate) fn new(
        hooks: Vec<HookMetadata>,
        warnings: Vec<String>,
        errors: Vec<crate::app_server_protocol::HookErrorInfo>,
        app_event_tx: AppEventSender,
    ) -> Self {
        Self::from_entry(
            HooksListEntry {
                cwd: std::path::PathBuf::new(),
                hooks,
                warnings,
                errors,
            },
            app_event_tx,
            crate::tui_core::keymap::RuntimeKeymap::defaults().list,
        )
    }

    pub(crate) fn from_entry(
        mut entry: HooksListEntry,
        app_event_tx: AppEventSender,
        keymap: ListKeymap,
    ) -> Self {
        entry.hooks.sort_by_key(|hook| hook.display_order);
        let mut view = Self {
            entry,
            page: HooksBrowserPage::Events,
            state: ScrollState::new(),
            complete: false,
            app_event_tx,
            keymap,
        };
        if view.page_len() > 0 {
            view.state.selected_idx = Some(
                view.event_rows()
                    .iter()
                    .position(|row| row.needs_review > 0)
                    .unwrap_or(0),
            );
        }
        view
    }

    fn event_rows(&self) -> Vec<EventRow> {
        HookEventName::iter()
            .map(|event_name| {
                let event_name: HookEventName = event_name.into();
                let installed = self
                    .entry
                    .hooks
                    .iter()
                    .filter(|hook| hook.event_name == event_name)
                    .count();
                let active = self
                    .entry
                    .hooks
                    .iter()
                    .filter(|hook| hook.event_name == event_name && hook_is_active(hook))
                    .count();
                let needs_review = self
                    .entry
                    .hooks
                    .iter()
                    .filter(|hook| hook.event_name == event_name && hook_needs_review(hook))
                    .count();
                EventRow {
                    event_name,
                    installed,
                    active,
                    needs_review,
                }
            })
            .collect()
    }

    fn handlers_for_event(&self, event_name: HookEventName) -> impl Iterator<Item = &HookMetadata> {
        self.entry
            .hooks
            .iter()
            .filter(move |hook| hook.event_name == event_name)
    }

    fn selected_event(&self) -> Option<HookEventName> {
        self.state
            .selected_idx
            .and_then(|idx| HookEventName::iter().nth(idx))
            .map(Into::into)
    }

    fn selected_hook_index(&self, event_name: HookEventName) -> Option<usize> {
        let selected_visible_idx = self.state.selected_idx?;
        self.entry
            .hooks
            .iter()
            .enumerate()
            .filter(|(_, hook)| hook.event_name == event_name)
            .nth(selected_visible_idx)
            .map(|(idx, _)| idx)
    }

    fn selected_hook(&self, event_name: HookEventName) -> Option<&HookMetadata> {
        self.selected_hook_index(event_name)
            .and_then(|idx| self.entry.hooks.get(idx))
    }

    fn move_up(&mut self) {
        let len = self.page_len();
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, self.max_visible_rows());
    }

    fn move_down(&mut self) {
        let len = self.page_len();
        self.state.move_down_wrap(len);
        self.state.ensure_visible(len, self.max_visible_rows());
    }

    fn page_up(&mut self) {
        let len = self.page_len();
        self.state.page_up_clamped(len, self.max_visible_rows());
    }

    fn page_down(&mut self) {
        let len = self.page_len();
        self.state.page_down_clamped(len, self.max_visible_rows());
    }

    fn jump_top(&mut self) {
        let len = self.page_len();
        self.state.jump_top(len, self.max_visible_rows());
    }

    fn jump_bottom(&mut self) {
        let len = self.page_len();
        self.state.jump_bottom(len, self.max_visible_rows());
    }

    fn page_len(&self) -> usize {
        match self.page {
            HooksBrowserPage::Events => HookEventName::iter().count(),
            HooksBrowserPage::Handlers(event_name) => self.handlers_for_event(event_name).count(),
        }
    }

    fn max_visible_rows(&self) -> usize {
        MAX_POPUP_ROWS.min(self.page_len().max(1))
    }

    fn open_selected_event(&mut self) {
        let Some(event_name) = self.selected_event() else {
            return;
        };
        self.page = HooksBrowserPage::Handlers(event_name);
        self.state = ScrollState::new();
        if self.page_len() > 0 {
            self.state.selected_idx = Some(0);
        }
    }

    fn toggle_selected_hook(&mut self, event_name: HookEventName) {
        let Some(idx) = self.selected_hook_index(event_name) else {
            return;
        };
        let Some(hook) = self.entry.hooks.get_mut(idx) else {
            return;
        };
        if hook.is_managed {
            return;
        }
        if hook_needs_review(hook) {
            return;
        }

        hook.enabled = !hook.enabled;
        self.app_event_tx.send(AppEvent::SetHookEnabled {
            key: hook.key.clone(),
            enabled: hook.enabled,
        });
    }

    fn trust_selected_hook(&mut self, event_name: HookEventName) {
        let Some(idx) = self.selected_hook_index(event_name) else {
            return;
        };
        let Some(hook) = self.entry.hooks.get_mut(idx) else {
            return;
        };
        if !hook_needs_review(hook) {
            return;
        }

        hook.trust_status = HookTrustStatus::Trusted;
        self.app_event_tx.send(AppEvent::TrustHook {
            key: hook.key.clone(),
            current_hash: hook.current_hash.clone(),
        });
    }

    fn trust_all_hooks(&mut self) {
        let mut updates = Vec::new();
        for hook in &mut self.entry.hooks {
            if !hook_needs_review(hook) {
                continue;
            }

            hook.trust_status = HookTrustStatus::Trusted;
            updates.push(HookTrustUpdate {
                key: hook.key.clone(),
                current_hash: hook.current_hash.clone(),
            });
        }
        if !updates.is_empty() {
            self.app_event_tx.send(AppEvent::TrustHooks { updates });
        }
    }

    fn close(&mut self) {
        self.complete = true;
    }

    fn return_to_events(&mut self) {
        let selected_event_name = match self.page {
            HooksBrowserPage::Events => None,
            HooksBrowserPage::Handlers(event_name) => Some(event_name),
        };
        self.page = HooksBrowserPage::Events;
        self.state = ScrollState::new();
        self.state.selected_idx = selected_event_name
            .and_then(|event_name| {
                HookEventName::iter()
                    .position(|candidate| HookEventName::from(candidate) == event_name)
            })
            .or_else(|| (self.page_len() > 0).then_some(0));
    }

    fn event_header_lines() -> Vec<Line<'static>> {
        vec![
            "Hooks".bold().into(),
            "Lifecycle hooks from config and enabled plugins."
                .dim()
                .into(),
        ]
    }

    fn review_needed_total_count(&self) -> usize {
        self.entry
            .hooks
            .iter()
            .filter(|hook| hook_needs_review(hook))
            .count()
    }

    #[allow(clippy::disallowed_methods)]
    fn handler_header_lines(
        event_name: HookEventName,
        review_needed_count: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = vec![format!("{} hooks", event_label(event_name)).bold().into()];
        match review_needed_message(review_needed_count) {
            None => lines.push(
                "Turn hooks on or off. Your changes are saved automatically."
                    .dim()
                    .into(),
            ),
            Some(message) => lines.push(message.yellow().into()),
        }
        lines
    }

    fn review_needed_count(&self, event_name: HookEventName) -> usize {
        self.handlers_for_event(event_name)
            .filter(|hook| hook_needs_review(hook))
            .count()
    }

    #[allow(clippy::disallowed_methods)]
    fn event_table_lines(&self) -> Vec<Line<'static>> {
        let rows = self.event_rows();
        let show_review = rows.iter().any(|row| row.needs_review > 0);
        let mut lines = Vec::new();
        let mut header = vec![
            format!("{:<EVENT_COLUMN_WIDTH$}", "Event").into(),
            format!("{:<COUNT_COLUMN_WIDTH$}", "Installed").into(),
            format!("{:<COUNT_COLUMN_WIDTH$}", "Active").into(),
        ];
        if show_review {
            header.push(format!("{:<COUNT_COLUMN_WIDTH$}", "Review").into());
        }
        header.push("Description".into());
        lines.push(Line::from(header));
        for (idx, row) in rows.into_iter().enumerate() {
            let selected = self.state.selected_idx == Some(idx);
            let needs_review = row.needs_review > 0;
            let mut row_line = vec![
                Span::from(format!(
                    "{:<EVENT_COLUMN_WIDTH$}",
                    event_label(row.event_name)
                )),
                Span::from(format!("{:<COUNT_COLUMN_WIDTH$}", row.installed)),
                Span::from(format!("{:<COUNT_COLUMN_WIDTH$}", row.active)),
            ];
            if show_review {
                let review_count = Span::from(format!("{:<COUNT_COLUMN_WIDTH$}", row.needs_review));
                row_line.push(if needs_review {
                    review_count.yellow()
                } else {
                    review_count
                });
            }
            row_line.push(Span::from(event_description(row.event_name)));

            if selected {
                let style = accent_style();
                for span in &mut row_line {
                    *span = span.clone().set_style(style);
                }
            } else {
                row_line[1] = row_line[1].clone().dim();
                row_line[2] = row_line[2].clone().dim();
                if show_review && !needs_review {
                    row_line[3] = row_line[3].clone().dim();
                }
                let description_idx = row_line.len() - 1;
                row_line[description_idx] = row_line[description_idx].clone().dim();
            }
            lines.push(Line::from(row_line));
        }
        lines
    }

    fn event_issue_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if self.entry.warnings.is_empty() && self.entry.errors.is_empty() {
            return lines;
        }

        lines.push("Issues".bold().into());
        lines.extend(
            self.entry
                .warnings
                .iter()
                .map(|warning| format!("⚠ {warning}").into()),
        );
        lines.extend(self.entry.errors.iter().map(|error| {
            format!("■ {}: {}", error.path.display(), error.message)
                .red()
                .into()
        }));
        lines
    }

    #[allow(clippy::disallowed_methods)]
    fn event_page_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Self::event_header_lines();
        lines.push(Line::default());

        if let Some(message) = review_needed_message(self.review_needed_total_count()) {
            lines.push(format!("⚠ {message}").yellow().into());
            lines.push(Line::default());
        }

        let issue_lines = self.event_issue_lines();
        if !issue_lines.is_empty() {
            lines.extend(issue_lines);
            lines.push(Line::default());
        }

        lines.extend(self.event_table_lines());
        lines
    }

    #[allow(clippy::disallowed_methods)]
    fn handler_row_lines(&self, event_name: HookEventName, width: usize) -> Vec<Line<'static>> {
        self.handlers_for_event(event_name)
            .enumerate()
            .map(|(idx, hook)| {
                let marker = if hook_needs_review(hook) {
                    '!'
                } else if hook_is_active(hook) {
                    'x'
                } else {
                    ' '
                };
                let row = match hook.trust_status {
                    HookTrustStatus::Modified => {
                        format!("[{marker}] {} · modified", hook_title(idx))
                    }
                    HookTrustStatus::Untrusted => format!("[{marker}] {} · new", hook_title(idx)),
                    HookTrustStatus::Managed | HookTrustStatus::Trusted => {
                        format!("[{marker}] {}", hook_title(idx))
                    }
                };
                let mut line = Line::from(row);
                line = truncate_line_with_ellipsis_if_overflow(line, width);
                let needs_review = hook_needs_review(hook);
                if self.state.selected_idx == Some(idx) {
                    if needs_review {
                        line = line.yellow().bold();
                    } else {
                        line = line.patch_style(accent_style());
                    }
                } else if needs_review {
                    line = line.yellow();
                } else if hook.is_managed {
                    line = line.dim();
                }
                line
            })
            .collect()
    }

    fn detail_lines(&self, event_name: HookEventName, width: usize) -> Vec<Line<'static>> {
        let Some(hook) = self.selected_hook(event_name) else {
            return vec!["No hooks installed for this event.".dim().into()];
        };

        let mut lines = vec![detail_line("Event", event_label(event_name))];
        if let Some(matcher) = hook.matcher.as_deref() {
            lines.extend(detail_wrapped_lines(
                "Matcher", matcher, width, /*max_lines*/ None,
            ));
        }
        lines.extend(detail_wrapped_lines(
            "Source",
            &detail_source_value(hook),
            width,
            /*max_lines*/ None,
        ));
        lines.extend(detail_wrapped_lines(
            "Command",
            hook.command.as_deref().unwrap_or("-"),
            width,
            Some(MAX_COMMAND_DETAIL_LINES),
        ));
        lines.push(detail_line("Timeout", &format!("{}s", hook.timeout_sec)));
        if let Some(limit) = hook.additional_context_limit {
            let value = if limit == 0 {
                "unlimited".to_string()
            } else {
                format!("limit: {limit} approximate tokens")
            };
            lines.push(detail_line("Context", &value));
        }
        lines.push(detail_line("Trust", hook_trust_label(hook.trust_status)));
        lines
    }

    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let hint_area = Rect {
            x: area.x + 2,
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height,
        };
        let footer = match self.page {
            HooksBrowserPage::Events if self.review_needed_total_count() > 0 => Line::from(vec![
                "Press ".into(),
                key_hint::plain(KeyCode::Char('t')).into(),
                " to trust all; ".into(),
                key_hint::plain(KeyCode::Enter).into(),
                " to review hooks; ".into(),
                key_hint::plain(KeyCode::Esc).into(),
                " to close".into(),
            ]),
            HooksBrowserPage::Events => Line::from(vec![
                "Press ".into(),
                key_hint::plain(KeyCode::Enter).into(),
                " to view hooks; ".into(),
                key_hint::plain(KeyCode::Esc).into(),
                " to close".into(),
            ]),
            HooksBrowserPage::Handlers(event_name) => {
                let selected_hook = self.selected_hook(event_name);
                if selected_hook.is_none() {
                    Line::from(vec![
                        "Press ".into(),
                        key_hint::plain(KeyCode::Esc).into(),
                        " to go back".into(),
                    ])
                } else if selected_hook.is_some_and(|hook| hook.is_managed) {
                    Line::from(vec![
                        "Managed hooks are always on; press ".into(),
                        key_hint::plain(KeyCode::Esc).into(),
                        " to go back".into(),
                    ])
                } else if selected_hook.is_some_and(hook_needs_review) {
                    Line::from(vec![
                        "Press ".into(),
                        key_hint::plain(KeyCode::Char('t')).into(),
                        " to trust; ".into(),
                        key_hint::plain(KeyCode::Esc).into(),
                        " to go back".into(),
                    ])
                } else {
                    Line::from(vec![
                        "Press ".into(),
                        key_hint::plain(KeyCode::Char(' ')).into(),
                        " or ".into(),
                        key_hint::plain(KeyCode::Enter).into(),
                        " to toggle; ".into(),
                        key_hint::plain(KeyCode::Esc).into(),
                        " to go back".into(),
                    ])
                }
            }
        };
        footer.dim().render(hint_area, buf);
    }
}

impl BottomPaneView for HooksBrowserView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event {
            _ if self.keymap.move_up.is_pressed(key_event) => self.move_up(),
            _ if self.keymap.move_down.is_pressed(key_event) => self.move_down(),
            _ if self.keymap.page_up.is_pressed(key_event) => self.page_up(),
            _ if self.keymap.page_down.is_pressed(key_event) => self.page_down(),
            _ if self.keymap.jump_top.is_pressed(key_event) => self.jump_top(),
            _ if self.keymap.jump_bottom.is_pressed(key_event) => self.jump_bottom(),
            _ if self.keymap.accept.is_pressed(key_event)
                && self.page == HooksBrowserPage::Events =>
            {
                self.open_selected_event()
            }
            _ if self.keymap.accept.is_pressed(key_event) => {
                if let HooksBrowserPage::Handlers(event_name) = self.page {
                    self.toggle_selected_hook(event_name);
                }
            }
            KeyEvent {
                code: KeyCode::Char(' '),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if let HooksBrowserPage::Handlers(event_name) = self.page {
                    self.toggle_selected_hook(event_name);
                }
            }
            KeyEvent {
                code: KeyCode::Char('t'),
                modifiers: KeyModifiers::NONE,
                ..
            } => match self.page {
                HooksBrowserPage::Events => self.trust_all_hooks(),
                HooksBrowserPage::Handlers(event_name) => self.trust_selected_hook(event_name),
            },
            _ if self.keymap.cancel.is_pressed(key_event) => match self.page {
                HooksBrowserPage::Events => self.close(),
                HooksBrowserPage::Handlers(_) => self.return_to_events(),
            },
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.close();
        CancellationEvent::Handled
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        true
    }
}

impl Renderable for HooksBrowserView {
    fn desired_height(&self, width: u16) -> u16 {
        let content_width = width.saturating_sub(4) as usize;
        let height = match self.page {
            HooksBrowserPage::Events => self.event_page_lines().len(),
            HooksBrowserPage::Handlers(event_name) => {
                let row_count = self.handler_row_lines(event_name, content_width).len();
                let header_line_count =
                    Self::handler_header_lines(event_name, self.review_needed_count(event_name))
                        .len();
                if row_count == 0 {
                    header_line_count + 2
                } else {
                    let visible_row_count = row_count.min(MAX_POPUP_ROWS);
                    header_line_count
                        + 1
                        + visible_row_count
                        + 1
                        + self.detail_lines(event_name, content_width).len()
                }
            }
        };
        (height + 3).try_into().unwrap_or(u16::MAX)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }

        let [content_area, footer_area] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        let content_area = render_menu_surface(content_area, buf);
        let width = content_area.width as usize;
        let lines = match self.page {
            HooksBrowserPage::Events => self.event_page_lines(),
            HooksBrowserPage::Handlers(event_name) => {
                let mut lines =
                    Self::handler_header_lines(event_name, self.review_needed_count(event_name));
                let rows = self.handler_row_lines(event_name, width);
                if rows.is_empty() {
                    lines.push(Line::default());
                    lines.push(Line::from(
                        "No hooks installed for this event.".dim().italic(),
                    ));
                    lines.push(Line::default());
                    Paragraph::new(lines).render(content_area, buf);
                    self.render_footer(footer_area, buf);
                    return;
                }
                let list_height = rows.len().clamp(1, MAX_POPUP_ROWS) as u16;
                lines.push(Line::default());
                let header_height = lines.len() as u16;
                let [header_area, list_area, detail_area] = Layout::vertical([
                    Constraint::Length(header_height),
                    Constraint::Length(list_height),
                    Constraint::Fill(1),
                ])
                .areas(content_area);
                Paragraph::new(lines.clone()).render(header_area, buf);
                let visible_rows = rows
                    .into_iter()
                    .skip(self.state.scroll_top)
                    .take(list_height as usize)
                    .collect::<Vec<_>>();
                Paragraph::new(visible_rows).render(list_area, buf);
                let mut detail_lines = vec![Line::default()];
                detail_lines.extend(self.detail_lines(event_name, width));
                Paragraph::new(detail_lines).render(detail_area, buf);
                self.render_footer(footer_area, buf);
                return;
            }
        };
        Paragraph::new(lines).render(content_area, buf);
        self.render_footer(footer_area, buf);
    }
}

// ── 格式化/标签辅助（外移子模块） ──
mod formatting;
use formatting::*;
