use crate::app_server_protocol::AskForApproval;
use crate::config_compat::types::ApprovalsReviewer;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::account::PlanType;
use crate::protocol_compat::models::ActivePermissionProfile;
use crate::protocol_compat::models::BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS;
use crate::protocol_compat::models::BUILT_IN_PERMISSION_PROFILE_READ_ONLY;
use crate::protocol_compat::models::BUILT_IN_PERMISSION_PROFILE_WORKSPACE;
use crate::protocol_compat::models::PermissionProfile;
use crate::protocol_compat::openai_models::ReasoningEffort;
use crate::tui_core::history_cell::CompositeHistoryCell;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::history_cell::PlainHistoryCell;
use crate::tui_core::history_cell::plain_lines;
use crate::tui_core::history_cell::with_border_with_inner_width;
use crate::tui_core::legacy_core::config::Config;
use crate::tui_core::token_usage::TokenUsage;
use crate::tui_core::token_usage::TokenUsageInfo;
use crate::tui_core::version::REFLECT_CLI_VERSION;
use crate::utils_absolute_path::AbsolutePathBuf;
use crate::utils_sandbox_summary::summarize_permission_profile;
use chrono::DateTime;
use chrono::Local;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use url::Url;

use super::account::StatusAccountDisplay;
use super::format::FieldFormatter;
use super::format::line_display_width;
use super::format::push_label;
use super::format::truncate_line_to_width;
use super::helpers::compose_account_display;
use super::helpers::compose_model_display;
use super::helpers::format_directory_display;
use super::helpers::format_tokens_compact;
use super::rate_limits::RateLimitSnapshotDisplay;
use super::rate_limits::StatusRateLimitData;
use super::rate_limits::StatusRateLimitRow;
use super::rate_limits::StatusRateLimitValue;
use super::rate_limits::compose_rate_limit_data;
use super::rate_limits::compose_rate_limit_data_many;
use super::rate_limits::format_status_limit_summary;
use super::rate_limits::render_status_limit_progress_bar;
use super::remote_connection::RemoteConnectionStatus;
use crate::tui_core::wrapping::RtOptions;
use crate::tui_core::wrapping::word_wrap_lines;
use std::sync::Arc;
use std::sync::RwLock;

