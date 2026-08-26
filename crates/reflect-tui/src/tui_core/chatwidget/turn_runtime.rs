//! `ChatWidget` 的代理轮次生命周期与运行时簿记。
//!
//! 本模块负责任务开始/完成状态、运行时指标、计划更新以及最终消息分隔符的处理。

use super::*;

const LEGACY_SAFETY_ACCESS_BLOCK_PREFIX: &str =
    "Invalid prompt: we've limited access to this content for safety reasons.";
const BIO_POLICY_SAFETY_ACCESS_BLOCK_PREFIX: &str =
    "This content was flagged for possible biological risk.";

fn is_safety_access_block_message(message: &str) -> bool {
    message.starts_with(LEGACY_SAFETY_ACCESS_BLOCK_PREFIX)
        || message.starts_with(BIO_POLICY_SAFETY_ACCESS_BLOCK_PREFIX)
}

impl ChatWidget {
    fn clear_guardian_review_status(&mut self) {
        self.status_state.pending_guardian_review_status.clear();
        if self.status_state.current_status.is_guardian_review() {
            let header = self
                .mcp_startup_status_header()
                .unwrap_or_else(|| String::from("Working"));
            self.set_status_header(header);
        }
    }

    /// 将底部窗格的“任务运行中”指示器与当前的生命周期同步。
    ///
    /// 底部窗格只有一个运行标志，但本模块将其视为代理轮次生命周期与 MCP 启动生命周期两者的派生状态。
    pub(super) fn update_task_running_state(&mut self) {
        self.bottom_pane.set_task_running(
            self.turn_lifecycle.agent_turn_running || self.mcp_startup_status.is_some(),
        );
        self.refresh_plan_mode_nudge();
        self.refresh_status_surfaces();
    }

    pub(super) fn collect_runtime_metrics_delta(&mut self) {
        let delta = self.session_telemetry.runtime_metrics_summary();
        {
            self.apply_runtime_metrics_delta(delta);
        }
    }

    pub(super) fn apply_runtime_metrics_delta(&mut self, delta: RuntimeMetricsSummary) {
        let should_log_timing = has_websocket_timing_metrics(&delta);
        self.turn_runtime_metrics.merge(&delta);
        if should_log_timing {
            self.log_websocket_timing_totals(delta);
        }
    }

    pub(super) fn log_websocket_timing_totals(&mut self, delta: RuntimeMetricsSummary) {
        if let Some(label) = history_cell::runtime_metrics_label(delta) {
            self.add_plain_history_lines(vec![
                vec!["• ".dim(), format!("WebSocket timing: {label}").dark_gray()].into(),
            ]);
        }
    }

    pub(super) fn refresh_runtime_metrics(&mut self) {
        self.collect_runtime_metrics_delta();
    }

    // 原始推理与摘要推理使用相同的流程

    pub(super) fn on_task_started(&mut self) {
        self.input_queue.user_turn_pending_start = false;
        self.reset_safety_buffering_for_turn_start();
        self.turn_lifecycle.start(Instant::now());
        self.transcript.reset_turn_flags();
        self.adaptive_chunking.reset();
        if self.plan_stream_controller.take().is_some() {
            self.request_pending_usage_output_insertion_after_stream_shutdown();
        }
        self.turn_runtime_metrics = RuntimeMetricsSummary::default();
        self.session_telemetry.reset_runtime_metrics();
        self.bottom_pane.clear_quit_shortcut_hint();
        self.quit_shortcut_expires_at = None;
        self.quit_shortcut_key = None;
        self.update_task_running_state();
        self.status_state.retry_status_header = None;
        self.clear_active_hook_cell();
        self.status_state.pending_status_indicator_restore = false;
        self.bottom_pane
            .set_interrupt_hint_visible(/*visible*/ true);
        self.status_state.terminal_title_status_kind = TerminalTitleStatusKind::Working;
        if self.mcp_startup_status.is_none() || !self.status_header_is_mcp_startup_owned() {
            self.set_status_header(String::from("Working"));
        }
        self.reasoning_summary_parts.clear();
        self.reasoning_buffer.clear();
        self.reasoning_header = None;
        self.set_ambient_pet_notification(
            crate::tui_core::pets::PetNotificationKind::Running,
            /*body*/ None,
        );
        self.request_redraw();
    }

