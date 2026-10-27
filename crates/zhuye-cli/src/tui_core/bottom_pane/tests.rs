//! Bottom pane 的测试集。
//!
//! 从 bottom_pane/mod.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::app_server_protocol::CommandExecutionApprovalDecision;
use crate::tui_core::app::app_server_requests::ResolvedAppServerRequest;
use crate::tui_core::app_command::AppCommand as Op;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::status_indicator_widget::STATUS_DETAILS_DEFAULT_MAX_LINES;
use crate::tui_core::status_indicator_widget::StatusDetailsCapitalization;
use crate::tui_core::test_support::PathBufExt;
use crate::tui_core::test_support::test_path_buf;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use insta::assert_snapshot;
use pretty_assertions::assert_eq;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;
use tokio::sync::mpsc::unbounded_channel;

fn snapshot_buffer(buf: &Buffer) -> String {
    let mut lines = Vec::new();
    for y in 0..buf.area().height {
        let mut row = String::new();
        for x in 0..buf.area().width {
            row.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        lines.push(row);
    }
    lines.join("\n")
}

fn render_snapshot(pane: &BottomPane, area: Rect) -> String {
    let mut buf = Buffer::empty(area);
    pane.render(area, &mut buf);
    snapshot_buffer(&buf)
}

fn test_pane(app_event_tx: AppEventSender) -> BottomPane {
    test_pane_with_disable_paste_burst(app_event_tx, /*disable_paste_burst*/ false)
}

fn test_pane_with_disable_paste_burst(
    app_event_tx: AppEventSender,
    disable_paste_burst: bool,
) -> BottomPane {
    BottomPane::new(BottomPaneParams {
        app_event_tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst,
        animations_enabled: true,
        skills: Some(Vec::new()),
    })
}

fn exec_request() -> ApprovalRequest {
    ApprovalRequest::Exec(ExecApprovalRequest {
        thread_id: crate::protocol_compat::ThreadId::new(),
        thread_label: None,
        id: "1".to_string(),
        environment_id: None,
        command: vec!["echo".into(), "ok".into()],
        reason: None,
        available_decisions: vec![
            CommandExecutionApprovalDecision::Accept,
            CommandExecutionApprovalDecision::Cancel,
        ],
        network_approval_context: None,
        additional_permissions: None,
    })
}

#[derive(Default)]
struct DismissibleView {
    id: Option<&'static str>,
    dismiss_exec_id: Option<&'static str>,
    complete: bool,
}

impl Renderable for DismissibleView {
    fn render(&self, _area: Rect, _buf: &mut Buffer) {}

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

impl BottomPaneView for DismissibleView {
    fn is_complete(&self) -> bool {
        self.complete
    }

    fn view_id(&self) -> Option<&'static str> {
        self.id
    }

    fn dismiss_app_server_request(&mut self, request: &ResolvedAppServerRequest) -> bool {
        let ResolvedAppServerRequest::ExecApproval { id } = request else {
            return false;
        };
        if self.dismiss_exec_id != Some(id.as_str()) {
            return false;
        }

        self.complete = true;
        true
    }
}

#[derive(Default)]
struct CompletingView {
    id: Option<&'static str>,
    complete: bool,
}

impl Renderable for CompletingView {
    fn render(&self, _area: Rect, _buf: &mut Buffer) {}

    fn desired_height(&self, _width: u16) -> u16 {
        0
    }
}

impl BottomPaneView for CompletingView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.code == KeyCode::Enter {
            self.complete = true;
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn view_id(&self) -> Option<&'static str> {
        self.id
    }
}

#[test]
fn ctrl_c_on_modal_consumes_without_showing_quit_hint() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: true,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });
    pane.push_approval_request(exec_request(), &features);
    assert_eq!(CancellationEvent::Handled, pane.on_ctrl_c());
    assert!(!pane.quit_shortcut_hint_visible());
    assert_eq!(CancellationEvent::NotHandled, pane.on_ctrl_c());
}

#[test]
fn ctrl_c_cancels_history_search_without_clearing_draft_or_showing_quit_hint() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: true,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });
    pane.insert_str("draft");

    pane.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert!(pane.composer.popup_active());

    assert_eq!(CancellationEvent::Handled, pane.on_ctrl_c());
    assert_eq!(pane.composer_text(), "draft");
    assert!(!pane.composer.popup_active());
    assert!(!pane.quit_shortcut_hint_visible());
}