#[derive(Debug, Clone)]
struct StatusContextWindowData {
    percent_remaining: i64,
    tokens_in_context: i64,
    window: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct StatusTokenUsageData {
    total: i64,
    input: i64,
    output: i64,
    context_window: Option<StatusContextWindowData>,
}

#[derive(Debug)]
struct StatusRateLimitState {
    rate_limits: StatusRateLimitData,
    refreshing_rate_limits: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StatusHistoryHandle {
    rate_limit_state: Arc<RwLock<StatusRateLimitState>>,
}

impl StatusHistoryHandle {
    pub(crate) fn finish_rate_limit_refresh(
        &self,
        rate_limits: &[RateLimitSnapshotDisplay],
        now: DateTime<Local>,
    ) {
        let rate_limits = if rate_limits.len() <= 1 {
            compose_rate_limit_data(rate_limits.first(), now)
        } else {
            compose_rate_limit_data_many(rate_limits, now)
        };
        #[expect(clippy::expect_used)]
        let mut state = self
            .rate_limit_state
            .write()
            .expect("status history rate-limit state poisoned");
        state.rate_limits = rate_limits;
        state.refreshing_rate_limits = false;
    }
}

#[derive(Debug)]
struct StatusHistoryCell {
    model_name: String,
    model_details: Vec<String>,
    directory: PathBuf,
    permissions: String,
    agents_summary: Arc<RwLock<String>>,
    collaboration_mode: Option<String>,
    model_provider: Option<String>,
    remote_connection: Option<RemoteConnectionStatus>,
    account: Option<StatusAccountDisplay>,
    thread_name: Option<String>,
    session_id: Option<String>,
    forked_from: Option<String>,
    token_usage: StatusTokenUsageData,
    rate_limit_state: Arc<RwLock<StatusRateLimitState>>,
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_status_output(
    config: &Config,
    account_display: Option<&StatusAccountDisplay>,
    token_info: Option<&TokenUsageInfo>,
    total_usage: &TokenUsage,
    session_id: &Option<ThreadId>,
    thread_name: Option<String>,
    forked_from: Option<ThreadId>,
    rate_limits: Option<&RateLimitSnapshotDisplay>,
    _plan_type: Option<PlanType>,
    now: DateTime<Local>,
    model_name: &str,
    collaboration_mode: Option<&str>,
    reasoning_effort_override: Option<Option<ReasoningEffort>>,
) -> CompositeHistoryCell {
    let snapshots = rate_limits.map(std::slice::from_ref).unwrap_or_default();
    new_status_output_with_rate_limits(
        config,
        account_display,
        token_info,
        total_usage,
        session_id,
        thread_name,
        forked_from,
        snapshots,
        _plan_type,
        now,
        model_name,
        collaboration_mode,
        reasoning_effort_override,
        /*refreshing_rate_limits*/ false,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn new_status_output_with_rate_limits(
    config: &Config,
    account_display: Option<&StatusAccountDisplay>,
    token_info: Option<&TokenUsageInfo>,
    total_usage: &TokenUsage,
    session_id: &Option<ThreadId>,
    thread_name: Option<String>,
    forked_from: Option<ThreadId>,
    rate_limits: &[RateLimitSnapshotDisplay],
    _plan_type: Option<PlanType>,
    now: DateTime<Local>,
    model_name: &str,
    collaboration_mode: Option<&str>,
    reasoning_effort_override: Option<Option<ReasoningEffort>>,
    refreshing_rate_limits: bool,
) -> CompositeHistoryCell {
    new_status_output_with_rate_limits_handle(
        config,
        /*runtime_model_provider_base_url*/ None,
        /*remote_connection*/ None,
        account_display,
        token_info,
        total_usage,
        session_id,
        thread_name,
        forked_from,
        rate_limits,
        _plan_type,
        now,
        model_name,
        collaboration_mode,
        reasoning_effort_override,
        "<none>".to_string(),
        refreshing_rate_limits,
    )
    .0
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn new_status_output_with_rate_limits_handle(
    config: &Config,
    runtime_model_provider_base_url: Option<&str>,
    remote_connection: Option<&RemoteConnectionStatus>,
    account_display: Option<&StatusAccountDisplay>,
    token_info: Option<&TokenUsageInfo>,
    total_usage: &TokenUsage,
    session_id: &Option<ThreadId>,
    thread_name: Option<String>,
    forked_from: Option<ThreadId>,
    rate_limits: &[RateLimitSnapshotDisplay],
    _plan_type: Option<PlanType>,
    now: DateTime<Local>,
    model_name: &str,
    collaboration_mode: Option<&str>,
    reasoning_effort_override: Option<Option<ReasoningEffort>>,
    agents_summary: String,
    refreshing_rate_limits: bool,
) -> (CompositeHistoryCell, StatusHistoryHandle) {
    let command = PlainHistoryCell::new(vec!["/status".magenta().into()]);
    let (card, handle) = StatusHistoryCell::new(
        config,
        runtime_model_provider_base_url,
        remote_connection,
        account_display,
        token_info,
        total_usage,
        session_id,
        thread_name,
        forked_from,
        rate_limits,
        _plan_type,
        now,
        model_name,
        collaboration_mode,
        reasoning_effort_override,
        agents_summary,
        refreshing_rate_limits,
    );

    (
        CompositeHistoryCell::new(vec![Box::new(command), Box::new(card)]),
        handle,
    )
}

impl StatusHistoryCell {
    #[allow(clippy::too_many_arguments)]
    fn new(
        config: &Config,
        runtime_model_provider_base_url: Option<&str>,
        remote_connection: Option<&RemoteConnectionStatus>,
        account_display: Option<&StatusAccountDisplay>,
        token_info: Option<&TokenUsageInfo>,
        total_usage: &TokenUsage,
        session_id: &Option<ThreadId>,
        thread_name: Option<String>,
        forked_from: Option<ThreadId>,
        rate_limits: &[RateLimitSnapshotDisplay],
        _plan_type: Option<PlanType>,
        now: DateTime<Local>,
        model_name: &str,
        collaboration_mode: Option<&str>,
        reasoning_effort_override: Option<Option<ReasoningEffort>>,
        agents_summary: String,
        refreshing_rate_limits: bool,
    ) -> (Self, StatusHistoryHandle) {
        let approval_policy = AskForApproval::from(config.permissions.approval_policy.value());
        let permission_profile = config.permissions.effective_permission_profile();
        let workspace_roots = config.effective_workspace_roots();
        let mut config_entries = vec![
            ("workdir", config.cwd.display().to_string()),
            ("model", model_name.to_string()),
            ("provider", config.model_provider_id.clone()),
            (
                "approval",
                config.permissions.approval_policy.value().to_string(),
            ),
            (
                "sandbox",
                summarize_permission_profile(
                    &permission_profile,
                    &config.cwd,
                    workspace_roots.as_slice(),
                ),
            ),
        ];
        if config.model_provider.wire_api == "responses" {
            let effort_value = reasoning_effort_override
                .unwrap_or_else(|| {
                    config
                        .model_reasoning_effort
                        .clone()
                        .map(|s| crate::protocol_compat::openai_models::ReasoningEffort::from(s))
                })
                .map(|effort| effort.to_string())
                .unwrap_or_else(|| "none".to_string());
            config_entries.push(("reasoning effort", effort_value));
            config_entries.push(("reasoning summaries", {
                let summary = config.model_reasoning_summary.clone();
                if summary.is_empty() {
                    "auto".to_string()
                } else {
                    summary
                }
            }));
        }
        let (model_name, model_details) = compose_model_display(model_name, &config_entries);
        let approval = config_entries
            .iter()
            .find(|(k, _)| *k == "approval")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| "<unknown>".to_string());
        let active_permission_profile = config.permissions.active_permission_profile();
        let sandbox =
            status_permission_summary(&permission_profile, &config.cwd, workspace_roots.as_slice());
        let workspace_root_suffix = workspace_root_suffix(workspace_roots.as_slice(), &config.cwd);
        let approval = status_approval_label(approval_policy, config.approvals_reviewer, &approval);
        let permissions = status_permissions_label(
            active_permission_profile.as_ref(),
            &permission_profile,
            approval_policy,
            &sandbox,
            &approval,
            workspace_root_suffix.as_deref(),
        );
        let model_provider = format_model_provider(config, runtime_model_provider_base_url);
        let account = compose_account_display(account_display);
        let session_id = session_id.as_ref().map(std::string::ToString::to_string);
        let forked_from = forked_from.map(|id| id.to_string());
        let default_usage = TokenUsage::default();
        let (context_usage, context_window) = match token_info {
            Some(info) => (&info.last_token_usage, info.model_context_window),
            None => (
                &default_usage,
                config.model_context_window.parse::<i64>().ok(),
            ),
        };
        let context_window = context_window.map(|window| StatusContextWindowData {
            percent_remaining: context_usage.percent_of_context_window_remaining(window),
            tokens_in_context: context_usage.tokens_in_context_window(),
            window,
        });

        let token_usage = StatusTokenUsageData {
            total: total_usage.blended_total(),
            input: total_usage.non_cached_input(),
            output: total_usage.output_tokens,
            context_window,
        };
        let rate_limits = if rate_limits.len() <= 1 {
            compose_rate_limit_data(rate_limits.first(), now)
        } else {
            compose_rate_limit_data_many(rate_limits, now)
        };
        let rate_limit_state = Arc::new(RwLock::new(StatusRateLimitState {
            rate_limits,
            refreshing_rate_limits,
        }));
        let agents_summary = Arc::new(RwLock::new(agents_summary));

        (
            Self {
                model_name,
                model_details,
                directory: config.cwd.to_path_buf(),
                permissions,
                collaboration_mode: collaboration_mode.map(ToString::to_string),
                model_provider,
                remote_connection: remote_connection.cloned(),
                account,
                thread_name,
                session_id,
                forked_from,
                token_usage,
                agents_summary,
                rate_limit_state: rate_limit_state.clone(),
            },
            StatusHistoryHandle { rate_limit_state },
        )
    }

    fn token_usage_spans(&self) -> Vec<Span<'static>> {
        let total_fmt = format_tokens_compact(self.token_usage.total);
        let input_fmt = format_tokens_compact(self.token_usage.input);
        let output_fmt = format_tokens_compact(self.token_usage.output);

        vec![
            Span::from(total_fmt),
            Span::from(" total "),
            Span::from(" (").dim(),
            Span::from(input_fmt).dim(),
            Span::from(" input").dim(),
            Span::from(" + ").dim(),
            Span::from(output_fmt).dim(),
            Span::from(" output").dim(),
            Span::from(")").dim(),
        ]
    }

    fn context_window_spans(&self) -> Option<Vec<Span<'static>>> {
        let context = self.token_usage.context_window.as_ref()?;
        let percent = context.percent_remaining;
        let used_fmt = format_tokens_compact(context.tokens_in_context);
        let window_fmt = format_tokens_compact(context.window);

        Some(vec![
            Span::from(format!("{percent}% left")),
            Span::from(" (").dim(),
            Span::from(used_fmt).dim(),
            Span::from(" used / ").dim(),
            Span::from(window_fmt).dim(),
            Span::from(")").dim(),
        ])
    }

    fn rate_limit_lines(
        &self,
        state: &StatusRateLimitState,
        available_inner_width: usize,
        formatter: &FieldFormatter,
    ) -> Vec<Line<'static>> {
        match &state.rate_limits {
            StatusRateLimitData::Available(rows_data) => {
                if rows_data.is_empty() {
                    return vec![formatter.line(
                        "Limits",
                        vec![Span::from("not available for this account").dim()],
                    )];
                }

                self.rate_limit_row_lines(rows_data, available_inner_width, formatter)
            }
            StatusRateLimitData::Stale(rows_data) => {
                let mut lines =
                    self.rate_limit_row_lines(rows_data, available_inner_width, formatter);
                lines.push(formatter.line(
                    "Warning",
                    vec![Span::from(if state.refreshing_rate_limits {
                        "limits may be stale - run /status again shortly."
                    } else {
                        "limits may be stale - start new turn to refresh."
                    })
                    .dim()],
                ));
                lines
            }
            StatusRateLimitData::Unavailable => {
                vec![formatter.line(
                    "Limits",
                    vec![Span::from("not available for this account").dim()],
                )]
            }
            StatusRateLimitData::Missing => {
                vec![formatter.line(
                    "Limits",
                    vec![Span::from(if state.refreshing_rate_limits {
                        "refresh requested; run /status again shortly."
                    } else {
                        "data not available yet"
                    })
                    .dim()],
                )]
            }
        }
    }

