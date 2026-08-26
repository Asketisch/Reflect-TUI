//! 反馈视图（feedback_view） 的测试集。
//!
//! 从 bottom_pane/feedback_view.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::feedback::FeedbackDiagnostic;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event_sender::AppEventSender;
use pretty_assertions::assert_eq;

fn render(view: &FeedbackNoteView, width: u16) -> String {
    let height = view.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);
    render_buffer(area, &buf)
}

fn render_renderable(renderable: &dyn Renderable, width: u16) -> String {
    let height = renderable.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    renderable.render(area, &mut buf);
    render_buffer(area, &buf)
}

fn render_buffer(area: Rect, buf: &Buffer) -> String {
    let mut lines: Vec<String> = (0..area.height)
        .map(|row| {
            let mut line = String::new();
            for col in 0..area.width {
                let symbol = buf[(area.x + col, area.y + row)].symbol();
                if symbol.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(symbol);
                }
            }
            line.trim_end().to_string()
        })
        .collect();

    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn render_cell(cell: &impl history_cell::HistoryCell, width: u16) -> String {
    cell.display_lines(width)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn make_view(category: FeedbackCategory) -> FeedbackNoteView {
    let (tx_raw, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    FeedbackNoteView::new(
        category, /*turn_id*/ None, tx, /*include_logs*/ true,
    )
}

#[test]
fn feedback_view_bad_result() {
    let view = make_view(FeedbackCategory::BadResult);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_bad_result", rendered);
}

#[test]
fn feedback_view_good_result() {
    let view = make_view(FeedbackCategory::GoodResult);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_good_result", rendered);
}

#[test]
fn feedback_view_bug() {
    let view = make_view(FeedbackCategory::Bug);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_bug", rendered);
}

#[test]
fn feedback_view_other() {
    let view = make_view(FeedbackCategory::Other);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_other", rendered);
}

#[test]
fn feedback_view_safety_check() {
    let view = make_view(FeedbackCategory::SafetyCheck);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_safety_check", rendered);
}

#[test]
fn feedback_view_with_connectivity_diagnostics() {
    let (tx_raw, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let view = FeedbackNoteView::new(
        FeedbackCategory::Bug,
        /*turn_id*/ None,
        tx,
        /*include_logs*/ false,
    );
    let rendered = render(&view, /*width*/ 60);

    insta::assert_snapshot!("feedback_view_with_connectivity_diagnostics", rendered);
}

#[test]
fn feedback_upload_consent_lists_doctor_report() {
    let (tx_raw, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let params = feedback_upload_consent_params(
        tx,
        FeedbackCategory::Bug,
        Some(std::path::PathBuf::from("rollout.jsonl")),
        Some("auto-review-rollout.jsonl".to_string()),
        /*include_windows_sandbox_log*/ false,
        &FeedbackDiagnostics::default(),
    );

    let rendered = render_renderable(params.header.as_ref(), /*width*/ 60);

    insta::assert_snapshot!("feedback_upload_consent_lists_doctor_report", rendered);
}

#[test]
fn feedback_upload_consent_lists_windows_sandbox_log_when_included() {
    let (tx_raw, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let params = feedback_upload_consent_params(
        tx,
        FeedbackCategory::Bug,
        Some(std::path::PathBuf::from("rollout.jsonl")),
        Some("auto-review-rollout.jsonl".to_string()),
        /*include_windows_sandbox_log*/ true,
        &FeedbackDiagnostics::default(),
    );

    let rendered = render_renderable(params.header.as_ref(), /*width*/ 60);

    insta::assert_snapshot!(
        "feedback_upload_consent_lists_windows_sandbox_log_when_included",
        rendered
    );
}

#[test]
fn submit_feedback_emits_submit_event_with_trimmed_note() {
    let (tx_raw, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut view = FeedbackNoteView::new(
        FeedbackCategory::Bug,
        Some("turn-123".to_string()),
        tx,
        /*include_logs*/ true,
    );
    view.textarea.insert_str("  something broke  ");

    view.submit();

    let event = rx.try_recv().expect("submit feedback event");
    assert!(matches!(
        event,
        AppEvent::SubmitFeedback {
            category: FeedbackCategory::Bug,
            reason: Some(reason),
            turn_id: Some(turn_id),
            include_logs: true,
        } if reason == "something broke" && turn_id == "turn-123"
    ));
    assert_eq!(view.is_complete(), true);
}

#[test]
fn submit_feedback_omits_empty_note() {
    let (tx_raw, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut view = FeedbackNoteView::new(
        FeedbackCategory::GoodResult,
        /*turn_id*/ None,
        tx,
        /*include_logs*/ false,
    );

    view.submit();

    let event = rx.try_recv().expect("submit feedback event");
    assert!(matches!(
        event,
        AppEvent::SubmitFeedback {
            category: FeedbackCategory::GoodResult,
            reason: None,
            turn_id: None,
            include_logs: false,
        }
    ));
}

#[test]
fn should_show_feedback_connectivity_details_only_for_non_good_result_with_diagnostics() {
    let diagnostics = FeedbackDiagnostics::new(vec![FeedbackDiagnostic {
        headline: "Proxy environment variables are set and may affect connectivity.".to_string(),
        details: vec!["HTTP_PROXY = http://proxy.example.com:8080".to_string()],
    }]);

    assert_eq!(
        should_show_feedback_connectivity_details(FeedbackCategory::Bug, &diagnostics),
        true
    );
    assert_eq!(
        should_show_feedback_connectivity_details(FeedbackCategory::GoodResult, &diagnostics),
        false
    );
    assert_eq!(
        should_show_feedback_connectivity_details(
            FeedbackCategory::BadResult,
            &FeedbackDiagnostics::default()
        ),
        false
    );
}

#[test]
fn issue_url_available_for_bug_bad_result_safety_check_and_other() {
    let bug_url = issue_url_for_category(
        FeedbackCategory::Bug,
        "thread-1",
        FeedbackAudience::OpenAiEmployee,
    );
    let expected_slack_url = "http://go/reflect-feedback-internal".to_string();
    assert_eq!(bug_url.as_deref(), Some(expected_slack_url.as_str()));

    let bad_result_url = issue_url_for_category(
        FeedbackCategory::BadResult,
        "thread-2",
        FeedbackAudience::OpenAiEmployee,
    );
    assert!(bad_result_url.is_some());

    let other_url = issue_url_for_category(
        FeedbackCategory::Other,
        "thread-3",
        FeedbackAudience::OpenAiEmployee,
    );
    assert!(other_url.is_some());

    let safety_check_url = issue_url_for_category(
        FeedbackCategory::SafetyCheck,
        "thread-4",
        FeedbackAudience::OpenAiEmployee,
    );
    assert!(safety_check_url.is_some());

    assert!(
        issue_url_for_category(
            FeedbackCategory::GoodResult,
            "t",
            FeedbackAudience::OpenAiEmployee
        )
        .is_none()
    );
    let bug_url_non_employee =
        issue_url_for_category(FeedbackCategory::Bug, "t", FeedbackAudience::External);
    let expected_external_url = "https://github.com/asketisch/reflect/issues/new?template=3-cli.yml&steps=Uploaded%20thread:%20t";
    assert_eq!(bug_url_non_employee.as_deref(), Some(expected_external_url));
}

#[test]
fn feedback_success_cell_matches_external_bug_copy() {
    let rendered = render_cell(
        &feedback_success_cell(
            FeedbackCategory::Bug,
            /*include_logs*/ true,
            "thread-1",
            FeedbackAudience::External,
        ),
        /*width*/ 120,
    );
    assert_eq!(
        rendered,
        "• Feedback uploaded. Please open an issue using the following URL:\n\n  https://github.com/asketisch/reflect/issues/new?template=3-cli.yml&steps=Uploaded%20thread:%20thread-1\n\n  Or mention your thread ID thread-1 in an existing issue."
    );
}

#[test]
fn feedback_success_cell_matches_employee_bug_copy() {
    let rendered = render_cell(
        &feedback_success_cell(
            FeedbackCategory::Bug,
            /*include_logs*/ true,
            "thread-2",
            FeedbackAudience::OpenAiEmployee,
        ),
        /*width*/ 120,
    );
    assert_eq!(
        rendered,
        "• Feedback uploaded. Please report this in #reflect-feedback:\n\n  http://go/reflect-feedback-internal\n\n  Share this and add some info about your problem:\n    https://go/reflect-feedback/thread-2"
    );
}

#[test]
fn feedback_success_cell_matches_good_result_copy() {
    let rendered = render_cell(
        &feedback_success_cell(
            FeedbackCategory::GoodResult,
            /*include_logs*/ false,
            "thread-3",
            FeedbackAudience::External,
        ),
        /*width*/ 120,
    );
    assert_eq!(
        rendered,
        "• Feedback recorded (no logs). Thanks for the feedback!\n\n  Thread ID: thread-3"
    );
}

#[test]
fn feedback_success_cell_uses_issue_links_for_remaining_categories() {
    for category in [
        FeedbackCategory::BadResult,
        FeedbackCategory::SafetyCheck,
        FeedbackCategory::Other,
    ] {
        let rendered = render_cell(
            &feedback_success_cell(
                category,
                /*include_logs*/ false,
                "thread-4",
                FeedbackAudience::External,
            ),
            /*width*/ 120,
        );
        assert!(rendered.contains("Please open an issue using the following URL:"));
        assert!(rendered.contains("thread-4"));
    }
}