// live ring 已移除；相关测试已删除。

#[test]
fn overlay_not_shown_above_approval_modal() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    // 创建一个审批模态框（活动视图）。
    pane.push_approval_request(exec_request(), &features);

    // 渲染并验证首行不包含浮层。
    let area = Rect::new(0, 0, 60, 6);
    let mut buf = Buffer::empty(area);
    pane.render(area, &mut buf);

    let mut r0 = String::new();
    for x in 0..area.width {
        r0.push(buf[(x, 0)].symbol().chars().next().unwrap_or(' '));
    }
    assert!(
        !r0.contains("Working"),
        "overlay should not render above modal"
    );
}

#[test]
fn approval_request_shows_immediately_without_recent_typing() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = test_pane(tx);

    pane.push_approval_request(exec_request(), &features);

    assert_eq!(pane.view_stack.len(), 1);
    assert!(pane.delayed_approval_requests.is_empty());
}

#[test]
fn approval_request_is_delayed_after_recent_typing() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = test_pane(tx);
    let now = Instant::now();
    pane.last_composer_activity_at = Some(now);

    pane.push_approval_request(exec_request(), &features);

    assert!(pane.view_stack.is_empty());
    assert_eq!(pane.delayed_approval_requests.len(), 1);

    pane.pre_draw_tick_at(
        now + APPROVAL_PROMPT_TYPING_IDLE_DELAY - Duration::from_millis(/*millis*/ 1),
    );
    assert!(pane.view_stack.is_empty());
    assert_eq!(pane.delayed_approval_requests.len(), 1);

    pane.pre_draw_tick_at(now + APPROVAL_PROMPT_TYPING_IDLE_DELAY);
    assert_eq!(pane.view_stack.len(), 1);
    assert!(pane.delayed_approval_requests.is_empty());
}

#[test]
fn continued_typing_resets_delayed_approval_idle_deadline() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = test_pane(tx);
    let first_activity = Instant::now();
    pane.last_composer_activity_at = Some(first_activity);
    pane.push_approval_request(exec_request(), &features);

    let continued_activity = first_activity + Duration::from_millis(/*millis*/ 750);
    pane.record_composer_activity_at(continued_activity);

    pane.pre_draw_tick_at(first_activity + APPROVAL_PROMPT_TYPING_IDLE_DELAY);
    assert!(pane.view_stack.is_empty());
    assert_eq!(pane.delayed_approval_requests.len(), 1);

    pane.pre_draw_tick_at(continued_activity + APPROVAL_PROMPT_TYPING_IDLE_DELAY);
    assert_eq!(pane.view_stack.len(), 1);
    assert!(pane.delayed_approval_requests.is_empty());
}

#[test]
fn typed_approval_shortcuts_during_delay_stay_in_composer() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = test_pane_with_disable_paste_burst(tx, /*disable_paste_burst*/ true);
    pane.last_composer_activity_at = Some(Instant::now());
    pane.push_approval_request(exec_request(), &features);

    pane.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    pane.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));

    assert_eq!(pane.composer_text(), "ya");
    assert!(pane.view_stack.is_empty());
    assert_eq!(pane.delayed_approval_requests.len(), 1);
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, AppEvent::SubmitThreadOp { .. }),
            "delayed approval shortcut should not submit an approval: {event:?}"
        );
    }
}

#[test]
fn delayed_approval_shortcut_works_after_idle_deadline() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = test_pane(tx);
    let now = Instant::now();
    pane.last_composer_activity_at = Some(now);
    pane.push_approval_request(exec_request(), &features);

    pane.pre_draw_tick_at(now + APPROVAL_PROMPT_TYPING_IDLE_DELAY);
    pane.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

    let mut approval_decision = None;
    while let Ok(event) = rx.try_recv() {
        if let AppEvent::SubmitThreadOp {
            op: Op::ExecApproval { decision, .. },
            ..
        } = event
        {
            approval_decision = Some(decision);
        }
    }
    assert_eq!(
        approval_decision,
        Some(CommandExecutionApprovalDecision::Accept)
    );
}

#[test]
fn dismiss_app_server_request_prunes_delayed_approval() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = test_pane(tx);
    let now = Instant::now();
    pane.last_composer_activity_at = Some(now);
    pane.push_approval_request(exec_request(), &features);

    assert!(
        pane.dismiss_app_server_request(&ResolvedAppServerRequest::ExecApproval {
            id: "1".to_string(),
        })
    );
    assert!(pane.delayed_approval_requests.is_empty());

    pane.pre_draw_tick_at(now + APPROVAL_PROMPT_TYPING_IDLE_DELAY);
    assert!(pane.view_stack.is_empty());
}