    pub(super) fn on_task_complete(
        &mut self,
        last_agent_message: Option<String>,
        duration_ms: Option<i64>,
        from_replay: bool,
    ) {
        self.input_queue.submit_pending_steers_after_interrupt = false;
        // 仅当本轮没有更早的条目级事件（AgentMessageItem、计划提交、审查输出）
        // 已经记录 markdown 时，才使用 turn-complete 通知中的 `last_agent_message`
        // 作为复制来源。这样可以避免最终摘要覆盖更具体的来源。
        let sanitized_last_agent_message = last_agent_message.as_deref().map(|message| {
            parse_assistant_markdown(message, self.config.cwd.as_path()).visible_markdown
        });
        if let Some(message) = sanitized_last_agent_message
            .as_ref()
            .filter(|message| !message.is_empty())
            && !self.transcript.saw_copy_source_this_turn
        {
            self.record_agent_markdown(message);
        }
        // 桌面通知：优先使用通知载荷；若存在条目级复制来源则回退到它，否则发送空字符串。
        let notification_response = sanitized_last_agent_message
            .as_ref()
            .filter(|message| !message.is_empty())
            .cloned()
            .or_else(|| {
                if self.transcript.saw_copy_source_this_turn {
                    self.transcript.last_agent_markdown.clone()
                } else {
                    None
                }
            })
            .unwrap_or_default();
        self.transcript.saw_copy_source_this_turn = false;
        // 如果当前有活跃的流，则将其终结。
        self.flush_answer_stream_with_separator();
        if let Some(mut controller) = self.plan_stream_controller.take() {
            let had_live_tail = controller.has_live_tail();
            self.clear_active_stream_tail();
            let (cell, source) = controller.finalize();
            if !had_live_tail && let Some(cell) = cell {
                self.add_boxed_history(cell);
            }
            if let Some(source) = source {
                self.note_stream_consolidation_queued();
                self.app_event_tx
                    .send(AppEvent::ConsolidateProposedPlan(source));
            }
            self.request_pending_usage_output_insertion_after_stream_shutdown();
        }
        self.flush_unified_exec_wait_streak();
        if !from_replay {
            self.collect_runtime_metrics_delta();
            let runtime_metrics = (!self.turn_runtime_metrics.is_empty())
                .then_some(self.turn_runtime_metrics.clone());
            let show_work_separator = self.transcript.had_work_activity
                && (self.transcript.needs_final_message_separator || runtime_metrics.is_some());
            if show_work_separator || runtime_metrics.is_some() {
                let elapsed_seconds = if show_work_separator {
                    duration_ms
                        .and_then(|duration_ms| u64::try_from(duration_ms).ok())
                        .map(|duration_ms| duration_ms / 1_000)
                        .or_else(|| {
                            self.bottom_pane
                                .status_widget()
                                .map(crate::tui_core::status_indicator_widget::StatusIndicatorWidget::elapsed_seconds)
                        })
                } else {
                    None
                };
                self.add_to_history(history_cell::FinalMessageSeparator::new(
                    elapsed_seconds,
                    runtime_metrics,
                ));
            }
            self.turn_runtime_metrics = RuntimeMetricsSummary::default();
            self.transcript.needs_final_message_separator = false;
            self.transcript.had_work_activity = false;
            self.request_status_line_branch_refresh();
            self.request_status_line_git_summary_refresh();
        }
        // 所有内容都已进入历史后，标记任务停止并请求重绘。
        self.status_state.pending_status_indicator_restore = false;
        self.input_queue.user_turn_pending_start = false;
        self.clear_active_hook_cell();
        self.clear_guardian_review_status();
        self.turn_lifecycle.finish();
        self.clear_safety_buffering();
        self.update_task_running_state();
        self.running_commands.clear();
        self.suppressed_exec_calls.clear();
        self.last_unified_wait = None;
        self.unified_exec_wait_streak = None;
        if !from_replay {
            let body = Notification::agent_turn_preview(&notification_response);
            self.set_ambient_pet_notification(
                crate::tui_core::pets::PetNotificationKind::Review,
                body,
            );
        }
        self.request_redraw();

        let had_pending_steers = !self.input_queue.pending_steers.is_empty();
        self.refresh_pending_input_preview();

        if !from_replay && !self.has_queued_follow_up_messages() && !had_pending_steers {
            self.maybe_prompt_plan_implementation();
        }
        // 为重放的完成事件保留该标志，以便线程切换重放之后的下一次实时 TurnComplete
        // 仍能显示一次提示。
        if !from_replay {
            self.transcript.saw_plan_item_this_turn = false;
        }
        // 如果存在排队的用户消息，现在只发送一条以开始下一轮。
        let follow_up_started = self.maybe_send_next_queued_input();
        let active_goal_continuing = self
            .current_goal_status
            .as_ref()
            .is_some_and(GoalStatusState::is_active);
        // 仅当代理真正在等待用户时才发送通知。
        // 排队的后续输入和活跃目标的延续都会立即开始下一轮，
        // 因此在该边界发送通知会带来虚假的“需要关注”感。
        if !follow_up_started && !active_goal_continuing {
            self.notify(Notification::AgentTurnComplete {
                response: notification_response,
            });
        }

        self.maybe_show_pending_rate_limit_prompt();
    }

