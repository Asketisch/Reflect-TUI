//! Hook 历史单元 的测试集。
//!
//! 从 history_cell/hook_cell.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::tui_core::test_support::PathBufExt;
use crate::tui_core::test_support::test_path_buf;
use pretty_assertions::assert_eq;
use ratatui::style::Modifier;

#[test]
fn completed_hook_with_warning_uses_default_bold_bullet() {
    let entries = vec![HookOutputEntry {
        kind: HookOutputEntryKind::Warning,
        text: "Heads up from the hook".to_string(),
    }];

    let bullet = hook_completed_bullet(HookRunStatus::Completed, &entries);

    assert_eq!(bullet.content.as_ref(), "•");
    assert_eq!(bullet.style.fg, None);
    assert!(bullet.style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn completed_hook_short_multiline_context_preserves_display_transcript_and_raw_lines() {
    let cell = completed_hook_cell(
        HookEventName::SessionStart,
        HookRunStatus::Completed,
        vec![HookOutputEntry {
            kind: HookOutputEntryKind::Context,
            text: "## Working Memory Recall\n\nSource: Reflect compaction".to_string(),
        }],
    );
    let expected = vec![
        "• SessionStart hook (completed)".to_string(),
        "  hook context: ## Working Memory Recall".to_string(),
        "".to_string(),
        "    Source: Reflect compaction".to_string(),
    ];

    assert_eq!(line_texts(&cell.display_lines(/*width*/ 80)), expected);
    assert_eq!(line_texts(&cell.transcript_lines(/*width*/ 80)), expected);
    assert_eq!(line_texts(&cell.raw_lines()), expected);
}

#[test]
fn completed_hook_long_single_line_context_is_truncated_only_in_display() {
    let full_context = format!(
        "{}tail-marker",
        "context words that should wrap across the terminal width ".repeat(8)
    );
    let cell = completed_hook_cell(
        HookEventName::SessionStart,
        HookRunStatus::Completed,
        vec![HookOutputEntry {
            kind: HookOutputEntryKind::Context,
            text: full_context.clone(),
        }],
    );

    let display_lines = cell.display_lines(/*width*/ 80);
    let display = line_texts(&display_lines);
    assert_eq!(display.len(), 4);
    assert_eq!(
        Paragraph::new(Text::from(display_lines[1..].to_vec()))
            .wrap(Wrap { trim: false })
            .line_count(/*width*/ 80),
        HOOK_CONTEXT_MAX_DISPLAY_ROWS
    );
    assert!(
        display
            .iter()
            .any(|line| line.contains("ctrl + t to view transcript")),
        "expected truncated context to advertise the transcript: {display:?}"
    );
    assert!(display.iter().all(|line| !line.contains("tail-marker")));

    let expected_full = vec![
        "• SessionStart hook (completed)".to_string(),
        format!("  hook context: {full_context}"),
    ];
    assert_eq!(
        line_texts(&cell.transcript_lines(/*width*/ 80)),
        expected_full
    );
    assert_eq!(line_texts(&cell.raw_lines()), expected_full);
}

#[test]
fn completed_hook_non_context_entries_are_not_truncated() {
    for kind in [
        HookOutputEntryKind::Warning,
        HookOutputEntryKind::Stop,
        HookOutputEntryKind::Feedback,
        HookOutputEntryKind::Error,
    ] {
        let cell = completed_hook_cell(
            HookEventName::UserPromptSubmit,
            HookRunStatus::Stopped,
            vec![HookOutputEntry {
                kind,
                text: "first\nsecond\nthird\nfourth\nfifth".to_string(),
            }],
        );

        let display = line_texts(&cell.display_lines(/*width*/ 20));
        assert!(
            display.iter().any(|line| line == "    fifth"),
            "expected {kind:?} output to remain complete: {display:?}"
        );
        assert!(
            display
                .iter()
                .all(|line| !line.contains("ctrl + t to view transcript")),
            "did not expect a transcript hint for {kind:?}: {display:?}"
        );
    }
}

#[test]
fn completed_stop_hook_multiline_system_message_prefixes_first_line_only() {
    let cell = completed_hook_cell(
        HookEventName::Stop,
        HookRunStatus::Completed,
        vec![HookOutputEntry {
            kind: HookOutputEntryKind::Warning,
            text: "Heads up\nReview generated files".to_string(),
        }],
    );

    assert_eq!(
        line_texts(&cell.display_lines(/*width*/ 80)),
        vec![
            "• Stop (completed) says: Heads up".to_string(),
            "    Review generated files".to_string(),
        ]
    );
}

#[test]
fn pending_hook_does_not_animate_transcript() {
    let cell = HookCell::new_active(hook_run_summary("hook-1"), /*animations_enabled*/ true);

    assert_eq!(cell.transcript_animation_tick(), None);
}

#[test]
fn visible_hook_animates_transcript_when_animations_enabled() {
    let mut cell =
        HookCell::new_active(hook_run_summary("hook-1"), /*animations_enabled*/ true);
    cell.reveal_running_runs_now_for_test();
    cell.advance_time(Instant::now());

    assert_eq!(cell.transcript_animation_tick(), Some(0));
}

#[test]
fn visible_hook_does_not_animate_transcript_when_animations_disabled() {
    let mut cell = HookCell::new_active(
        hook_run_summary("hook-1"),
        /*animations_enabled*/ false,
    );
    cell.reveal_running_runs_now_for_test();
    cell.advance_time(Instant::now());

    assert_eq!(cell.transcript_animation_tick(), None);
}

#[test]
fn visible_hook_without_animations_omits_spinner() {
    let mut cell = HookCell::new_active(
        hook_run_summary("hook-1"),
        /*animations_enabled*/ false,
    );
    cell.reveal_running_runs_now_for_test();
    cell.advance_time(Instant::now());

    let rendered: Vec<String> = cell
        .display_lines(/*width*/ 80)
        .iter()
        .map(line_text)
        .collect();

    assert_eq!(
        rendered,
        vec!["Running PostToolUse hook: checking output policy".to_string()]
    );
}

fn completed_hook_cell(
    event_name: HookEventName,
    status: HookRunStatus,
    entries: Vec<HookOutputEntry>,
) -> HookCell {
    let mut run = hook_run_summary("hook-1");
    run.event_name = event_name;
    run.status = status;
    run.status_message = None;
    run.completed_at = Some(2);
    run.duration_ms = Some(1);
    run.entries = entries;
    HookCell::new_completed(run, /*animations_enabled*/ false)
}

fn line_texts(lines: &[Line<'_>]) -> Vec<String> {
    lines.iter().map(line_text).collect()
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn hook_run_summary(id: &str) -> HookRunSummary {
    HookRunSummary {
        id: id.to_string(),
        event_name: HookEventName::PostToolUse,
        handler_type: crate::app_server_protocol::HookHandlerType::Command,
        execution_mode: crate::app_server_protocol::HookExecutionMode::Sync,
        scope: crate::app_server_protocol::HookScope::Turn,
        source_path: test_path_buf("/tmp/hooks.json").abs(),
        source: crate::app_server_protocol::HookSource::User,
        display_order: 0,
        status: HookRunStatus::Running,
        status_message: Some("checking output policy".to_string()),
        started_at: 1,
        completed_at: None,
        duration_ms: None,
        entries: Vec::new(),
    }
}
