//! `ChatWidget` 的用户输入提交、队列排空与草稿恢复流程。
//!
//! 队列数据本身位于 `input_queue`；本模块持有应用级
//! 效果：获取 composer 输入、提交用户回合、排空排队
//! 后续输入，以及在中断或线程切换时恢复草稿状态。

use super::*;

impl ChatWidget {
    pub(crate) fn set_parent_owned_thread(&mut self) {
        self.blocks_direct_input = true;
        self.bottom_pane.set_parent_owned_thread();
    }

    pub(super) fn handle_composer_input_result(
        &mut self,
        input_result: InputResult,
        had_modal_or_popup: bool,
    ) {
        match input_result {
            InputResult::Submitted {
                text,
                text_elements,
            } => {
                let user_message = self.user_message_from_submission(text, text_elements);
                if user_message.text.is_empty()
                    && user_message.local_images.is_empty()
                    && user_message.remote_image_urls.is_empty()
                {
                    return;
                }
                let should_submit_now = self.is_session_configured()
                    && !self.is_plan_streaming_in_tui()
                    && !self.input_queue.suppress_queue_autosend;
                if should_submit_now {
                    if self.only_user_shell_commands_running()
                        && !user_message.text.starts_with('!')
                    {
                        self.queue_user_message(user_message);
                        return;
                    }
                    // 用户提交时才会发出 Submitted。
                    // 只有在真正提交回合时才重置推理标题。
                    self.reasoning_buffer.clear();
                    self.reasoning_header = None;
                    self.reasoning_summary_parts.clear();
                    self.set_status_header(String::from("Working"));
                    self.submit_user_message(user_message);
                } else {
                    self.queue_user_message(user_message);
                }
            }
            InputResult::Queued {
                text,
                text_elements,
                action,
                pending_pastes,
            } => {
                let user_message = self.user_message_from_submission(text, text_elements);
                self.queue_user_message_with_options(user_message, action, pending_pastes);
            }
            InputResult::Command(cmd) => {
                self.handle_slash_command_dispatch(cmd);
            }
            InputResult::ServiceTierCommand(command) => {
                self.handle_service_tier_command_dispatch(command);
            }
            InputResult::CommandWithArgs(cmd, args, text_elements) => {
                self.handle_slash_command_with_args_dispatch(cmd, args, text_elements);
            }
            InputResult::ParentOwnedInputBlocked => {
                self.add_error_message(PARENT_OWNED_INPUT_MESSAGE.to_string());
            }
            InputResult::None => {}
        }
        if had_modal_or_popup && self.bottom_pane.no_modal_or_popup_active() {
            self.maybe_send_next_queued_input();
        }
        self.refresh_plan_mode_nudge();
    }

    pub(super) fn defer_input_until_settings_applied(&mut self) {
        if !self.bottom_pane.no_modal_or_popup_active() {
            self.input_queue.suppress_queue_autosend = true;
        }
    }

    pub(super) fn on_modal_or_popup_closed(&mut self) {
        if self.input_queue.suppress_queue_autosend {
            self.app_event_tx.send(AppEvent::SettingsSelectionClosed);
        } else {
            self.maybe_send_next_queued_input();
        }
    }

    pub(super) fn queue_user_message(&mut self, user_message: UserMessage) {
        self.queue_user_message_with_options(user_message, QueuedInputAction::Plain, Vec::new());
    }

    pub(crate) fn set_queue_submissions_until_session_configured(&mut self, queue: bool) {
        self.bottom_pane
            .set_queue_submissions(queue && !self.is_session_configured());
    }

    pub(super) fn queue_user_message_with_options(
        &mut self,
        user_message: UserMessage,
        action: QueuedInputAction,
        pending_pastes: Vec<(String, String)>,
    ) {
        if !self.is_session_configured()
            || self.is_user_turn_pending_or_running()
            || self.input_queue.suppress_queue_autosend
        {
            self.input_queue
                .queued_user_messages
                .push_back(QueuedUserMessage {
                    user_message,
                    action,
                    pending_pastes,
                });
            self.input_queue
                .queued_user_message_history_records
                .push_back(UserMessageHistoryRecord::UserMessageText);
            self.refresh_pending_input_preview();
        } else {
            self.submit_user_message(user_message);
        }
    }

