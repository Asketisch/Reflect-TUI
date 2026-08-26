//! `ChatWidget` 的流式转写更新。
//!
//! 本模块负责助手、计划和推理的增量更新，包括流式尾随单元格、提交节拍以及中断延迟处理。

use super::*;

impl ChatWidget {
    pub(super) fn restore_reasoning_status_header(&mut self) {
        if self.reasoning_header.is_none() {
            self.reasoning_header = extract_first_bold(&self.reasoning_buffer);
        }
        if let Some(header) = self.reasoning_header.clone() {
            self.status_state.terminal_title_status_kind = TerminalTitleStatusKind::Thinking;
            self.set_status_header(header);
        } else if self.bottom_pane.is_task_running() {
            self.status_state.terminal_title_status_kind = TerminalTitleStatusKind::Working;
            self.set_status_header(String::from("Working"));
        }
    }

    pub(super) fn flush_answer_stream_with_separator(&mut self) {
        let had_stream_controller = self.stream_controller.is_some();
        if let Some(mut controller) = self.stream_controller.take() {
            let scrollback_reflow = if controller.has_live_tail() {
                crate::tui_core::app_event::ConsolidationScrollbackReflow::Required
            } else {
                crate::tui_core::app_event::ConsolidationScrollbackReflow::IfResizeReflowRan
            };
            self.clear_active_stream_tail();
            let (cell, source) = controller.finalize();
            // 与换行符触发的流式提交行为保持一致：当助手输出准备提交到历史时，隐藏内联状态行，以便转写内容替换它。
            if cell.is_some() {
                self.bottom_pane.hide_status_indicator();
            }
            let deferred_history_cell = if scrollback_reflow
                == crate::tui_core::app_event::ConsolidationScrollbackReflow::Required
            {
                cell
            } else {
                if let Some(cell) = cell {
                    self.add_boxed_history(cell);
                }
                None
            };
            // 将连续的流式 AgentMessageCells 合并为单个 AgentMarkdownCell，使其能够在 resize 时从源重新渲染。
            if let Some(source) = source {
                let source =
                    parse_assistant_markdown(&source, self.config.cwd.as_path()).visible_markdown;
                let inline_visualization_context = self.thread_id.and_then(|thread_id| {
                    crate::tui_core::inline_visualization::InlineVisualizationContext::from_config(
                        self.config.cwd.as_path(),
                        thread_id,
                    )
                });
                self.note_stream_consolidation_queued();
                self.app_event_tx.send(AppEvent::ConsolidateAgentMessage {
                    source,
                    cwd: self.config.cwd.to_path_buf(),
                    inline_visualization_context,
                    scrollback_reflow,
                    deferred_history_cell,
                });
            }
        }
        self.adaptive_chunking.reset();
        if had_stream_controller && self.stream_controllers_idle() {
            self.app_event_tx.send(AppEvent::StopCommitAnimation);
        }
        if had_stream_controller {
            self.request_pending_usage_output_insertion_after_stream_shutdown();
        }
    }

    pub(super) fn stream_controllers_idle(&self) -> bool {
        self.stream_controller
            .as_ref()
            .map(|controller| controller.queued_lines() == 0)
            .unwrap_or(true)
            && self
                .plan_stream_controller
                .as_ref()
                .map(|controller| controller.queued_lines() == 0)
                .unwrap_or(true)
    }

    /// 仅在以下条件同时满足时才恢复状态指示器：解说评论的完成事件待处理、轮次仍在运行，且所有流式队列均已清空。
    ///
    /// 该门控可在正常输出仍处于活跃流式状态时避免闪烁，同时仍然在解说评论块结束而轮次本身尚未结束时，恢复可见的"工作中"提示。
    pub(super) fn maybe_restore_status_indicator_after_stream_idle(&mut self) {
        if !self.status_state.pending_status_indicator_restore
            || !self.bottom_pane.is_task_running()
            || !self.stream_controllers_idle()
        {
            return;
        }

        self.bottom_pane.ensure_status_indicator();
        self.set_status(
            self.status_state.current_status.header.clone(),
            self.status_state.current_status.details.clone(),
            StatusDetailsCapitalization::Preserve,
            self.status_state.current_status.details_max_lines,
        );
        self.status_state.pending_status_indicator_restore = false;
    }