    pub(super) fn maybe_prompt_plan_implementation(&mut self) {
        if !self.collaboration_modes_enabled() {
            return;
        }
        if self.has_queued_follow_up_messages() {
            return;
        }
        if self.active_mode_kind() != ModeKind::Plan {
            return;
        }
        if !self.transcript.saw_plan_item_this_turn {
            return;
        }
        if !self.bottom_pane.no_modal_or_popup_active() {
            return;
        }

        if matches!(
            self.rate_limit_switch_prompt,
            RateLimitSwitchPromptState::Pending
        ) {
            return;
        }

        self.open_plan_implementation_prompt();
    }

    pub(super) fn open_plan_implementation_prompt(&mut self) {
        let default_mask = collaboration_modes::default_mode_mask(self.model_catalog.as_ref());
        let context_usage_label = self.plan_implementation_context_usage_label();

        self.bottom_pane
            .show_selection_view(plan_implementation::selection_view_params(
                default_mask,
                self.transcript.latest_proposed_plan_markdown.as_deref(),
                context_usage_label.as_deref(),
            ));
        self.notify(Notification::PlanModePrompt {
            title: PLAN_IMPLEMENTATION_TITLE.to_string(),
        });
    }

    /// 返回计划实施提示所用的上下文已用量标签。
    ///
    /// 页脚显示的是上下文剩余量，因为那属于环境状态；但该提示询问的是在
    /// 实施计划之前是否丢弃先前的会话状态。报告已用上下文可以让清理的
    /// 取舍更加明确。完全全新或未知的上下文窗口不返回标签，这样
    /// “清除上下文”选项就不会在缺乏证据时暗示紧迫性。
    pub(super) fn plan_implementation_context_usage_label(&self) -> Option<String> {
        let info = self.token_info.as_ref()?;
        let percent = self.context_remaining_percent(info);

        let used_tokens = self.context_used_tokens(info, percent.is_some());
        if let Some(percent) = percent {
            let used_percent = 100 - percent.clamp(0, 100);
            if used_percent <= 0 {
                return None;
            }
            return Some(format!("{used_percent}% used"));
        }

        if let Some(tokens) = used_tokens
            && tokens > 0
        {
            return Some(format!("{} used", format_tokens_compact(tokens)));
        }

        None
    }

    pub(super) fn has_queued_follow_up_messages(&self) -> bool {
        self.input_queue.has_queued_follow_up_messages()
    }