#[test]
fn dismiss_app_server_request_removes_matching_buried_view() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = test_pane(tx);

    pane.push_view(Box::new(DismissibleView {
        id: Some("buried"),
        dismiss_exec_id: Some("request-1"),
        complete: false,
    }));
    pane.push_view(Box::new(DismissibleView {
        id: Some("top"),
        dismiss_exec_id: None,
        complete: false,
    }));

    assert!(
        pane.dismiss_app_server_request(&ResolvedAppServerRequest::ExecApproval {
            id: "request-1".to_string(),
        })
    );
    assert_eq!(pane.view_stack.len(), 1);
    assert_eq!(
        pane.view_stack.last().and_then(|view| view.view_id()),
        Some("top")
    );
}

#[test]
fn dismiss_app_server_request_returns_false_when_no_view_matches() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = test_pane(tx);

    pane.push_view(Box::new(DismissibleView {
        id: Some("first"),
        dismiss_exec_id: Some("other-request"),
        complete: false,
    }));
    pane.push_view(Box::new(DismissibleView {
        id: Some("second"),
        dismiss_exec_id: None,
        complete: false,
    }));

    assert!(
        !pane.dismiss_app_server_request(&ResolvedAppServerRequest::ExecApproval {
            id: "request-1".to_string(),
        })
    );
    assert_eq!(pane.view_stack.len(), 2);
    assert_eq!(
        pane.view_stack.last().and_then(|view| view.view_id()),
        Some("second")
    );
}

#[test]
fn completing_top_view_preserves_underlying_view() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = test_pane(tx);

    pane.push_view(Box::new(DismissibleView {
        id: Some("underlying"),
        dismiss_exec_id: None,
        complete: false,
    }));
    pane.push_view(Box::new(CompletingView {
        id: Some("top"),
        complete: false,
    }));

    pane.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(pane.view_stack.len(), 1);
    assert_eq!(
        pane.view_stack.last().and_then(|view| view.view_id()),
        Some("underlying")
    );
}

#[test]
fn composer_shown_after_denied_while_task_running() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let features = Features::with_defaults();
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    // 启动一个运行中的任务，使状态指示器在输入框上方处于激活状态。
    pane.set_task_running(/*running*/ true);

    // 推入一个审批模态框（如命令审批），它应隐藏状态视图。
    pane.push_approval_request(exec_request(), &features);

    // 模拟在模态框上按下 'n'（否）。
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    pane.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

    // 拒绝后，由于任务仍在运行，状态指示器应
    // 在输入框上方可见。模态框应已消失。
    assert!(
        pane.view_stack.is_empty(),
        "no active modal view after denial"
    );

    // 渲染并确保首行包含 Working 标题，且其下方有一行输入框。
    // 给动画线程一点时间完成一次跳动。
    std::thread::sleep(Duration::from_millis(120));
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    pane.render(area, &mut buf);
    let mut row0 = String::new();
    for x in 0..area.width {
        row0.push(buf[(x, 0)].symbol().chars().next().unwrap_or(' '));
    }
    assert!(
        row0.contains("Working"),
        "expected Working header after denial on row 0: {row0:?}"
    );

    // 输入框占位符应在其下方某处可见。
    let mut found_composer = false;
    for y in 1..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            row.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        if row.contains("Ask Reflect") {
            found_composer = true;
            break;
        }
    }
    assert!(
        found_composer,
        "expected composer visible under status line"
    );
}

#[test]
fn status_indicator_visible_during_command_execution() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    // 开始一个任务：显示初始状态。
    pane.set_task_running(/*running*/ true);

    // 使用一个能让状态行在输入框上方可见的高度。
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    pane.render(area, &mut buf);

    let bufs = snapshot_buffer(&buf);
    assert!(bufs.contains("• Working"), "expected Working header");
}

#[test]
fn status_and_composer_fill_height_without_bottom_padding() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    // 激活 spinner（状态视图替换输入框），且没有 live ring。
    pane.set_task_running(/*running*/ true);

    // 使用 height == desired_height；期望为间隔行 + 状态行 + 输入框行，且没有尾部填充。
    let height = pane.desired_height(/*width*/ 30);
    assert!(
        height >= 3,
        "expected at least 3 rows to render spacer, status, and composer; got {height}"
    );
    let area = Rect::new(0, 0, 30, height);
    assert_snapshot!(
        "status_and_composer_fill_height_without_bottom_padding",
        render_snapshot(&pane, area)
    );
}