    pub(super) fn finalize_completed_assistant_message(&mut self, message: Option<&str>) {
        // 如果当前存在 stream_controller，则最终的消息载荷是多余的，因为可见内容已通过增量累积完成。
        if self.stream_controller.is_none()
            && let Some(message) = message
            && !message.is_empty()
        {
            self.handle_streaming_delta(message.to_string());
        }
        self.flush_answer_stream_with_separator();
        self.handle_stream_finished();
        self.request_redraw();
    }

    pub(super) fn on_agent_message_delta(&mut self, delta: String) {
        self.handle_streaming_delta(delta);
    }

    pub(super) fn on_plan_delta(&mut self, delta: String) {
        if self.active_mode_kind() != ModeKind::Plan {
            return;
        }
        if !self.transcript.plan_item_active {
            self.transcript.plan_item_active = true;
            self.transcript.plan_delta_buffer.clear();
        }
        self.transcript.plan_delta_buffer.push_str(&delta);
        if self.plan_stream_controller.is_none() {
            // 在开始计划流之前，先刷新所有活动的 exec 单元格组。
            self.flush_unified_exec_wait_streak();
            self.flush_active_cell();
            self.plan_stream_controller = Some(PlanStreamController::new(
                self.current_stream_width(/*reserved_cols*/ 4),
                self.config.cwd.as_path(),
                self.history_render_mode(),
            ));
        }
        if let Some(controller) = self.plan_stream_controller.as_mut()
            && controller.push(&delta)
        {
            self.app_event_tx.send(AppEvent::StartCommitAnimation);
            self.run_catch_up_commit_tick();
        }
        // 未结束的源由控制器缓冲，无法改变可见的尾随内容。
        if delta.contains('\n') && self.sync_active_stream_tail() {
            self.request_redraw();
        }
    }

    pub(super) fn on_plan_item_completed(&mut self, text: String) {
        let streamed_plan = self.transcript.plan_delta_buffer.trim().to_string();
        let plan_text = if text.trim().is_empty() {
            streamed_plan
        } else {
            text
        };
        if !plan_text.trim().is_empty() {
            self.record_agent_markdown(&plan_text);
            self.transcript.latest_proposed_plan_markdown = Some(plan_text.clone());
        }
        // 计划提交的节拍可能会隐藏状态行；记录是否已对计划进行流式输出，以便完成时在流式队列空闲后恢复状态行。
        let should_restore_after_stream = self.plan_stream_controller.is_some();
        self.transcript.plan_delta_buffer.clear();
        self.transcript.plan_item_active = false;
        self.transcript.saw_plan_item_this_turn = true;
        let (finalized_streamed_cell, consolidated_plan_source) =
            if let Some(mut controller) = self.plan_stream_controller.take() {
                let had_live_tail = controller.has_live_tail();
                self.clear_active_stream_tail();
                let (cell, source) = controller.finalize();
                if had_live_tail {
                    (None, source)
                } else {
                    (cell, source)
                }
            } else {
                (None, None)
            };
        if let Some(cell) = finalized_streamed_cell {
            self.add_boxed_history(cell);
            // TODO: 如果移除计划流式输出，或者需要协调流式内容与最终内容之间的不一致，
            // 则用最终的计划条目文本替换流式输出。
            if let Some(source) = consolidated_plan_source {
                self.note_stream_consolidation_queued();
                self.app_event_tx
                    .send(AppEvent::ConsolidateProposedPlan(source));
            }
        } else if !plan_text.is_empty() {
            self.add_to_history(history_cell::new_proposed_plan(
                plan_text,
                self.config.cwd.as_path(),
            ));
        } else if let Some(source) = consolidated_plan_source {
            self.note_stream_consolidation_queued();
            self.app_event_tx
                .send(AppEvent::ConsolidateProposedPlan(source));
        }
        if should_restore_after_stream {
            self.status_state.pending_status_indicator_restore = true;
            self.maybe_restore_status_indicator_after_stream_idle();
            self.request_pending_usage_output_insertion_after_stream_shutdown();
        }
    }

