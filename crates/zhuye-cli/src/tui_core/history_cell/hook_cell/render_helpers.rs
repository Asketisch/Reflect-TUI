//! hook 历史单元渲染辅助：上下文预览、运行分组、状态派生等。从 hook_cell.rs 抽出。

use super::*;

pub(super) fn push_full_hook_output_entry(lines: &mut Vec<Line<'static>>, entry: &HookOutputEntry) {
    let prefix = hook_output_prefix(entry.kind);
    let mut output_lines = entry.text.split('\n');
    if let Some(first_line) = output_lines.next() {
        lines.push(format!("{HOOK_OUTPUT_INDENT}{prefix}{first_line}").into());
    }
    for line in output_lines {
        if line.is_empty() {
            lines.push("".into());
        } else {
            lines.push(format!("{HOOK_OUTPUT_BODY_INDENT}{line}").into());
        }
    }
}

pub(super) fn hook_context_preview_lines(text: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let mut wrapped = Vec::new();
    let mut source_lines = text.split('\n');
    let first_line = source_lines.next().unwrap_or_default();
    push_wrapped_hook_context_line(
        &mut wrapped,
        first_line,
        width,
        Line::from(format!(
            "{HOOK_OUTPUT_INDENT}{}",
            hook_output_prefix(HookOutputEntryKind::Context)
        )),
    );
    for line in source_lines {
        if line.is_empty() {
            wrapped.push("".into());
        } else {
            push_wrapped_hook_context_line(
                &mut wrapped,
                line,
                width,
                Line::from(HOOK_OUTPUT_BODY_INDENT),
            );
        }
    }

    if wrapped.len() <= HOOK_CONTEXT_MAX_DISPLAY_ROWS {
        return wrapped;
    }

    let retained_rows = HOOK_CONTEXT_MAX_DISPLAY_ROWS - 1;
    let omitted_rows = wrapped.len() - retained_rows;
    wrapped.truncate(retained_rows);
    let hint = vec![
        HOOK_OUTPUT_BODY_INDENT.into(),
        format!("… +{omitted_rows} lines ({TRANSCRIPT_HINT})").dim(),
    ]
    .into();
    wrapped.push(truncate_line_with_ellipsis_if_overflow(hint, width));
    wrapped
}

pub(super) fn push_wrapped_hook_context_line(
    output: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    initial_indent: Line<'static>,
) {
    let line = Line::from(text.to_string());
    let wrapped = word_wrap_line(
        &line,
        RtOptions::new(width)
            .initial_indent(initial_indent)
            .subsequent_indent(Line::from(HOOK_OUTPUT_BODY_INDENT)),
    );
    push_owned_lines(&wrapped, output);
}

impl HookRunState {
    /// 为实时 hook 运行创建隐藏的初始状态。
    pub(super) fn pending(start_time: Instant) -> Self {
        Self::PendingReveal {
            start_time,
            reveal_deadline: start_time + HOOK_RUN_REVEAL_DELAY,
        }
    }

    /// 为具有可见输出或显著状态的 hook 创建持久最终状态。
    pub(super) fn completed(status: HookRunStatus, entries: Vec<HookOutputEntry>) -> Self {
        Self::Completed { status, entries }
    }

    /// 当运行仍在等待完成事件或定时器清理时返回 true。
    pub(super) fn is_active(&self) -> bool {
        match self {
            HookRunState::PendingReveal { .. }
            | HookRunState::VisibleRunning { .. }
            | HookRunState::QuietLinger { .. } => true,
            HookRunState::Completed { .. } => false,
        }
    }

    /// 当此运行至少为当前渲染贡献一行时返回 true。
    pub(super) fn should_render(&self) -> bool {
        match self {
            HookRunState::VisibleRunning { .. }
            | HookRunState::QuietLinger { .. }
            | HookRunState::Completed { .. } => true,
            HookRunState::PendingReveal { .. } => false,
        }
    }

    /// 对应保留在活动单元之外的已完成运行返回 true。
    pub(super) fn has_persistent_output(&self) -> bool {
        match self {
            HookRunState::Completed { status, entries } => {
                *status != HookRunStatus::Completed || !entries.is_empty()
            }
            HookRunState::PendingReveal { .. }
            | HookRunState::VisibleRunning { .. }
            | HookRunState::QuietLinger { .. } => false,
        }
    }

    /// 返回活动状态的原始开始时间。
    ///
    /// 已完成的运行不再显示动画，因此有意不保留开始时间。
    pub(super) fn start_time(&self) -> Option<Instant> {
        match self {
            HookRunState::PendingReveal { start_time, .. }
            | HookRunState::VisibleRunning { start_time, .. }
            | HookRunState::QuietLinger { start_time, .. } => Some(*start_time),
            HookRunState::Completed { .. } => None,
        }
    }

    /// 当运行应被视为进行中行时返回 true。
    pub(super) fn is_running_visible(&self) -> bool {
        matches!(
            self,
            HookRunState::VisibleRunning { .. } | HookRunState::QuietLinger { .. }
        )
    }

    /// 待处理运行超过截止时间后将其显示。
    ///
    /// 仅当此次调用改变状态时才返回 true，从而使定时器回调避免
    /// 不必要的重绘。
    pub(super) fn reveal_if_due(&mut self, now: Instant) -> bool {
        let HookRunState::PendingReveal {
            start_time,
            reveal_deadline,
        } = self
        else {
            return false;
        };
        if now < *reveal_deadline {
            return false;
        }
        *self = HookRunState::VisibleRunning {
            start_time: *start_time,
            visible_since: now,
        };
        true
    }

