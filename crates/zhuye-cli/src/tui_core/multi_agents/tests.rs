//! 多 agent（multi_agents） 的测试集。
//!
//! 从 multi_agents.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::tui_core::history_cell::HistoryCell;
#[cfg(target_os = "macos")]
use crossterm::event::KeyEvent;
#[cfg(target_os = "macos")]
use crossterm::event::KeyModifiers;
use insta::assert_snapshot;
use pretty_assertions::assert_eq;
use ratatui::style::Color;
use ratatui::style::Modifier;
use std::collections::HashMap;

#[test]
fn interacted_sub_agent_activity_does_not_change_liveness() {
    let item = ThreadItem::SubAgentActivity {
        id: "activity-1".to_string(),
        kind: SubAgentActivityKind::Interacted,
        agent_thread_id: ThreadId::new().to_string(),
        agent_path: "/root/child".to_string(),
    };

    assert_eq!(sub_agent_activity_display(&item), None);
}

#[test]
fn collab_events_snapshot() {
    let sender_thread_id = ThreadId::from_string("00000000-0000-0000-0000-000000000001")
        .expect("valid sender thread id");
    let robie_id = ThreadId::from_string("00000000-0000-0000-0000-000000000002")
        .expect("valid robie thread id");
    let bob_id =
        ThreadId::from_string("00000000-0000-0000-0000-000000000003").expect("valid bob thread id");

    let spawn = tool_call_history_cell(
        &ThreadItem::CollabAgentToolCall {
            id: "call-spawn".to_string(),
            tool: CollabAgentTool::SpawnAgent,
            status: CollabAgentToolCallStatus::Completed,
            sender_thread_id: sender_thread_id.to_string(),
            receiver_thread_ids: vec![robie_id.to_string()],
            prompt: Some("Compute 11! and reply with just the integer result.".to_string()),
            model: Some("gpt-5".to_string()),
            reasoning_effort: Some(ReasoningEffortConfig::High),
            agents_states: HashMap::from([(
                robie_id.to_string(),
                agent_state(CollabAgentStatus::PendingInit, /*message*/ None),
            )]),
        },
        /*cached_spawn_request*/ None,
        |thread_id| metadata_for(thread_id, robie_id, bob_id),
    )
    .expect("spawn item renders");

    let send = tool_call_history_cell(
        &ThreadItem::CollabAgentToolCall {
            id: "call-send".to_string(),
            tool: CollabAgentTool::SendInput,
            status: CollabAgentToolCallStatus::Completed,
            sender_thread_id: sender_thread_id.to_string(),
            receiver_thread_ids: vec![robie_id.to_string()],
            prompt: Some("Please continue and return the answer only.".to_string()),
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::from([(
                robie_id.to_string(),
                agent_state(CollabAgentStatus::Running, /*message*/ None),
            )]),
        },
        /*cached_spawn_request*/ None,
        |thread_id| metadata_for(thread_id, robie_id, bob_id),
    )
    .expect("send-input item renders");

    let waiting = tool_call_history_cell(
        &ThreadItem::CollabAgentToolCall {
            id: "call-wait".to_string(),
            tool: CollabAgentTool::Wait,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: sender_thread_id.to_string(),
            receiver_thread_ids: vec![robie_id.to_string()],
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        },
        /*cached_spawn_request*/ None,
        |thread_id| metadata_for(thread_id, robie_id, bob_id),
    )
    .expect("wait begin item renders");

    let finished = tool_call_history_cell(
        &ThreadItem::CollabAgentToolCall {
            id: "call-wait".to_string(),
            tool: CollabAgentTool::Wait,
            status: CollabAgentToolCallStatus::Completed,
            sender_thread_id: sender_thread_id.to_string(),
            receiver_thread_ids: vec![robie_id.to_string(), bob_id.to_string()],
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::from([
                (
                    robie_id.to_string(),
                    agent_state(CollabAgentStatus::Completed, Some("39916800")),
                ),
                (
                    bob_id.to_string(),
                    agent_state(CollabAgentStatus::Errored, Some("tool timeout")),
                ),
            ]),
        },
        /*cached_spawn_request*/ None,
        |thread_id| metadata_for(thread_id, robie_id, bob_id),
    )
    .expect("wait end item renders");

    let close = tool_call_history_cell(
        &ThreadItem::CollabAgentToolCall {
            id: "call-close".to_string(),
            tool: CollabAgentTool::CloseAgent,
            status: CollabAgentToolCallStatus::Completed,
            sender_thread_id: sender_thread_id.to_string(),
            receiver_thread_ids: vec![robie_id.to_string()],
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::from([(
                robie_id.to_string(),
                agent_state(CollabAgentStatus::Completed, Some("39916800")),
            )]),
        },
        /*cached_spawn_request*/ None,
        |thread_id| metadata_for(thread_id, robie_id, bob_id),
    )
    .expect("close item renders");

    let snapshot = [spawn, send, waiting, finished, close]
        .iter()
        .map(cell_to_text)
        .collect::<Vec<_>>()
        .join("\n\n");
    assert_snapshot!("collab_agent_transcript", snapshot);
}