    pub(super) fn on_agent_reasoning_delta(&mut self, delta: String) {
        // 对于推理增量，不流式写入历史。累积当前推理块，
        // 并提取第一个加粗元素（位于 **/** 之间）作为分块标题。将该标题显示为状态。
        self.reasoning_buffer.push_str(&delta);

        if self.safety_buffering_is_waiting() {
            return;
        }

        if self.unified_exec_wait_streak.is_some() {
            // 统一 exec 等待应优先于从推理派生的状态标题。
            return;
        }

        if self.reasoning_header.is_none() {
            self.reasoning_header = extract_first_bold(&self.reasoning_buffer);
        }
        let Some(header) = self.reasoning_header.as_deref() else {
            // 尚未提取到粗体标题时的兜底：保持现有标题不变。
            return;
        };

        let status = &self.status_state.current_status;
        if self.status_state.terminal_title_status_kind == TerminalTitleStatusKind::Thinking
            && status.header == header
            && status.details.is_none()
            && status.details_max_lines == STATUS_DETAILS_DEFAULT_MAX_LINES
            && self
                .bottom_pane
                .status_widget()
                .is_none_or(|status| status.header() == header)
        {
            return;
        }

        // 将 shimmer 标题更新为提取到的推理块标题。
        let header = header.to_string();
        self.status_state.terminal_title_status_kind = TerminalTitleStatusKind::Thinking;
        if !self.set_status_header(header) {
            self.request_redraw();
        }
    }

    pub(super) fn on_agent_reasoning_final(&mut self) {
        // 推理块结束时，记录仅用于 transcript 的内容。
        if !self.reasoning_buffer.is_empty() {
            self.reasoning_summary_parts
                .push(std::mem::take(&mut self.reasoning_buffer));
        }
        if !self.reasoning_summary_parts.is_empty() {
            let reasoning_parts = std::mem::take(&mut self.reasoning_summary_parts);
            let cell = history_cell::new_reasoning_summary_block(
                reasoning_parts,
                self.config.cwd.as_path(),
            );
            self.add_boxed_history(cell);
        }
        self.reasoning_buffer.clear();
        self.reasoning_header = None;
        self.reasoning_summary_parts.clear();
        self.request_redraw();
    }

    pub(super) fn on_reasoning_section_break(&mut self) {
        // 开启新的推理块用于标题提取，并累积 transcript 内容。
        if !self.reasoning_buffer.is_empty() {
            self.reasoning_summary_parts
                .push(std::mem::take(&mut self.reasoning_buffer));
        }
        self.reasoning_header = None;
    }

    pub(super) fn on_stream_error(&mut self, message: String, additional_details: Option<String>) {
        self.status_state.remember_retry_status_header();
        self.bottom_pane.ensure_status_indicator();
        self.status_state.terminal_title_status_kind = TerminalTitleStatusKind::Thinking;
        self.set_status(
            message,
            additional_details,
            StatusDetailsCapitalization::CapitalizeFirst,
            STATUS_DETAILS_DEFAULT_MAX_LINES,
        );
    }

    /// 处理 `AgentMessage` 回合项的完成。
    ///
    /// 评论性内容完成时设置延迟恢复标记，使状态行在流队列空闲后恢复。
    /// 最终答案完成（或遗留模型的阶段缺失）时清除该标记以保持历史行为。
    pub(super) fn on_agent_message_item_completed(
        &mut self,
        item: AgentMessageItem,
        from_replay: bool,
    ) {
        let mut message = String::new();
        for content in &item.content {
            match content {
                AgentMessageContent::Text { text } => message.push_str(text),
            }
        }
        let parsed = parse_assistant_markdown(&message, self.config.cwd.as_path());
        self.finalize_completed_assistant_message(
            (!parsed.visible_markdown.is_empty()).then_some(parsed.visible_markdown.as_str()),
        );
        if matches!(item.phase, Some(MessagePhase::FinalAnswer) | None)
            && !parsed.visible_markdown.is_empty()
        {
            self.record_agent_markdown(&parsed.visible_markdown);
        }
        if !from_replay
            && let Some(cwd) = parsed.last_created_branch_cwd()
            && let Some(thread_id) = self.thread_id
            && let Some(runner) = self.workspace_command_runner.clone()
        {
            let cwd = PathBuf::from(cwd);
            let tx = self.app_event_tx.clone();
            tokio::spawn(async move {
                if let Some(branch) =
                    crate::tui_core::branch_summary::current_branch_name(runner.as_ref(), &cwd)
                        .await
                {
                    tx.send(AppEvent::SyncThreadGitBranch { thread_id, branch });
                }
            });
        }
        self.status_state.pending_status_indicator_restore = match item.phase {
            // 不支持前置内容的模型只在回合完成时输出 AgentMessageItem。
            Some(MessagePhase::FinalAnswer) | None => !self.input_queue.pending_steers.is_empty(),
            Some(MessagePhase::Commentary) => true,
        };
        self.maybe_restore_status_indicator_after_stream_idle();
    }