    fn rate_limit_row_lines(
        &self,
        rows: &[StatusRateLimitRow],
        available_inner_width: usize,
        formatter: &FieldFormatter,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::with_capacity(rows.len().saturating_mul(2));

        for row in rows {
            match &row.value {
                StatusRateLimitValue::Window {
                    percent_used,
                    resets_at,
                    details,
                } => {
                    let percent_remaining = (100.0 - percent_used).clamp(0.0, 100.0);
                    let summary = format_status_limit_summary(percent_remaining);
                    let full_value_spans = vec![
                        Span::from(render_status_limit_progress_bar(percent_remaining)),
                        Span::from(" "),
                        Span::from(summary.clone()),
                    ];
                    // 在窄终端上，保持百分比可见，
                    // 而不是让固定宽度的进度条挤掉重置时间的显示空间。
                    let value_spans = if line_display_width(&Line::from(full_value_spans.clone()))
                        <= formatter.value_width(available_inner_width)
                    {
                        full_value_spans
                    } else {
                        vec![Span::from(summary)]
                    };
                    let base_spans = formatter.full_spans(row.label.as_str(), value_spans);
                    let base_line = Line::from(base_spans.clone());

                    if let Some(resets_at) = resets_at.as_ref() {
                        let resets_span = Span::from(format!("(resets {resets_at})")).dim();
                        let mut inline_spans = base_spans.clone();
                        inline_spans.push(Span::from(" ").dim());
                        inline_spans.push(resets_span.clone());

                        if line_display_width(&Line::from(inline_spans.clone()))
                            <= available_inner_width
                        {
                            lines.push(Line::from(inline_spans));
                        } else {
                            lines.push(base_line);
                            let reset_text = format!("(resets {resets_at})");
                            let reset_width = formatter.value_width(available_inner_width).max(1);
                            let wrap_options =
                                textwrap::Options::new(reset_width).break_words(false);
                            // 重置时间是此行中可操作的信息，因此将其换行
                            // 到延续行，而不是截断部分时间/日期。
                            lines.extend(
                                textwrap::wrap(reset_text.as_str(), wrap_options)
                                    .into_iter()
                                    .map(|wrapped| {
                                        formatter.continuation(vec![
                                            Span::from(wrapped.into_owned()).dim(),
                                        ])
                                    }),
                            );
                        }
                    } else {
                        lines.push(base_line);
                    }
                    if let Some(details) = details {
                        let detail_width = formatter.value_width(available_inner_width).max(1);
                        let wrap_options = textwrap::Options::new(detail_width).break_words(false);
                        lines.extend(
                            textwrap::wrap(details.as_str(), wrap_options)
                                .into_iter()
                                .map(|wrapped| {
                                    formatter
                                        .continuation(vec![Span::from(wrapped.into_owned()).dim()])
                                }),
                        );
                    }
                }
                StatusRateLimitValue::Text(text) => {
                    let label = row.label.clone();
                    let spans =
                        formatter.full_spans(label.as_str(), vec![Span::from(text.clone())]);
                    lines.push(Line::from(spans));
                }
            }
        }

        lines
    }

