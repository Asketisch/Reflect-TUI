//! hooks 浏览器视图格式化/标签辅助函数簇。从 hooks_browser_view.rs 抽出。

use super::*;

pub(super) fn hook_is_active(hook: &HookMetadata) -> bool {
    hook.enabled
        && matches!(
            hook.trust_status,
            HookTrustStatus::Managed | HookTrustStatus::Trusted
        )
}

pub(super) fn review_needed_message(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 hook needs review before it can run.".to_string()),
        count => Some(format!("{count} hooks need review before they can run.")),
    }
}

pub(super) struct EventRow {
    pub(super) event_name: HookEventName,
    pub(super) installed: usize,
    pub(super) active: usize,
    pub(super) needs_review: usize,
}

pub(super) fn hook_trust_label(status: HookTrustStatus) -> &'static str {
    match status {
        HookTrustStatus::Managed => "Managed",
        HookTrustStatus::Trusted => "Trusted",
        HookTrustStatus::Untrusted => "New hook - review required",
        HookTrustStatus::Modified => "Modified since last trusted - review required",
    }
}

pub(super) fn event_label(event_name: HookEventName) -> &'static str {
    match event_name {
        HookEventName::PreToolUse => "PreToolUse",
        HookEventName::PermissionRequest => "PermissionRequest",
        HookEventName::PostToolUse => "PostToolUse",
        HookEventName::PreCompact => "PreCompact",
        HookEventName::PostCompact => "PostCompact",
        HookEventName::SessionStart => "SessionStart",
        HookEventName::SessionEnd => "SessionEnd",
        HookEventName::UserPromptSubmit => "UserPromptSubmit",
        HookEventName::SubagentStart => "SubagentStart",
        HookEventName::SubagentStop => "SubagentStop",
        HookEventName::Stop => "Stop",
    }
}

pub(super) fn event_description(event_name: HookEventName) -> &'static str {
    match event_name {
        HookEventName::PreToolUse => "Before a tool executes",
        HookEventName::PermissionRequest => "When permission is requested",
        HookEventName::PostToolUse => "After a tool executes",
        HookEventName::PreCompact => "Before context compaction",
        HookEventName::PostCompact => "After context compaction",
        HookEventName::SessionStart => "When a new session starts",
        HookEventName::SessionEnd => "Right before a session ends",
        HookEventName::UserPromptSubmit => "When the user submits a prompt",
        HookEventName::SubagentStart => "When a subagent is created",
        HookEventName::SubagentStop => "Right before a subagent ends its turn",
        HookEventName::Stop => "Right before Reflect ends its turn",
    }
}

pub(super) fn hook_title(idx: usize) -> String {
    format!("Hook {}", idx + 1)
}

pub(super) fn hook_source_summary(hook: &HookMetadata) -> String {
    match hook.source {
        HookSource::Plugin => hook
            .plugin_id
            .as_deref()
            .map(|plugin_id| format!("Plugin - {plugin_id}"))
            .unwrap_or_else(|| "Plugin".to_string()),
        _ => config_source_label(hook.source).to_string(),
    }
}

pub(super) fn detail_source_value(hook: &HookMetadata) -> String {
    match hook.source {
        HookSource::Plugin => hook_source_summary(hook),
        HookSource::System
        | HookSource::Mdm
        | HookSource::CloudRequirements
        | HookSource::CloudManagedConfig
        | HookSource::LegacyManagedConfigFile
        | HookSource::LegacyManagedConfigMdm => config_source_label(hook.source).to_string(),
        _ => format!(
            "{} - {}",
            config_source_label(hook.source),
            format_directory_display(&hook.source_path, /*max_width*/ None)
        ),
    }
}

pub(super) fn config_source_label(source: HookSource) -> &'static str {
    match source {
        HookSource::System => "Admin config",
        HookSource::User => "User config",
        HookSource::Project => "Project config",
        HookSource::Mdm => "Admin config",
        HookSource::SessionFlags => "Session flags",
        HookSource::Plugin => unreachable!("plugin hooks are handled by summary_source"),
        HookSource::CloudRequirements => "Admin config",
        HookSource::CloudManagedConfig => "Cloud-managed config",
        HookSource::LegacyManagedConfigFile => "Admin config",
        HookSource::LegacyManagedConfigMdm => "Admin config",
        HookSource::Unknown => "Unknown source",
    }
}

pub(super) fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![format!("{label:<10}").into(), value.to_string().dim()])
}

pub(super) fn detail_wrapped_lines(
    label: &str,
    value: &str,
    width: usize,
    max_lines: Option<usize>,
) -> Vec<Line<'static>> {
    let prefix = format!("{label:<10}");
    let available = width.saturating_sub(prefix.width()).max(1);
    let mut wrapped = textwrap::wrap(value, available).into_iter();
    let first = wrapped.next().unwrap_or_default().into_owned();
    let mut lines = vec![Line::from(vec![prefix.into(), first.dim()])];
    lines
        .extend(wrapped.map(|line| Line::from(vec!["          ".into(), line.into_owned().dim()])));
    let Some(max_lines) = max_lines else {
        return lines;
    };
    if lines.len() <= max_lines {
        return lines;
    }

    lines.truncate(max_lines);
    if let Some(last_line) = lines.last_mut() {
        let prefix_width = last_line.spans[..last_line.spans.len().saturating_sub(1)]
            .iter()
            .map(ratatui::prelude::Span::width)
            .sum::<usize>();
        let max_width = width.saturating_sub(prefix_width);
        let Some(last_span) = last_line.spans.last_mut() else {
            return lines;
        };
        let truncated = truncate_line_with_ellipsis_if_overflow(
            Line::from(format!("{}…", last_span.content)),
            max_width,
        );
        let content = truncated
            .spans
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<String>();
        last_span.content = content.into();
    }
    lines
}