    /// 流式提交的周期性 tick。平滑模式下保持单行节奏，
    /// 追赶模式则批量排空以降低队列延迟。
    pub(crate) fn on_commit_tick(&mut self) {
        self.run_commit_tick();
    }

    /// 运行常规的周期性提交 tick。
    pub(super) fn run_commit_tick(&mut self) {
        self.run_commit_tick_with_scope(CommitTickScope::AnyMode);
    }

    /// 仅当追赶模式激活时运行一次机会性提交 tick。
    pub(super) fn run_catch_up_commit_tick(&mut self) {
        self.run_commit_tick_with_scope(CommitTickScope::CatchUpOnly);
    }

    /// 针对当前流队列快照运行一次提交 tick。
    ///
    /// `scope` 控制本次调用是可在平滑模式下提交，还是仅在追赶
    /// 模式激活时提交。流式输出进行中会隐藏状态行，避免出现重复的
    /// “进行中”提示。恢复动作单独受控，仅在评论完成后且流队列
    /// 空闲时才重新显示状态行。
    pub(super) fn run_commit_tick_with_scope(&mut self, scope: CommitTickScope) {
        let now = Instant::now();
        let outcome = run_commit_tick(
            &mut self.adaptive_chunking,
            self.stream_controller.as_mut(),
            self.plan_stream_controller.as_mut(),
            scope,
            now,
        );
        for cell in outcome.cells {
            self.bottom_pane.hide_status_indicator();
            self.add_boxed_history(cell);
        }
        if scope == CommitTickScope::AnyMode || outcome.has_controller {
            self.sync_active_stream_tail();
        }

        if outcome.has_controller && outcome.all_idle {
            self.maybe_restore_status_indicator_after_stream_idle();
            self.app_event_tx.send(AppEvent::StopCommitAnimation);
        }

        if self.turn_lifecycle.agent_turn_running {
            self.refresh_runtime_metrics();
        }
    }

    pub(super) fn flush_interrupt_queue(&mut self) {
        let mut mgr = std::mem::take(&mut self.interrupts);
        mgr.flush_all(self);
        self.interrupts = mgr;
    }

    /// 将生命周期负载移入中断队列或其即时处理器。
    #[inline]
    pub(super) fn defer_or_handle<T>(
        &mut self,
        payload: T,
        push: impl FnOnce(&mut InterruptManager, T),
        handle: impl FnOnce(&mut Self, T),
    ) {
        // 在排队的中断之间保持确定性 FIFO：一旦因活动写入周期
        // 有内容入队，就持续排队直到队列被排空，避免乱序
        // （例如 ExecEnd 先于 ExecBegin）。
        if self.stream_controller.is_some() || !self.interrupts.is_empty() {
            push(&mut self.interrupts, payload);
        } else {
            handle(self, payload);
        }
    }

    pub(super) fn handle_stream_finished(&mut self) {
        if self.task_complete_pending {
            self.bottom_pane.hide_status_indicator();
            self.task_complete_pending = false;
        }
        // 流完成表示刚插入了非 exec 内容。
        self.flush_interrupt_queue();
    }

