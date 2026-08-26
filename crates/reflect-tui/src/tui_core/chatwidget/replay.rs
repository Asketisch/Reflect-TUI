//! `ChatWidget` 的会话线程回放渲染。
//!
//! 本模块将 turn 和 item 重新填充到会话记录状态，同时避免触发仅限实时的副作用。

use super::*;
use crate::utils_path_uri::LegacyAppPathString;

impl ChatWidget {
    /// 在恢复已有会话时，将一部分初始事件回放到 UI 中以填充会话记录。这是对实时事件流的近似模拟，
    /// 并且刻意保持保守：只渲染可安全回放的 item，以避免触发副作用。事件 id 传为 `None`，
    /// 以区分回放事件和实时事件。
    pub(crate) fn replay_thread_turns(&mut self, turns: Vec<Turn>, replay_kind: ReplayKind) {
        let hidden_nested_review_turns = std::iter::once(/*value*/ false)
            .chain(turns.windows(/*size*/ 2).map(|turns| {
                crate::tui_core::app_backtrack::is_hidden_nested_review_turn(&turns[0], &turns[1])
            }))
            .collect::<Vec<_>>();
        for (turn, hidden_nested_review_turn) in turns.into_iter().zip(hidden_nested_review_turns) {
            let Turn {
                id: turn_id,
                items_view: _,
                items,
                status,
                error,
                started_at,
                completed_at,
                duration_ms,
            } = turn;
            if matches!(status, TurnStatus::InProgress) {
                self.turn_lifecycle.last_turn_id = Some(turn_id.clone());
                self.last_non_retry_error = None;
                self.on_task_started();
            }
            for item in items {
                if hidden_nested_review_turn && matches!(item, ThreadItem::UserMessage { .. }) {
                    continue;
                }
                self.replay_thread_item(item, turn_id.clone(), replay_kind);
            }
            let status = if hidden_nested_review_turn {
                TurnStatus::Completed
            } else {
                status
            };
            if matches!(
                status,
                TurnStatus::Completed | TurnStatus::Interrupted | TurnStatus::Failed
            ) {
                self.handle_turn_completed_notification(
                    TurnCompletedNotification {
                        thread_id: self.thread_id.map(|id| id.to_string()).unwrap_or_default(),
                        turn: Turn {
                            id: turn_id,
                            items_view: crate::app_server_protocol::TurnItemsView::NotLoaded,
                            items: Vec::new(),
                            status,
                            error,
                            started_at,
                            completed_at,
                            duration_ms,
                        },
                    },
                    Some(replay_kind),
                );
            }
        }
    }

    pub(crate) fn replay_thread_item(
        &mut self,
        item: ThreadItem,
        turn_id: String,
        replay_kind: ReplayKind,
    ) {
        self.handle_thread_item(item, turn_id, ThreadItemRenderSource::Replay(replay_kind));
    }

    pub(super) fn handle_thread_item(
        &mut self,
        item: ThreadItem,
        turn_id: String,
        render_source: ThreadItemRenderSource,
    ) {
        let from_replay = render_source.is_replay();
        let replay_kind = render_source.replay_kind();
        match item {
            ThreadItem::UserMessage { content, .. } => {
                self.on_committed_user_message(&content, from_replay);
            }
            ThreadItem::AgentMessage {
                id,
                text,
                phase,
                memory_citation,
            } => {
                self.on_agent_message_item_completed(
                    AgentMessageItem {
                        id,
                        content: vec![AgentMessageContent::Text { text }],
                        phase: phase.and_then(|p| p.to_core()),
                        memory_citation: memory_citation.map(|citation| {
                            crate::protocol_compat::memory_citation::MemoryCitation {
                                entries: citation
                                    .entries
                                    .into_iter()
                                    .map(|entry| {
                                        crate::protocol_compat::memory_citation::MemoryCitationEntry {
                                            path: entry.path,
                                            line_start: entry.line_start,
                                            line_end: entry.line_end,
                                            note: entry.note,
                                        }
                                    })
                                    .collect(),
                                rollout_ids: citation.thread_ids,
                            }
                        }),
                    },
                    from_replay,
                );
            }
            ThreadItem::Plan { text, .. } => self.on_plan_item_completed(text),
            ThreadItem::Reasoning {
                summary, content, ..
            } => {
                if from_replay {
                    let reasoning_parts = summary.into_iter().chain(
                        self.config
                            .show_raw_agent_reasoning
                            .then_some(content)
                            .into_iter()
                            .flatten(),
                    );
                    for (index, delta) in reasoning_parts.enumerate() {
                        if index > 0 {
                            self.on_reasoning_section_break();
                        }
                        self.on_agent_reasoning_delta(delta);
                    }
                }
                self.on_agent_reasoning_final();
            }
            item @ ThreadItem::CommandExecution {
                status: crate::app_server_protocol::CommandExecutionStatus::InProgress,
                ..
            } => self.on_command_execution_started(item),
            item @ ThreadItem::CommandExecution { .. } => self.on_command_execution_completed(item),
            ThreadItem::FileChange {
                status: crate::app_server_protocol::PatchApplyStatus::InProgress,
                ..
            } => {}
            item @ ThreadItem::FileChange { .. } => self.on_file_change_completed(item),
            item @ ThreadItem::McpToolCall {
                status: crate::app_server_protocol::McpToolCallStatus::InProgress,
                ..
            } => self.on_mcp_tool_call_started(item),
            item @ ThreadItem::McpToolCall { .. } => self.on_mcp_tool_call_completed(item),
            ThreadItem::WebSearch(item) => {
                self.on_web_search_begin(item.id.clone());
                self.on_web_search_end(
                    item.id,
                    item.query,
                    item.action
                        .unwrap_or(crate::app_server_protocol::WebSearchAction::Other),
                );
            }
            ThreadItem::ImageView { id: _, path } => {
                self.on_view_image_tool_call(LegacyAppPathString::Local(path));
            }
            ThreadItem::ImageGeneration(item) => {
                self.on_image_generation_end(
                    item.id,
                    item.status,
                    Some(item.revised_prompt),
                    Some(crate::utils_absolute_path::AbsolutePathBuf::from(
                        item.saved_path,
                    )),
                );
            }
            ThreadItem::EnteredReviewMode { review, .. } => {
                if from_replay {
                    self.enter_review_mode_with_hint(review, /*from_replay*/ true);
                }
            }
            ThreadItem::ExitedReviewMode { .. } => {
                self.exit_review_mode_after_item();
            }
            ThreadItem::ContextCompaction { .. } => {
                self.add_info_message("Context compacted".to_string(), /*hint*/ None);
            }
            ThreadItem::HookPrompt { .. } => {}
            ThreadItem::CollabAgentToolCall {
                id,
                tool,
                status,
                sender_thread_id,
                receiver_thread_ids,
                prompt,
                model,
                reasoning_effort,
                agents_states,
            } => self.on_collab_agent_tool_call(ThreadItem::CollabAgentToolCall {
                id,
                tool,
                status,
                sender_thread_id,
                receiver_thread_ids,
                prompt,
                model,
                reasoning_effort,
                agents_states,
            }),
            item @ ThreadItem::SubAgentActivity { .. } => self.on_sub_agent_activity(item),
            ThreadItem::DynamicToolCall { .. } => {}
            ThreadItem::Sleep(_) => {}
        }

        if matches!(replay_kind, Some(ReplayKind::ThreadSnapshot)) && turn_id.is_empty() {
            self.request_redraw();
        }
    }
}
