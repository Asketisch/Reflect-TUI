//! 回退（app_backtrack） 的测试集。
//!
//! 从 app_backtrack.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::app_server_protocol::UserInput;
use crate::tui_core::bottom_pane::MentionBinding;
use crate::tui_core::history_cell::AgentMessageCell;
use crate::tui_core::history_cell::HistoryCell;
use pretty_assertions::assert_eq;
use ratatui::prelude::Line;
use std::path::PathBuf;
use std::sync::Arc;

fn render_lines(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

fn turn(turn_id: &str, status: TurnStatus, user_messages: usize) -> Turn {
    Turn {
        id: turn_id.to_string(),
        items: (0..user_messages)
            .map(|index| ThreadItem::UserMessage {
                id: format!("user-{index}"),
                client_id: None,
                content: vec![UserInput::Text {
                    text: format!("{turn_id}-prompt-{index}"),
                    text_elements: Vec::new(),
                }],
            })
            .collect(),
        items_view: crate::app_server_protocol::TurnItemsView::Full,
        status,
        error: None,
        started_at: None,
        completed_at: None,
        duration_ms: None,
    }
}

fn prompt(text: &str) -> UserMessage {
    UserMessage {
        text: text.to_string(),
        local_images: Vec::new(),
        remote_image_urls: Vec::new(),
        text_elements: Vec::new(),
        mention_bindings: Vec::new(),
    }
}

#[test]
fn backtrack_fork_before_turn_id_resolves_first_and_later_prompts() {
    let turns = vec![
        turn("turn-1", TurnStatus::Completed, /*user_messages*/ 1),
        turn(
            "turn-compaction",
            TurnStatus::Completed,
            /*user_messages*/ 0,
        ),
        turn("turn-2", TurnStatus::Completed, /*user_messages*/ 1),
    ];

    assert_eq!(
        backtrack_fork_before_turn_id(
            &turns,
            /*nth_user_message*/ 0,
            &mut prompt("turn-1-prompt-0"),
        )
        .expect("first prompt should resolve"),
        None
    );
    assert_eq!(
        backtrack_fork_before_turn_id(
            &turns,
            /*nth_user_message*/ 1,
            &mut prompt("turn-2-prompt-0"),
        )
        .expect("later prompt should resolve"),
        Some("turn-2".to_string())
    );
}

#[test]
fn backtrack_fork_before_turn_id_rejects_mid_turn_steers() {
    let turns = vec![turn(
        "turn-1",
        TurnStatus::Completed,
        /*user_messages*/ 2,
    )];

    let error = backtrack_fork_before_turn_id(
        &turns,
        /*nth_user_message*/ 1,
        &mut prompt("turn-1-prompt-1"),
    )
    .expect_err("a steer cannot be branched independently");

    assert_eq!(
        error.to_string(),
        "the selected prompt is a steer and cannot be branched independently"
    );
}

#[test]
fn backtrack_fork_before_turn_id_rejects_in_progress_and_missing_prompts() {
    let turns = vec![turn(
        "turn-1",
        TurnStatus::InProgress,
        /*user_messages*/ 1,
    )];

    assert_eq!(
        backtrack_fork_before_turn_id(
            &turns,
            /*nth_user_message*/ 0,
            &mut prompt("turn-1-prompt-0"),
        )
        .expect_err("in-progress prompt cannot be branched")
        .to_string(),
        "the selected prompt belongs to a turn that is still in progress"
    );
    assert_eq!(
        backtrack_fork_before_turn_id(
            &turns,
            /*nth_user_message*/ 1,
            &mut prompt("missing prompt"),
        )
        .expect_err("missing prompt cannot be branched")
        .to_string(),
        "the selected prompt was not found in the persisted thread"
    );

    let completed_turns = vec![turn(
        "turn-1",
        TurnStatus::Completed,
        /*user_messages*/ 1,
    )];
    assert_eq!(
        backtrack_fork_before_turn_id(
            &completed_turns,
            /*nth_user_message*/ 0,
            &mut prompt("different prompt"),
        )
        .expect_err("a stale transcript prompt cannot be branched")
        .to_string(),
        "the selected transcript prompt no longer matches the persisted thread"
    );
}

#[test]
fn backtrack_fork_before_turn_id_skips_hidden_review_prompts() {
    let mut review_turn = turn(
        "turn-review",
        TurnStatus::Completed,
        /*user_messages*/ 1,
    );
    review_turn.items.insert(
        /*index*/ 0,
        ThreadItem::EnteredReviewMode {
            id: "review-start".to_string(),
            review: "changes against main".to_string(),
        },
    );
    review_turn.items.push(ThreadItem::ExitedReviewMode {
        id: "review-end".to_string(),
        review: "review complete".to_string(),
    });
    let turns = vec![
        turn("turn-1", TurnStatus::Completed, /*user_messages*/ 1),
        review_turn,
        turn("turn-2", TurnStatus::Completed, /*user_messages*/ 1),
    ];

    assert_eq!(
        backtrack_fork_before_turn_id(
            &turns,
            /*nth_user_message*/ 1,
            &mut prompt("turn-2-prompt-0"),
        )
        .expect("the visible prompt after review should resolve"),
        Some("turn-2".to_string())
    );
}

#[test]
fn backtrack_fork_before_turn_id_skips_hidden_nested_review_prompts() {
    let review_hint = "current changes";
    let review_prompt = "Review the current code changes (staged, unstaged, and untracked files).";
    let review_turn = Turn {
        items: vec![
            ThreadItem::EnteredReviewMode {
                id: "review-start".to_string(),
                review: review_hint.to_string(),
            },
            ThreadItem::ExitedReviewMode {
                id: "review-end".to_string(),
                review: "review complete".to_string(),
            },
        ],
        ..turn(
            "turn-review",
            TurnStatus::Completed,
            /*user_messages*/ 0,
        )
    };
    let review_child_turn = Turn {
        items: (0..2)
            .map(|index| ThreadItem::UserMessage {
                id: format!("review-prompt-{index}"),
                client_id: None,
                content: vec![UserInput::Text {
                    text: review_prompt.to_string(),
                    text_elements: Vec::new(),
                }],
            })
            .collect(),
        ..turn(
            "turn-review-child",
            TurnStatus::Interrupted,
            /*user_messages*/ 0,
        )
    };
    let interrupted_steered_turn = Turn {
        items: review_child_turn.items.clone(),
        completed_at: Some(1),
        ..turn(
            "turn-interrupted-steer",
            TurnStatus::Interrupted,
            /*user_messages*/ 0,
        )
    };
    assert!(!is_hidden_nested_review_turn(
        &review_turn,
        &interrupted_steered_turn,
    ));
    let turns = vec![
        review_turn,
        review_child_turn,
        turn("turn-2", TurnStatus::Completed, /*user_messages*/ 1),
    ];

    assert_eq!(
        backtrack_fork_before_turn_id(
            &turns,
            /*nth_user_message*/ 0,
            &mut prompt("turn-2-prompt-0"),
        )
        .expect("the visible prompt after a nested review should resolve"),
        Some("turn-2".to_string())
    );
}

#[test]
fn backtrack_fork_before_turn_id_restores_canonical_mention_bindings() {
    let mut selected_turn = turn("turn-2", TurnStatus::Completed, /*user_messages*/ 1);
    selected_turn.items = vec![ThreadItem::UserMessage {
        id: "selected-prompt".to_string(),
        client_id: None,
        content: vec![
            UserInput::Text {
                text: "use $skill @sample $google-calendar".to_string(),
                text_elements: Vec::new(),
            },
            UserInput::Skill {
                name: "skill".to_string(),
                path: PathBuf::from("/tmp/skills/skill/SKILL.md"),
            },
            UserInput::Mention {
                name: "Sample Plugin".to_string(),
                path: "plugin://sample@test".to_string(),
            },
            UserInput::Mention {
                name: "Google Calendar".to_string(),
                path: "app://google_calendar".to_string(),
            },
        ],
    }];
    let turns = vec![
        turn("turn-1", TurnStatus::Completed, /*user_messages*/ 1),
        selected_turn,
    ];
    let mut selected_prompt = prompt("use $skill @sample $google-calendar");

    assert_eq!(
        backtrack_fork_before_turn_id(&turns, /*nth_user_message*/ 1, &mut selected_prompt,)
            .expect("the selected prompt should resolve"),
        Some("turn-2".to_string())
    );
    assert_eq!(
        selected_prompt.mention_bindings,
        vec![
            MentionBinding {
                sigil: '$',
                mention: "skill".to_string(),
                path: "/tmp/skills/skill/SKILL.md".to_string(),
            },
            MentionBinding {
                sigil: '@',
                mention: "sample".to_string(),
                path: "plugin://sample@test".to_string(),
            },
            MentionBinding {
                sigil: '$',
                mention: "google-calendar".to_string(),
                path: "app://google_calendar".to_string(),
            },
        ]
    );
}

#[test]
fn agent_group_count_ignores_context_compacted_marker() {
    let cells: Vec<Arc<dyn HistoryCell>> = vec![
        Arc::new(AgentMessageCell::new(
            vec![Line::from("first")],
            /*is_first_line*/ true,
        )) as Arc<dyn HistoryCell>,
        Arc::new(crate::tui_core::history_cell::new_info_event(
            "Context compacted".to_string(),
            /*hint*/ None,
        )) as Arc<dyn HistoryCell>,
        Arc::new(AgentMessageCell::new(
            vec![Line::from("second")],
            /*is_first_line*/ true,
        )) as Arc<dyn HistoryCell>,
    ];

    assert_eq!(agent_group_count(&cells), 2);
}

#[test]
fn backtrack_target_requires_user_message() {
    let mut cells: Vec<Arc<dyn HistoryCell>> = vec![
        Arc::new(AgentMessageCell::new(
            vec![Line::from("assistant")],
            /*is_first_line*/ true,
        )) as Arc<dyn HistoryCell>,
        Arc::new(crate::tui_core::history_cell::new_info_event(
            "Context compacted".to_string(),
            /*hint*/ None,
        )) as Arc<dyn HistoryCell>,
    ];

    assert!(!has_backtrack_target(&cells));

    cells.push(Arc::new(UserHistoryCell {
        message: "hello".to_string(),
        text_elements: Vec::new(),
        local_image_paths: Vec::new(),
        remote_image_urls: Vec::new(),
    }) as Arc<dyn HistoryCell>);

    assert!(has_backtrack_target(&cells));
}

#[test]
fn backtrack_unavailable_info_message_snapshot() {
    let cell = crate::tui_core::history_cell::new_info_event(
        NO_PREVIOUS_MESSAGE_TO_EDIT.to_string(),
        /*hint*/ None,
    );
    let rendered = render_lines(&cell.display_lines(/*width*/ 80)).join("\n");

    insta::assert_snapshot!(rendered);
}