    /// 若处于空闲且有排队输入，则恰好提交一个以开始下一回合。
    pub(crate) fn maybe_send_next_queued_input(&mut self) -> bool {
        if self.input_queue.suppress_queue_autosend {
            return false;
        }
        if self.blocks_direct_input {
            return false;
        }
        if self.is_user_turn_pending_or_running() {
            return false;
        }
        let mut submitted_follow_up = false;
        while !self.is_user_turn_pending_or_running() {
            let Some((queued_message, history_record)) = self.pop_next_queued_user_message() else {
                break;
            };
            match queued_message.action {
                QueuedInputAction::Plain => {
                    submitted_follow_up = self.submit_user_message_with_history_record(
                        queued_message.into_user_message(),
                        history_record,
                    );
                    break;
                }
                QueuedInputAction::ParseSlash => {
                    let drain = self.submit_queued_slash_prompt(queued_message);
                    if drain == QueueDrain::Stop {
                        submitted_follow_up = self.is_user_turn_pending_or_running();
                        break;
                    }
                }
                QueuedInputAction::RunShell => {
                    let drain = self.submit_queued_shell_prompt(queued_message.into_user_message());
                    if drain == QueueDrain::Stop {
                        submitted_follow_up = self.is_user_turn_pending_or_running();
                        break;
                    }
                }
            }
        }
        // 更新列表以反映剩余的排队消息（如果有）。
        self.refresh_pending_input_preview();
        submitted_follow_up
    }

    pub(super) fn is_user_turn_pending_or_running(&self) -> bool {
        self.input_queue.user_turn_pending_start || self.bottom_pane.is_task_running()
    }

    pub(super) fn only_user_shell_commands_running(&self) -> bool {
        self.turn_lifecycle.agent_turn_running
            && !self.running_commands.is_empty()
            && self
                .running_commands
                .values()
                .all(|command| command.source == ExecCommandSource::UserShell)
    }

    /// 重建并更新底部面板的待输入预览。
    pub(super) fn refresh_pending_input_preview(&mut self) {
        let preview = self.input_queue.preview();
        self.bottom_pane.set_pending_input_preview(
            preview.queued_messages,
            preview.pending_steers,
            preview.rejected_steers,
        );
    }

    pub(crate) fn submit_user_message_with_mode(
        &mut self,
        text: String,
        mut collaboration_mode: CollaborationModeMask,
    ) {
        if self.blocks_direct_input {
            self.add_error_message(PARENT_OWNED_INPUT_MESSAGE.to_string());
            return;
        }
        if collaboration_mode.mode == Some(ModeKind::Plan)
            && let Some(effort) = self.config.plan_mode_reasoning_effort.clone()
        {
            collaboration_mode.reasoning_effort = Some(Some(effort));
        }
        if self.turn_lifecycle.agent_turn_running
            && self.active_collaboration_mask.as_ref() != Some(&collaboration_mode)
        {
            self.add_error_message(
                "Cannot switch collaboration mode while a turn is running.".to_string(),
            );
            return;
        }
        self.set_collaboration_mask_from_user_action(collaboration_mode);
        let should_queue = self.is_plan_streaming_in_tui();
        let user_message = UserMessage {
            text,
            local_images: Vec::new(),
            remote_image_urls: Vec::new(),
            text_elements: Vec::new(),
            mention_bindings: Vec::new(),
        };
        if should_queue {
            self.queue_user_message(user_message);
        } else {
            self.submit_user_message(user_message);
        }
    }

    #[cfg(test)]
    pub(crate) fn queued_user_message_texts(&self) -> Vec<String> {
        self.input_queue
            .rejected_steers_queue
            .iter()
            .map(|message| message.text.clone())
            .chain(
                self.input_queue
                    .queued_user_messages
                    .iter()
                    .map(|message| message.text.clone()),
            )
            .collect()
    }
}