#[test]
fn status_only_snapshot() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);

    let width = 48;
    let height = pane.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    assert_snapshot!("status_only_snapshot", render_snapshot(&pane, area));
}

#[test]
fn unified_exec_summary_does_not_increase_height_when_status_visible() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);
    let width = 120;
    let before = pane.desired_height(width);

    pane.set_unified_exec_processes(vec!["sleep 5".to_string()]);
    let after = pane.desired_height(width);

    assert_eq!(after, before);

    let area = Rect::new(0, 0, width, after);
    let rendered = render_snapshot(&pane, area);
    assert!(rendered.contains("background terminal running · /ps to view"));
}

#[test]
fn status_with_details_and_queued_messages_snapshot() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);
    pane.update_status(
        "Working".to_string(),
        Some("First detail line\nSecond detail line".to_string()),
        StatusDetailsCapitalization::CapitalizeFirst,
        STATUS_DETAILS_DEFAULT_MAX_LINES,
    );
    pane.set_pending_input_preview(
        vec!["Queued follow-up question".to_string()],
        Vec::new(),
        Vec::new(),
    );

    let width = 48;
    let height = pane.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    assert_snapshot!(
        "status_with_details_and_queued_messages_snapshot",
        render_snapshot(&pane, area)
    );
}

#[test]
fn queued_messages_visible_when_status_hidden_snapshot() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);
    pane.set_pending_input_preview(
        vec!["Queued follow-up question".to_string()],
        Vec::new(),
        Vec::new(),
    );
    pane.hide_status_indicator();

    let width = 48;
    let height = pane.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    assert_snapshot!(
        "queued_messages_visible_when_status_hidden_snapshot",
        render_snapshot(&pane, area)
    );
}

#[test]
fn status_and_queued_messages_snapshot() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);
    pane.set_pending_input_preview(
        vec!["Queued follow-up question".to_string()],
        Vec::new(),
        Vec::new(),
    );

    let width = 48;
    let height = pane.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    assert_snapshot!(
        "status_and_queued_messages_snapshot",
        render_snapshot(&pane, area)
    );
}

#[test]
fn remote_images_render_above_composer_text() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_remote_image_urls(vec![
        "https://example.com/one.png".to_string(),
        "data:image/png;base64,aGVsbG8=".to_string(),
    ]);

    assert_eq!(pane.composer_text(), "");
    let width = 48;
    let height = pane.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    let snapshot = render_snapshot(&pane, area);
    assert!(snapshot.contains("[Image #1]"));
    assert!(snapshot.contains("[Image #2]"));
}

#[test]
fn drain_pending_submission_state_clears_remote_image_urls() {
    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_remote_image_urls(vec!["https://example.com/one.png".to_string()]);
    assert_eq!(pane.remote_image_urls().len(), 1);

    pane.drain_pending_submission_state();

    assert!(pane.remote_image_urls().is_empty());
}

#[test]
fn esc_with_skill_popup_does_not_interrupt_task() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(vec![SkillMetadata {
            name: "test-skill".to_string(),
            description: "test skill".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            path: test_path_buf("/tmp/test-skill/SKILL.md").abs(),
            scope: crate::tui_core::test_support::skill_scope_user(),
            enabled: true,
        }]),
    });

    pane.set_task_running(/*running*/ true);

    // 复现：运行中的任务 + 技能弹窗 + Esc 应关闭弹窗，而不是中断任务。
    pane.insert_str("$");
    assert!(
        pane.composer.popup_active(),
        "expected skill popup after typing `$`"
    );

    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, AppEvent::ReflectOp(Op::Interrupt)),
            "expected Esc to not send Op::Interrupt when dismissing skill popup"
        );
    }
    assert!(
        !pane.composer.popup_active(),
        "expected Esc to dismiss skill popup"
    );
}