    fn collect_rate_limit_labels(
        &self,
        state: &StatusRateLimitState,
        seen: &mut BTreeSet<String>,
        labels: &mut Vec<String>,
    ) {
        match &state.rate_limits {
            StatusRateLimitData::Available(rows) => {
                if rows.is_empty() {
                    push_label(labels, seen, "Limits");
                } else {
                    for row in rows {
                        push_label(labels, seen, row.label.as_str());
                    }
                }
            }
            StatusRateLimitData::Stale(rows) => {
                for row in rows {
                    push_label(labels, seen, row.label.as_str());
                }
                push_label(labels, seen, "Warning");
            }
            StatusRateLimitData::Unavailable => push_label(labels, seen, "Limits"),
            StatusRateLimitData::Missing => push_label(labels, seen, "Limits"),
        }
    }
}

// ── 标签/格式化辅助（外移子模块） ──
mod labels;
use labels::*;

impl HistoryCell for StatusHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(vec![
            Span::from(format!("{}>_ ", FieldFormatter::INDENT)).dim(),
            Span::from("Reflect").bold(),
            Span::from(" ").dim(),
            Span::from(format!("(v{REFLECT_CLI_VERSION})")).dim(),
        ]));

        let available_inner_width = usize::from(width.saturating_sub(4));
        if available_inner_width == 0 {
            return Vec::new();
        }