#[cfg(target_os = "macos")]
#[test]
fn agent_shortcut_matches_option_arrow_word_motion_fallbacks_only_when_allowed() {
    assert!(previous_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Left, KeyModifiers::ALT),
        /*allow_word_motion_fallback*/ false,
    ));
    assert!(next_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Right, KeyModifiers::ALT),
        /*allow_word_motion_fallback*/ false,
    ));
    assert!(previous_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
        /*allow_word_motion_fallback*/ true,
    ));
    assert!(next_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
        /*allow_word_motion_fallback*/ true,
    ));
    assert!(!previous_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
        /*allow_word_motion_fallback*/ false,
    ));
    assert!(!next_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
        /*allow_word_motion_fallback*/ false,
    ));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn agent_shortcut_matches_option_arrows_only() {
    assert!(previous_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Left, crossterm::event::KeyModifiers::ALT,),
        /*allow_word_motion_fallback*/ false
    ));
    assert!(next_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Right, crossterm::event::KeyModifiers::ALT,),
        /*allow_word_motion_fallback*/ false
    ));
    assert!(!previous_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Char('b'), crossterm::event::KeyModifiers::ALT,),
        /*allow_word_motion_fallback*/ false
    ));
    assert!(!next_agent_shortcut_matches(
        KeyEvent::new(KeyCode::Char('f'), crossterm::event::KeyModifiers::ALT,),
        /*allow_word_motion_fallback*/ false
    ));
}

#[test]
fn title_styles_nickname_and_role() {
    let sender_thread_id = ThreadId::from_string("00000000-0000-0000-0000-000000000001")
        .expect("valid sender thread id");
    let robie_id = ThreadId::from_string("00000000-0000-0000-0000-000000000002")
        .expect("valid robie thread id");
    let cell = tool_call_history_cell(
        &ThreadItem::CollabAgentToolCall {
            id: "call-spawn".to_string(),
            tool: CollabAgentTool::SpawnAgent,
            status: CollabAgentToolCallStatus::Completed,
            sender_thread_id: sender_thread_id.to_string(),
            receiver_thread_ids: vec![robie_id.to_string()],
            prompt: Some(String::new()),
            model: Some("gpt-5".to_string()),
            reasoning_effort: Some(ReasoningEffortConfig::High),
            agents_states: HashMap::from([(
                robie_id.to_string(),
                agent_state(CollabAgentStatus::PendingInit, /*message*/ None),
            )]),
        },
        /*cached_spawn_request*/ None,
        |thread_id| metadata_for(thread_id, robie_id, ThreadId::new()),
    )
    .expect("spawn item renders");

    let lines = cell.display_lines(/*width*/ 200);
    let title = &lines[0];
    assert_eq!(title.spans[2].content.as_ref(), "Robie");
    assert_eq!(title.spans[2].style.fg, Some(Color::Cyan));
    assert!(title.spans[2].style.add_modifier.contains(Modifier::BOLD));
    assert_eq!(title.spans[4].content.as_ref(), "[explorer]");
    assert_eq!(title.spans[4].style.fg, None);
    assert!(!title.spans[4].style.add_modifier.contains(Modifier::DIM));
    assert_eq!(title.spans[6].content.as_ref(), "(gpt-5 high)");
    assert_eq!(title.spans[6].style.fg, Some(Color::Magenta));
}

#[test]
fn collab_resume_interrupted_snapshot() {
    let sender_thread_id = ThreadId::from_string("00000000-0000-0000-0000-000000000001")
        .expect("valid sender thread id");
    let robie_id = ThreadId::from_string("00000000-0000-0000-0000-000000000002")
        .expect("valid robie thread id");

    let cell = tool_call_history_cell(
        &ThreadItem::CollabAgentToolCall {
            id: "call-resume".to_string(),
            tool: CollabAgentTool::ResumeAgent,
            status: CollabAgentToolCallStatus::Completed,
            sender_thread_id: sender_thread_id.to_string(),
            receiver_thread_ids: vec![robie_id.to_string()],
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::from([(
                robie_id.to_string(),
                agent_state(CollabAgentStatus::Interrupted, /*message*/ None),
            )]),
        },
        /*cached_spawn_request*/ None,
        |thread_id| metadata_for(thread_id, robie_id, ThreadId::new()),
    )
    .expect("resume item renders");

    assert_snapshot!("collab_resume_interrupted", cell_to_text(&cell));
}

fn agent_state(status: CollabAgentStatus, message: Option<&str>) -> CollabAgentState {
    CollabAgentState {
        status,
        message: message.map(str::to_string),
    }
}

fn metadata_for(thread_id: ThreadId, robie_id: ThreadId, bob_id: ThreadId) -> AgentMetadata {
    if thread_id == robie_id {
        AgentMetadata {
            agent_nickname: Some("Robie".to_string()),
            agent_role: Some("explorer".to_string()),
        }
    } else if thread_id == bob_id {
        AgentMetadata {
            agent_nickname: Some("Bob".to_string()),
            agent_role: Some("worker".to_string()),
        }
    } else {
        AgentMetadata::default()
    }
}

fn cell_to_text(cell: &PlainHistoryCell) -> String {
    cell.display_lines(/*width*/ 200)
        .iter()
        .map(line_to_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_to_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}