    #[inline]
    pub(super) fn handle_streaming_delta(&mut self, delta: String) {
        if !delta.is_empty() {
            self.mark_safety_buffering_agent_message_started();
        }
        if self.stream_controller.is_none() {
            // 启动 agent 流之前，冲刷仍处于激活状态的 exec 单元格分组。
            self.flush_unified_exec_wait_streak();
            self.flush_active_cell();
            // 若上一回合插入了非流式历史（exec 输出、补丁状态、MCP
            // 调用），在开始下一条流式助手消息之前渲染一个分隔符。
            if self.transcript.needs_final_message_separator && self.transcript.had_work_activity {
                self.add_to_history(history_cell::FinalMessageSeparator::new(
                    /*elapsed_seconds*/ None, /*runtime_metrics*/ None,
                ));
                self.transcript.needs_final_message_separator = false;
            } else if self.transcript.needs_final_message_separator {
                // 即使不显示分隔符也重置标记（没有产生工作）
                self.transcript.needs_final_message_separator = false;
            }
            let inline_visualization_context = self.thread_id.and_then(|thread_id| {
                crate::tui_core::inline_visualization::InlineVisualizationContext::from_config(
                    self.config.cwd.as_path(),
                    thread_id,
                )
            });
            self.stream_controller = Some(StreamController::new_with_inline_visualizations(
                self.current_stream_width(/*reserved_cols*/ 2),
                self.config.cwd.as_path(),
                self.history_render_mode(),
                inline_visualization_context,
            ));
        }
        if let Some(controller) = self.stream_controller.as_mut()
            && controller.push(&delta)
        {
            self.app_event_tx.send(AppEvent::StartCommitAnimation);
            self.run_catch_up_commit_tick();
        }
        // 未结束的源由控制器缓冲，无法改变可见的尾随内容。
        if delta.contains('\n') && self.sync_active_stream_tail() {
            self.request_redraw();
        }
    }

    pub(super) fn active_cell_is_stream_tail(&self) -> bool {
        self.transcript.active_cell.as_ref().is_some_and(|cell| {
            cell.as_any().is::<history_cell::StreamingAgentTailCell>()
                || cell.as_any().is::<history_cell::StreamingPlanTailCell>()
        })
    }

    pub(super) fn has_active_stream_tail(&self) -> bool {
        (self.stream_controller.is_some() || self.plan_stream_controller.is_some())
            && self.active_cell_is_stream_tail()
    }

    pub(super) fn sync_active_stream_tail(&mut self) -> bool {
        if let Some(controller) = self.stream_controller.as_ref() {
            let tail_lines = controller.current_tail_lines();
            if tail_lines.is_empty() {
                return self.clear_active_stream_tail();
            }

            self.bottom_pane.hide_status_indicator();
            let cell = history_cell::StreamingAgentTailCell::new(
                tail_lines,
                controller.tail_starts_stream(),
            );
            if self
                .transcript
                .active_cell
                .as_ref()
                .and_then(|active| {
                    active
                        .as_any()
                        .downcast_ref::<history_cell::StreamingAgentTailCell>()
                })
                .is_some_and(|active| active == &cell)
            {
                return false;
            }
            self.transcript.active_cell = Some(Box::new(cell));
            self.bump_active_cell_revision();
            return true;
        }

        if let Some(controller) = self.plan_stream_controller.as_ref() {
            let tail_lines = controller.current_tail_display_lines();
            if tail_lines.is_empty() {
                return self.clear_active_stream_tail();
            }

            self.bottom_pane.hide_status_indicator();
            let cell = history_cell::StreamingPlanTailCell::new(
                tail_lines,
                !controller.tail_starts_stream(),
            );
            if self
                .transcript
                .active_cell
                .as_ref()
                .and_then(|active| {
                    active
                        .as_any()
                        .downcast_ref::<history_cell::StreamingPlanTailCell>()
                })
                .is_some_and(|active| active == &cell)
            {
                return false;
            }
            self.transcript.active_cell = Some(Box::new(cell));
            self.bump_active_cell_revision();
            return true;
        }

        self.clear_active_stream_tail()
    }

    pub(super) fn clear_active_stream_tail(&mut self) -> bool {
        if self.active_cell_is_stream_tail() {
            self.transcript.active_cell = None;
            self.bump_active_cell_revision();
            return true;
        }
        false
    }
}