    pub(super) fn handle_app_server_steer_rejected_error(
        &mut self,
        reflect_error_info: &AppServerReflectErrorInfo,
    ) -> bool {
        matches!(
            reflect_error_info,
            AppServerReflectErrorInfo::ActiveTurnNotSteerable { .. }
        ) && self.enqueue_rejected_steer()
    }

    /// 将任何活跃的 exec 终结为失败，并停止/清除代理轮次的 UI 状态。
    ///
    /// 这不会清除 MCP 启动跟踪，因为 MCP 启动可能与轮次清理重叠，
    /// 并且在其进行期间应继续驱动底部窗格的运行指示器。
    pub(super) fn finalize_turn(&mut self) {
        self.clear_safety_buffering();
        // 在任何终止路径上，都先于失败单元格的最终化丢弃仅用于预览的流式尾随内容，
        // 以确保瞬态尾随单元格永远不会被持久化。
        self.clear_active_stream_tail();
        // 确保任何加载动画都被替换为红色的 ✗ 并刷入历史。
        self.finalize_active_cell_as_failed();
        // 轮次作用域的 hook 行是瞬态的实时状态；轮次结束后，
        // 如果取消前没有到达匹配的完成事件，就不要留下孤立的运行中行。
        self.clear_active_hook_cell();
        // 重置运行状态并清除流式缓冲区。
        self.input_queue.user_turn_pending_start = false;
        self.clear_guardian_review_status();
        self.turn_lifecycle.finish();
        self.update_task_running_state();
        self.running_commands.clear();
        self.suppressed_exec_calls.clear();
        self.last_unified_wait = None;
        self.unified_exec_wait_streak = None;
        self.adaptive_chunking.reset();
        self.stream_controller = None;
        self.plan_stream_controller = None;
        self.request_pending_usage_output_insertion_after_stream_shutdown();
        self.status_state.pending_status_indicator_restore = false;
        self.safety_buffering_prompt = None;
        self.request_status_line_branch_refresh();
        self.request_status_line_git_summary_refresh();
        self.maybe_show_pending_rate_limit_prompt();
    }

    pub(super) fn on_server_overloaded_error(&mut self, message: String) {
        self.input_queue.submit_pending_steers_after_interrupt = false;
        self.finalize_turn();

        let message = if message.trim().is_empty() {
            "Reflect is currently experiencing high load.".to_string()
        } else {
            message
        };

        self.add_to_history(history_cell::new_warning_event(message));
        self.request_redraw();
        self.maybe_send_next_queued_input();
    }

    pub(super) fn on_error(&mut self, message: String) {
        self.input_queue.submit_pending_steers_after_interrupt = false;
        self.flush_answer_stream_with_separator();
        self.finalize_turn();
        self.add_to_history(history_cell::new_error_event(message));
        self.set_ambient_pet_notification(
            crate::tui_core::pets::PetNotificationKind::Failed,
            /*body*/ None,
        );
        self.request_redraw();

        // 错误结束轮次后，尝试发送下一条排队的输入。
        self.maybe_send_next_queued_input();
    }

    pub(super) fn on_cyber_policy_error(&mut self) {
        self.input_queue.submit_pending_steers_after_interrupt = false;
        self.finalize_turn();
        self.add_to_history(history_cell::new_cyber_policy_error_event());
        self.request_redraw();

        // 错误结束轮次后，尝试发送下一条排队的输入。
        self.maybe_send_next_queued_input();
    }