        let account_value = self.account.as_ref().map(|account| match account {
            StatusAccountDisplay::ChatGpt { email, plan } => match (email, plan) {
                (Some(email), Some(plan)) => format!("{email} ({plan})"),
                (Some(email), None) => email.clone(),
                (None, Some(plan)) => plan.clone(),
                (None, None) => "Reflect".to_string(),
            },
            StatusAccountDisplay::ApiKey => {
                "API key configured (run reflect login to use Reflect)".to_string()
            }
        });

        let mut labels: Vec<String> = vec!["Model", "Directory", "Permissions", "Agents.md"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let mut seen: BTreeSet<String> = labels.iter().cloned().collect();
        let thread_name = self.thread_name.as_deref().filter(|name| !name.is_empty());
        #[expect(clippy::expect_used)]
        let rate_limit_state = self
            .rate_limit_state
            .read()
            .expect("status history rate-limit state poisoned");
        #[expect(clippy::expect_used)]
        let agents_summary = self
            .agents_summary
            .read()
            .expect("status history agents summary state poisoned")
            .clone();

        if self.model_provider.is_some() {
            push_label(&mut labels, &mut seen, "Model provider");
        }
        if account_value.is_some() {
            push_label(&mut labels, &mut seen, "Account");
        }
        if thread_name.is_some() {
            push_label(&mut labels, &mut seen, "Thread name");
        }
        if self.session_id.is_some() {
            push_label(&mut labels, &mut seen, "Session");
        }
        if self.session_id.is_some() && self.forked_from.is_some() {
            push_label(&mut labels, &mut seen, "Forked from");
        }
        if self.collaboration_mode.is_some() {
            push_label(&mut labels, &mut seen, "Collaboration mode");
        }
        push_label(&mut labels, &mut seen, "Token usage");
        if self.token_usage.context_window.is_some() {
            push_label(&mut labels, &mut seen, "Context window");
        }

        self.collect_rate_limit_labels(&rate_limit_state, &mut seen, &mut labels);

        let formatter = FieldFormatter::from_labels(labels.iter().map(String::as_str));
        let value_width = formatter.value_width(available_inner_width);

        if let Some(remote_connection) = self.remote_connection.as_ref() {
            let wrapped_remote = word_wrap_lines(
                [Line::from(vec![
                    Span::from(remote_connection.address.clone()),
                    Span::from(" (").dim(),
                    Span::from(remote_connection.version.clone()).dim(),
                    Span::from(")").dim(),
                ])],
                RtOptions::new(value_width.max(1)),
            );
            let mut wrapped_remote = wrapped_remote.into_iter();
            if let Some(first) = wrapped_remote.next() {
                lines.push(formatter.line("Remote", first.spans));
                lines.extend(wrapped_remote.map(|line| formatter.continuation(line.spans)));
            }
            lines.push(Line::from(Vec::<Span<'static>>::new()));
        }

        let mut model_spans = vec![Span::from(self.model_name.clone())];
        if !self.model_details.is_empty() {
            model_spans.push(Span::from(" (").dim());
            model_spans.push(Span::from(self.model_details.join(", ")).dim());
            model_spans.push(Span::from(")").dim());
        }

        let directory_value = format_directory_display(&self.directory, Some(value_width));

        lines.push(formatter.line("Model", model_spans));
        if let Some(model_provider) = self.model_provider.as_ref() {
            lines.push(formatter.line("Model provider", vec![Span::from(model_provider.clone())]));
        }
        lines.push(formatter.line("Directory", vec![Span::from(directory_value)]));
        lines.push(formatter.line("Permissions", vec![Span::from(self.permissions.clone())]));
        lines.push(formatter.line("Agents.md", vec![Span::from(agents_summary)]));

        if let Some(account_value) = account_value {
            lines.push(formatter.line("Account", vec![Span::from(account_value)]));
        }

        if let Some(thread_name) = thread_name {
            lines.push(formatter.line("Thread name", vec![Span::from(thread_name.to_string())]));
        }
        if let Some(collab_mode) = self.collaboration_mode.as_ref() {
            lines.push(formatter.line("Collaboration mode", vec![Span::from(collab_mode.clone())]));
        }
        if let Some(session) = self.session_id.as_ref() {
            lines.push(formatter.line("Session", vec![Span::from(session.clone())]));
        }
        if self.session_id.is_some()
            && let Some(forked_from) = self.forked_from.as_ref()
        {
            lines.push(formatter.line("Forked from", vec![Span::from(forked_from.clone())]));
        }

        lines.push(Line::from(Vec::<Span<'static>>::new()));
        // 仅对 Reflect 订阅者隐藏 token 用量
        if !matches!(self.account, Some(StatusAccountDisplay::ChatGpt { .. })) {
            lines.push(formatter.line("Token usage", self.token_usage_spans()));
        }

        if let Some(spans) = self.context_window_spans() {
            lines.push(formatter.line("Context window", spans));
        }

        lines.extend(self.rate_limit_lines(&rate_limit_state, available_inner_width, &formatter));

        let content_width = lines.iter().map(line_display_width).max().unwrap_or(0);
        let inner_width = content_width.min(available_inner_width);
        let truncated_lines: Vec<Line<'static>> = lines
            .into_iter()
            .map(|line| truncate_line_to_width(line, inner_width))
            .collect();

        with_border_with_inner_width(truncated_lines, inner_width)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.display_lines(u16::MAX))
    }

    fn display_hyperlink_lines(
        &self,
        width: u16,
    ) -> Vec<crate::tui_core::terminal_hyperlinks::HyperlinkLine> {
        crate::tui_core::terminal_hyperlinks::plain_hyperlink_lines(self.display_lines(width))
    }

    fn transcript_hyperlink_lines(
        &self,
        width: u16,
    ) -> Vec<crate::tui_core::terminal_hyperlinks::HyperlinkLine> {
        self.display_hyperlink_lines(width)
    }
}