    /// 返回此运行所持有的下一个状态机截止时间。
    pub(super) fn next_timer_deadline(&self) -> Option<Instant> {
        match self {
            HookRunState::PendingReveal {
                reveal_deadline, ..
            } => Some(*reveal_deadline),
            HookRunState::QuietLinger {
                removal_deadline, ..
            } => Some(*removal_deadline),
            HookRunState::VisibleRunning { .. } | HookRunState::Completed { .. } => None,
        }
    }

    /// 静默成功停留足够长时间后返回 true。
    pub(super) fn quiet_linger_expired(&self, now: Instant) -> bool {
        match self {
            HookRunState::QuietLinger {
                removal_deadline, ..
            } => now >= *removal_deadline,
            HookRunState::PendingReveal { .. }
            | HookRunState::VisibleRunning { .. }
            | HookRunState::Completed { .. } => false,
        }
    }

    /// 将可见的静默成功转换为临时停留状态。
    ///
    /// 当成功状态应立即移除时返回 false：它可能从未可见，或
    /// 已达到最短可见时长。
    pub(super) fn complete_quiet_success(&mut self, now: Instant) -> bool {
        let HookRunState::VisibleRunning {
            start_time,
            visible_since,
            ..
        } = self
        else {
            return false;
        };
        let start_time = *start_time;
        let minimum_deadline = *visible_since + QUIET_HOOK_MIN_VISIBLE;
        if now >= minimum_deadline {
            return false;
        }
        *self = HookRunState::QuietLinger {
            start_time,
            removal_deadline: minimum_deadline,
        };
        true
    }
}

impl RunningHookGroup {
    pub(super) fn new(key: RunningHookGroupKey, start_time: Option<Instant>) -> Self {
        Self {
            key,
            start_time,
            count: 1,
        }
    }
}

/// 生成一行分组的运行中 hook 状态。
pub(super) fn push_running_hook_group(
    lines: &mut Vec<Line<'static>>,
    group: &RunningHookGroup,
    animations_enabled: bool,
) {
    push_hook_line_separator(lines);
    let label = hook_event_label(group.key.event_name);
    let hook_text = if group.count == 1 {
        format!("Running {label} hook")
    } else {
        format!("Running {} {label} hooks", group.count)
    };
    push_running_hook_header(
        lines,
        &hook_text,
        group.start_time,
        group.key.status_message.as_deref(),
        animations_enabled,
    );
}

/// 生成所有运行中 hook 行共用的动画或静态标题。
pub(super) fn push_running_hook_header(
    lines: &mut Vec<Line<'static>>,
    hook_text: &str,
    start_time: Option<Instant>,
    status_message: Option<&str>,
    animations_enabled: bool,
) {
    let mut header = Vec::new();
    let motion_mode = MotionMode::from_animations_enabled(animations_enabled);
    if let Some(indicator) =
        activity_indicator(start_time, motion_mode, ReducedMotionIndicator::Hidden)
    {
        header.push(indicator);
        header.push(" ".into());
    }
    header.extend(shimmer_text(hook_text, motion_mode));
    if !animations_enabled && let Some(span) = header.last_mut() {
        span.style = span.style.patch(Style::default().bold());
    }
    if let Some(status_message) = status_message
        && !status_message.is_empty()
    {
        header.push(": ".into());
        header.push(status_message.to_string().dim());
    }
    lines.push(header.into());
}

/// 在 hook 块之间添加空白分隔行，但不留下开头空行。
pub(super) fn push_hook_line_separator(lines: &mut Vec<Line<'static>>) {
    if !lines.is_empty() {
        lines.push("".into());
    }
}

/// 合并可选的 Instant，同时保留已知的最早开始时间。
pub(super) fn earliest_instant(left: Option<Instant>, right: Option<Instant>) -> Option<Instant> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(crate) fn new_active_hook_cell(run: HookRunSummary, animations_enabled: bool) -> HookCell {
    HookCell::new_active(run, animations_enabled)
}

pub(crate) fn new_completed_hook_cell(run: HookRunSummary, animations_enabled: bool) -> HookCell {
    HookCell::new_completed(run, animations_enabled)
}

/// 对应在历史记录中隐藏的已完成 hook 返回 true。
pub(super) fn hook_run_is_quiet_success(run: &HookRunSummary) -> bool {
    run.status == HookRunStatus::Completed && run.entries.is_empty()
}

pub(super) fn hook_completed_bullet(
    status: HookRunStatus,
    entries: &[HookOutputEntry],
) -> Span<'static> {
    match status {
        HookRunStatus::Completed => {
            if entries
                .iter()
                .any(|entry| entry.kind == HookOutputEntryKind::Warning)
            {
                "•".bold()
            } else {
                "•".green().bold()
            }
        }
        HookRunStatus::Blocked | HookRunStatus::Failed | HookRunStatus::Stopped => "•".red().bold(),
        HookRunStatus::Running => "•".into(),
    }
}

pub(super) fn hook_output_prefix(kind: HookOutputEntryKind) -> &'static str {
    match kind {
        HookOutputEntryKind::Warning => "warning: ",
        HookOutputEntryKind::Stop => "stop: ",
        HookOutputEntryKind::Feedback => "feedback: ",
        HookOutputEntryKind::Context => "hook context: ",
        HookOutputEntryKind::Error => "error: ",
    }
}

pub(super) fn hook_event_label(event_name: HookEventName) -> &'static str {
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