#[test]
fn esc_dismisses_slash_command_popup_without_interrupting_task() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);

    // 复现：运行中的任务 + 斜杠命令弹窗 + Esc 应关闭弹窗，
    // 且不中断任务。
    pane.insert_str("/rev");
    assert!(
        pane.composer.popup_active(),
        "expected command popup after typing `/rev`"
    );

    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, AppEvent::ReflectOp(Op::Interrupt)),
            "expected Esc to not send Op::Interrupt while command popup is active"
        );
    }
    assert!(!pane.composer.popup_active());
    assert_eq!(pane.composer_text(), "/rev");

    let width = 60;
    let area = Rect::new(0, 0, width, pane.desired_height(width));
    assert_snapshot!(
        "slash_command_popup_dismissed",
        render_snapshot(&pane, area)
    );

    pane.insert_str("i");
    assert!(pane.composer.popup_active());
}

#[test]
fn esc_with_agent_command_without_popup_does_not_interrupt_task() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);

    // 复现：`/agent ` 会隐藏弹窗（光标越过命令名）。Esc 应
    // 保持编辑命令文本，而不是中断正在运行的任务。
    pane.insert_str("/agent ");
    assert!(
        !pane.composer.popup_active(),
        "expected command popup to be hidden after entering `/agent `"
    );

    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, AppEvent::ReflectOp(Op::Interrupt)),
            "expected Esc to not send Op::Interrupt while typing `/agent`"
        );
    }
    assert_eq!(pane.composer_text(), "/agent ");
}

#[test]
fn esc_release_after_dismissing_agent_picker_does_not_interrupt_task() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);
    pane.show_selection_view(SelectionViewParams {
        title: Some("Agents".to_string()),
        items: vec![SelectionItem {
            name: "Main".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    });

    pane.handle_key_event(KeyEvent::new_with_kind(
        KeyCode::Esc,
        KeyModifiers::NONE,
        KeyEventKind::Press,
    ));
    pane.handle_key_event(KeyEvent::new_with_kind(
        KeyCode::Esc,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ));

    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, AppEvent::ReflectOp(Op::Interrupt)),
            "expected Esc release after dismissing agent picker to not interrupt"
        );
    }
    assert!(
        pane.no_modal_or_popup_active(),
        "expected Esc press to dismiss the agent picker"
    );
}

#[test]
fn esc_interrupts_running_task_when_no_popup() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_task_running(/*running*/ true);

    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(
        matches!(rx.try_recv(), Ok(AppEvent::ReflectOp(Op::Interrupt))),
        "expected Esc to send Op::Interrupt while a task is running"
    );
}

#[test]
fn remapped_interrupt_turn_uses_configured_key_including_agent_drafts() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = test_pane(tx);
    let mut keymap = RuntimeKeymap::defaults();
    keymap.chat.interrupt_turn = vec![crate::tui_core::key_hint::plain(KeyCode::F(12))];
    pane.set_keymap_bindings(&keymap);
    pane.set_task_running(/*running*/ true);
    pane.insert_str("/agent ");

    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        rx.try_recv().is_err(),
        "expected Esc to remain local after remapping interruption"
    );

    pane.handle_key_event(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE));
    assert!(
        matches!(rx.try_recv(), Ok(AppEvent::ReflectOp(Op::Interrupt))),
        "expected configured key to interrupt while `/agent` is being edited"
    );
}

#[test]
fn selection_view_esc_respects_remapped_list_cancel() {
    let (tx_raw, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = test_pane(tx);
    let mut keymap = RuntimeKeymap::defaults();
    keymap.list.cancel = vec![crate::tui_core::key_hint::plain(KeyCode::Char('q'))];
    pane.set_keymap_bindings(&keymap);
    pane.show_selection_view(SelectionViewParams {
        title: Some("Agents".to_string()),
        items: vec![SelectionItem {
            name: "Main".to_string(),
            ..Default::default()
        }],
        on_cancel: Some(Box::new(|tx: &_| {
            tx.send(AppEvent::OpenApprovalsPopup);
        })),
        ..Default::default()
    });

    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(pane.active_view().is_some());
    assert!(rx.try_recv().is_err());

    pane.handle_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));

    assert!(pane.no_modal_or_popup_active());
    assert!(matches!(rx.try_recv(), Ok(AppEvent::OpenApprovalsPopup)));
}