    pub(super) fn on_rate_limit_error(&mut self, error_kind: RateLimitErrorKind, message: String) {
        let usage_limit_error = matches!(error_kind, RateLimitErrorKind::UsageLimit);
        let rate_limit_reached_type = self.reflect_rate_limit_reached_type.clone().map(|kind| {
            if usage_limit_error {
                match kind {
                    RateLimitReachedType::WorkspaceOwnerCreditsDepleted => {
                        RateLimitReachedType::WorkspaceOwnerUsageLimitReached
                    }
                    RateLimitReachedType::WorkspaceMemberCreditsDepleted => {
                        RateLimitReachedType::WorkspaceMemberUsageLimitReached
                    }
                    other => other,
                }
            } else {
                kind
            }
        });
        self.reflect_rate_limit_reached_type = rate_limit_reached_type.clone();
        match rate_limit_reached_type {
            Some(RateLimitReachedType::WorkspaceOwnerCreditsDepleted) => {
                self.on_error(
                    "You're out of credits. Your workspace is out of credits. Add credits to continue using Reflect."
                        .to_string(),
                );
            }
            Some(RateLimitReachedType::WorkspaceOwnerUsageLimitReached) => {
                self.on_error(
                    "Usage limit reached. You've reached your usage limit. Increase your limits to continue using reflect."
                        .to_string(),
                );
            }
            Some(RateLimitReachedType::WorkspaceMemberCreditsDepleted) => {
                self.on_error(message);
                self.open_workspace_owner_nudge_prompt(AddCreditsNudgeCreditType::Credits);
            }
            Some(RateLimitReachedType::WorkspaceMemberUsageLimitReached) => {
                self.on_error(message);
                self.open_workspace_owner_nudge_prompt(AddCreditsNudgeCreditType::UsageLimit);
            }
            Some(RateLimitReachedType::RateLimitReached) | None => {
                self.on_error(message);
            }
        }
    }

    pub(super) fn handle_non_retry_error(
        &mut self,
        message: String,
        reflect_error_info: Option<AppServerReflectErrorInfo>,
    ) {
        if reflect_error_info
            .as_ref()
            .is_some_and(|info| self.handle_app_server_steer_rejected_error(info))
        {
        } else if reflect_error_info
            .as_ref()
            .is_some_and(is_app_server_cyber_policy_error)
        {
            self.on_cyber_policy_error();
        } else if is_safety_access_block_message(&message)
            || serde_json::from_str::<serde_json::Value>(&message).is_ok_and(|response| {
                response["error"]["code"].as_str() == Some("bio_policy")
                    || response["error"]["message"]
                        .as_str()
                        .is_some_and(is_safety_access_block_message)
            })
        {
            self.input_queue.submit_pending_steers_after_interrupt = false;
            self.finalize_turn();
            self.add_to_history(history_cell::new_safety_access_block_event());
            self.request_redraw();
            self.maybe_send_next_queued_input();
        } else if let Some(info) = reflect_error_info
            .as_ref()
            .and_then(app_server_rate_limit_error_kind)
        {
            match info {
                RateLimitErrorKind::ServerOverloaded => self.on_server_overloaded_error(message),
                RateLimitErrorKind::UsageLimit | RateLimitErrorKind::Generic => {
                    self.on_rate_limit_error(info, message)
                }
            }
        } else {
            self.on_error(message);
        }
    }

    pub(super) fn on_warning(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !self.warning_display_state.should_display(&message) {
            return;
        }
        self.add_to_history(history_cell::new_warning_event(message));
        self.request_redraw();
    }

    pub(super) fn on_app_server_model_verification(
        &mut self,
        verifications: &[AppServerModelVerification],
    ) {
        if verifications.contains(&AppServerModelVerification::TrustedAccessForCyber) {
            self.on_warning(TRUSTED_ACCESS_FOR_CYBER_VERIFICATION_WARNING);
        }
    }

    pub(super) fn on_plan_update(&mut self, update: UpdatePlanArgs) {
        self.transcript.saw_plan_update_this_turn = true;
        let total = update.plan.len();
        let completed = update
            .plan
            .iter()
            .filter(|item| match &item.status {
                StepStatus::Completed => true,
                StepStatus::Pending | StepStatus::InProgress => false,
            })
            .count();
        self.transcript.last_plan_progress = (total > 0).then_some((completed, total));
        self.refresh_status_surfaces();
        self.add_to_history(history_cell::new_plan_update(update));
    }

    pub(super) fn interrupted_turn_message(&self, reason: TurnAbortReason) -> String {
        if reason == TurnAbortReason::BudgetLimited {
            return "Goal budget reached - the turn was stopped.".to_string();
        }

        "Conversation interrupted - tell the model what to do differently. Something went wrong? Hit `/feedback` to report the issue.".to_string()
    }
}