#[test]
fn esc_routes_to_handle_key_event_when_requested() {
    #[derive(Default)]
    struct EscRoutingView {
        on_ctrl_c_calls: Rc<Cell<usize>>,
        handle_calls: Rc<Cell<usize>>,
    }

    impl Renderable for EscRoutingView {
        fn render(&self, _area: Rect, _buf: &mut Buffer) {}

        fn desired_height(&self, _width: u16) -> u16 {
            0
        }
    }

    impl BottomPaneView for EscRoutingView {
        fn handle_key_event(&mut self, _key_event: KeyEvent) {
            self.handle_calls
                .set(self.handle_calls.get().saturating_add(1));
        }

        fn on_ctrl_c(&mut self) -> CancellationEvent {
            self.on_ctrl_c_calls
                .set(self.on_ctrl_c_calls.get().saturating_add(1));
            CancellationEvent::Handled
        }

        fn prefer_esc_to_handle_key_event(&self) -> bool {
            true
        }
    }

    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    let on_ctrl_c_calls = Rc::new(Cell::new(0));
    let handle_calls = Rc::new(Cell::new(0));
    pane.push_view(Box::new(EscRoutingView {
        on_ctrl_c_calls: Rc::clone(&on_ctrl_c_calls),
        handle_calls: Rc::clone(&handle_calls),
    }));

    pane.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(on_ctrl_c_calls.get(), 0);
    assert_eq!(handle_calls.get(), 1);
}

#[test]
fn release_events_are_ignored_for_active_view() {
    #[derive(Default)]
    struct CountingView {
        handle_calls: Rc<Cell<usize>>,
    }

    impl Renderable for CountingView {
        fn render(&self, _area: Rect, _buf: &mut Buffer) {}

        fn desired_height(&self, _width: u16) -> u16 {
            0
        }
    }

    impl BottomPaneView for CountingView {
        fn handle_key_event(&mut self, _key_event: KeyEvent) {
            self.handle_calls
                .set(self.handle_calls.get().saturating_add(1));
        }
    }

    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    let handle_calls = Rc::new(Cell::new(0));
    pane.push_view(Box::new(CountingView {
        handle_calls: Rc::clone(&handle_calls),
    }));

    pane.handle_key_event(KeyEvent::new_with_kind(
        KeyCode::Down,
        KeyModifiers::NONE,
        KeyEventKind::Press,
    ));
    pane.handle_key_event(KeyEvent::new_with_kind(
        KeyCode::Down,
        KeyModifiers::NONE,
        KeyEventKind::Release,
    ));

    assert_eq!(handle_calls.get(), 1);
}

#[test]
fn paste_completion_clears_stacked_views_and_restores_composer_input() {
    #[derive(Default)]
    struct BlockingView {
        handle_calls: Rc<Cell<usize>>,
    }

    impl Renderable for BlockingView {
        fn render(&self, _area: Rect, _buf: &mut Buffer) {}

        fn desired_height(&self, _width: u16) -> u16 {
            0
        }
    }

    impl BottomPaneView for BlockingView {
        fn handle_key_event(&mut self, _key_event: KeyEvent) {
            self.handle_calls
                .set(self.handle_calls.get().saturating_add(1));
        }
    }

    #[derive(Default)]
    struct PasteCompletesView {
        complete: bool,
    }

    impl Renderable for PasteCompletesView {
        fn render(&self, _area: Rect, _buf: &mut Buffer) {}

        fn desired_height(&self, _width: u16) -> u16 {
            0
        }
    }

    impl BottomPaneView for PasteCompletesView {
        fn handle_paste(&mut self, _pasted: String) -> bool {
            self.complete = true;
            true
        }

        fn is_complete(&self) -> bool {
            self.complete
        }
    }

    let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut pane = BottomPane::new(BottomPaneParams {
        app_event_tx: tx,
        frame_requester: FrameRequester::test_dummy(),
        has_input_focus: true,
        enhanced_keys_supported: false,
        placeholder_text: "Ask Reflect to do anything".to_string(),
        disable_paste_burst: false,
        animations_enabled: true,
        skills: Some(Vec::new()),
    });

    pane.set_composer_input_enabled(/*enabled*/ false, /*placeholder*/ None);

    let lower_view_handle_calls = Rc::new(Cell::new(0));
    pane.push_view(Box::new(BlockingView {
        handle_calls: Rc::clone(&lower_view_handle_calls),
    }));
    pane.push_view(Box::new(PasteCompletesView::default()));

    pane.handle_paste("hello".to_string());

    assert!(
        pane.view_stack.is_empty(),
        "paste completion should tear down the active modal flow"
    );

    pane.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    let area = Rect::new(0, 0, 40, pane.desired_height(/*width*/ 40).max(2));
    assert!(pane.cursor_pos(area).is_some());
    assert_eq!(lower_view_handle_calls.get(), 0);
}
