//! 回溯与 transcript overlay 事件路由。
//!
//! 本文件负责回溯模式(transcript overlay 中的 Esc/Enter 导航),
//! 同时也协调 transcript overlay 的关键渲染边界。
//!
//! 总体目标:保持主聊天视图与 transcript overlay 同步,同时允许用户在源保留
//! (source-preserving) 分支上编辑先前的 prompt。确认选择会在选定的回合之前
//! fork,并在新 composer 中恢复其 prompt。
//!
//! 回溯以一个小型状态机运作:
//! - 主视图中的第一次 `Esc` 启用(prime)该功能,并捕获一个基础 thread id。
//! - 后续 `Esc` 打开 transcript overlay(`Ctrl+T`),并在有可复用 prompt 时
//!   高亮一条用户消息。
//! - `Enter` 请求在选定 prompt 之前执行 fork,并重新打开它以便编辑。
//!
//! transcript overlay(`Ctrl+T`)渲染已提交的 transcript 单元,以及由当前正在进行
//! 的 `ChatWidget.active_cell` 派生的、仅用于渲染的 live tail。
//!
//! 该 live tail 在 `Overlay::Transcript` 的 `TuiEvent::Draw` 处理过程中通过
//! 向 `ChatWidget` 索取 active cell 缓存键与 transcript 行,并把它们传进
//! `TranscriptOverlay::sync_live_tail` 来保持同步。这保证了 overlay 既能
//! 反映已提交历史,也能反映进行中的活动,同时不改变 flush 或 coalescing 行为。

use std::any::TypeId;
use std::sync::Arc;

use crate::app_server_protocol::ThreadItem;
use crate::app_server_protocol::Turn;
use crate::app_server_protocol::TurnStatus;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::models::local_image_label_text;
use crate::tui_core::app::App;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::bottom_pane::LocalImageAttachment;
use crate::tui_core::chatwidget::ChatWidget;
use crate::tui_core::chatwidget::UserMessage;
use crate::tui_core::chatwidget::mention_bindings_from_user_inputs;
#[cfg(test)]
use crate::tui_core::history_cell::AgentMessageCell;
use crate::tui_core::history_cell::SessionInfoCell;
use crate::tui_core::history_cell::UserHistoryCell;
use crate::tui_core::pager_overlay::Overlay;
use crate::tui_core::tui;
use crate::tui_core::tui::TuiEvent;
use color_eyre::eyre::Result;
use color_eyre::eyre::bail;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;

const NO_PREVIOUS_MESSAGE_TO_EDIT: &str = "No previous message to edit.";
pub(crate) const SIDE_EDIT_PREVIOUS_UNAVAILABLE_MESSAGE: &str =
    "Editing previous prompts is unavailable in side conversations.";

/// 聚合 App 使用的所有回溯相关状态。
#[derive(Default)]
pub(crate) struct BacktrackState {
    /// Esc 在主视图中已经预热回溯模式时为 true。
    pub(crate) primed: bool,
    /// 正在检视其 transcript 的基础 thread 的 session id。
    ///
    /// 如果当前 thread 发生变化, 回溯选择将失效, 必须忽略。
    pub(crate) base_id: Option<ThreadId>,
    /// 当前高亮用户消息的索引。
    ///
    /// 这是「自上次会话开始以来的用户消息」过滤视图的索引,
    /// 而**不是** `transcript_cells` 的索引。`usize::MAX` 表示「无选中」。
    pub(crate) nth_user_message: usize,
    /// transcript overlay 正在显示回溯预览时为 true。
    pub(crate) overlay_preview_active: bool,
}

/// 用户可见的回溯选择,可在源保留分支上重新打开。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BacktrackSelection {
    pub(crate) thread_id: ThreadId,
    /// 选定的用户消息,自最近一次会话开始计数。
    pub(crate) nth_user_message: usize,
    pub(crate) prompt: UserMessage,
}

impl App {
    /// 在 transcript overlay 处于活动状态时路由 overlay 事件。
    ///
    /// 如果回溯预览处于活动状态, Esc / Left 后退选择, Right 前进选择, Enter 确认。
    /// 否则 Esc 开启预览模式, 其他事件全部转发给 overlay。
    pub(crate) async fn handle_backtrack_overlay_event(
        &mut self,
        tui: &mut tui::Tui,
        event: TuiEvent,
    ) -> Result<bool> {
        if self.backtrack.overlay_preview_active {
            match event {
                TuiEvent::Key(KeyEvent {
                    code: KeyCode::Esc,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) => {
                    self.overlay_step_backtrack(tui, event)?;
                    Ok(true)
                }
                TuiEvent::Key(KeyEvent {
                    code: KeyCode::Left,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) => {
                    self.overlay_step_backtrack(tui, event)?;
                    Ok(true)
                }
                TuiEvent::Key(KeyEvent {
                    code: KeyCode::Right,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) => {
                    self.overlay_step_backtrack_forward(tui, event)?;
                    Ok(true)
                }
                TuiEvent::Key(KeyEvent {
                    code: KeyCode::Enter,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    self.overlay_confirm_backtrack(tui);
                    Ok(true)
                }
                _ => {
                    self.overlay_forward_event(tui, event)?;
                    Ok(true)
                }
            }
        } else if let TuiEvent::Key(KeyEvent {
            code: KeyCode::Esc,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        }) = event
        {
            // 转录覆盖层中的第一个 Esc：从最新用户消息开始回溯预览。
            self.begin_overlay_backtrack_preview(tui);
            Ok(true)
        } else {
            // 不在回溯模式：将事件转发给覆盖层组件。
            self.overlay_forward_event(tui, event)?;
            Ok(true)
        }
    }

    /// 在没有任何 overlay 打开时, 处理用于回溯的全局 Esc 按键。
    pub(crate) fn handle_backtrack_esc_key(&mut self, tui: &mut tui::Tui) {
        if !self.chat_widget.composer_is_empty() {
            return;
        }

        if !self.backtrack.primed {
            self.prime_backtrack();
        } else if self.overlay.is_none() {
            self.open_backtrack_preview(tui);
        } else if self.backtrack.overlay_preview_active {
            self.step_backtrack_and_highlight(tui);
        }
    }

    /// 在选定的 prompt 之前请求一个源保留分支。
    pub(crate) fn apply_backtrack_selection(&mut self, selection: BacktrackSelection) {
        if self.chat_widget.side_conversation_active() {
            self.reset_backtrack_state();
            self.chat_widget
                .add_error_message(SIDE_EDIT_PREVIOUS_UNAVAILABLE_MESSAGE.to_string());
            return;
        }

        if self.chat_widget.thread_id() != Some(selection.thread_id) {
            return;
        }

        self.app_event_tx.send(AppEvent::ForkSessionForPromptEdit {
            thread_id: selection.thread_id,
            nth_user_message: selection.nth_user_message,
            prompt: selection.prompt,
        });
    }

    pub(crate) fn restore_backtrack_prompt_after_branch_error(
        &mut self,
        prompt: UserMessage,
        err: impl std::fmt::Display,
    ) {
        self.chat_widget.restore_user_message_to_composer(prompt);
        self.chat_widget.add_error_message(format!(
            "Failed to branch before the selected prompt: {err}"
        ));
    }

    /// 打开 transcript overlay(进入备用屏并显示完整 transcript)。
    pub(crate) fn open_transcript_overlay(&mut self, tui: &mut tui::Tui) {
        let _ = tui.enter_alt_screen();
        self.overlay = Some(Overlay::new_transcript(
            self.transcript_cells.clone(),
            self.keymap.pager.clone(),
        ));
        tui.frame_requester().schedule_frame();
    }

    /// 关闭 transcript overlay 并恢复常规 UI。
    pub(crate) fn close_transcript_overlay(&mut self, tui: &mut tui::Tui) {
        let _ = tui.leave_alt_screen();
        let was_backtrack = self.backtrack.overlay_preview_active;
        if !self.deferred_history_lines.is_empty() {
            let lines = std::mem::take(&mut self.deferred_history_lines);
            tui.insert_history_hyperlink_lines_with_wrap_policy(
                lines,
                self.history_line_wrap_policy(),
            );
        }
        self.overlay = None;
        self.backtrack.overlay_preview_active = false;
        tui.frame_requester().schedule_frame();
        if was_backtrack {
            // 确保覆盖层关闭时（例如通过 'q'）回溯状态被完全重置。
            self.reset_backtrack_state();
        }
    }

    /// 初始化回溯状态并显示 composer 提示。
    fn prime_backtrack(&mut self) {
        self.backtrack.primed = true;
        self.backtrack.nth_user_message = usize::MAX;
        self.backtrack.base_id = self.chat_widget.thread_id().map(|id| id.to_string());
        if has_backtrack_target(&self.transcript_cells) {
            self.chat_widget.show_esc_backtrack_hint();
        }
    }

    /// 打开 overlay 并开始回溯预览流程(第一步 + 高亮)。
    fn open_backtrack_preview(&mut self, tui: &mut tui::Tui) {
        if !has_backtrack_target(&self.transcript_cells) {
            self.reset_backtrack_state();
            self.chat_widget
                .add_info_message(NO_PREVIOUS_MESSAGE_TO_EDIT.to_string(), /*hint*/ None);
            tui.frame_requester().schedule_frame();
            return;
        }

        self.open_transcript_overlay(tui);
        self.backtrack.overlay_preview_active = true;
        // 编辑器被覆盖层隐藏；清除其提示。
        self.chat_widget.clear_esc_backtrack_hint();
        self.step_backtrack_and_highlight(tui);
    }

    /// 当 overlay 已经打开时, 开启预览模式并选择最近的用户消息。
    fn begin_overlay_backtrack_preview(&mut self, tui: &mut tui::Tui) {
        if !has_backtrack_target(&self.transcript_cells) {
            self.close_transcript_overlay(tui);
            self.chat_widget
                .add_info_message(NO_PREVIOUS_MESSAGE_TO_EDIT.to_string(), /*hint*/ None);
            tui.frame_requester().schedule_frame();
            return;
        }

        self.backtrack.primed = true;
        self.backtrack.base_id = self.chat_widget.thread_id().map(|id| id.to_string());
        self.backtrack.overlay_preview_active = true;
        let count = user_count(&self.transcript_cells);
        if let Some(last) = count.checked_sub(1) {
            self.apply_backtrack_selection_internal(last);
        }
        tui.frame_requester().schedule_frame();
    }

    /// 将选择向后移动到上一条用户消息并更新 overlay。
    fn step_backtrack_and_highlight(&mut self, tui: &mut tui::Tui) {
        let count = user_count(&self.transcript_cells);
        if count == 0 {
            return;
        }

        let last_index = count.saturating_sub(1);
        let next_selection = if self.backtrack.nth_user_message == usize::MAX {
            last_index
        } else if self.backtrack.nth_user_message == 0 {
            0
        } else {
            self.backtrack
                .nth_user_message
                .saturating_sub(1)
                .min(last_index)
        };

        self.apply_backtrack_selection_internal(next_selection);
        tui.frame_requester().schedule_frame();
    }

    /// 将选择向前移动到下一条用户消息并更新 overlay。
    fn step_forward_backtrack_and_highlight(&mut self, tui: &mut tui::Tui) {
        let count = user_count(&self.transcript_cells);
        if count == 0 {
            return;
        }

        let last_index = count.saturating_sub(1);
        let next_selection = if self.backtrack.nth_user_message == usize::MAX {
            last_index
        } else {
            self.backtrack
                .nth_user_message
                .saturating_add(1)
                .min(last_index)
        };

        self.apply_backtrack_selection_internal(next_selection);
        tui.frame_requester().schedule_frame();
    }

    /// 将计算出的回溯选择应用到 overlay 和内部计数器。
    fn apply_backtrack_selection_internal(&mut self, nth_user_message: usize) {
        if let Some(cell_idx) = nth_user_position(&self.transcript_cells, nth_user_message) {
            self.backtrack.nth_user_message = nth_user_message;
            if let Some(Overlay::Transcript(t)) = &mut self.overlay {
                t.set_highlight_cell(Some(cell_idx));
            }
        } else {
            self.backtrack.nth_user_message = usize::MAX;
            if let Some(Overlay::Transcript(t)) = &mut self.overlay {
                t.set_highlight_cell(/*cell*/ None);
            }
        }
    }

    /// 将事件转发给覆盖层，并在完成时关闭它。
    ///
    /// 转录覆盖层的绘制路径比较特殊，因为在活动单元格
    /// 仍在流式传输或修改时，覆盖层应与主视口匹配。
    ///
    /// `TranscriptOverlay` 拥有已提交的转录单元格，而 `ChatWidget` 拥有
    /// 当前进行中的活动单元格（通常是合并的 exec/tool 组）。绘制期间，我们将该
    /// 进行中的单元格作为缓存的、只渲染的实时尾部追加，使 `Ctrl+T` 在
    /// 后续刷新边界之前不会"丢失"工具调用。
    ///
    /// 此逻辑位于此处（而非覆盖层组件内部），因为 `ChatWidget` 是
    /// 活动单元格及其缓存失效键的事实来源，且 `App` 拥有
    /// 覆盖层生命周期和动画的帧调度。
    fn overlay_forward_event(&mut self, tui: &mut tui::Tui, event: TuiEvent) -> Result<()> {
        if matches!(&event, TuiEvent::Draw | TuiEvent::Resize(_, _))
            && let Some(Overlay::Transcript(t)) = &mut self.overlay
        {
            let active_key = self.chat_widget.active_cell_transcript_key();
            let chat_widget = &self.chat_widget;
            tui.draw(u16::MAX, |frame| {
                let width = frame.area().width.max(1);
                t.sync_live_tail(width, active_key, |w| {
                    chat_widget.active_cell_transcript_hyperlink_lines(w)
                });
                t.render(frame.area(), frame.buffer);
            })?;
            let close_overlay = t.is_done();
            if !close_overlay
                && active_key.is_some_and(|key| key.animation_tick.is_some())
                && t.is_scrolled_to_bottom()
            {
                tui.frame_requester()
                    .schedule_frame_in(std::time::Duration::from_millis(50));
            }
            if close_overlay {
                self.close_transcript_overlay(tui);
                tui.frame_requester().schedule_frame();
            }
            return Ok(());
        }

        if let Some(overlay) = &mut self.overlay {
            overlay.handle_event(tui, event)?;
            if overlay.is_done() {
                self.close_transcript_overlay(tui);
                tui.frame_requester().schedule_frame();
            }
        }
        Ok(())
    }

    /// 处理覆盖层回溯预览中的 Enter：确认选择并重置状态。
    fn overlay_confirm_backtrack(&mut self, tui: &mut tui::Tui) {
        let nth_user_message = self.backtrack.nth_user_message;
        let selection = self.backtrack_selection(nth_user_message);
        self.close_transcript_overlay(tui);
        if let Some(selection) = selection {
            self.apply_backtrack_selection(selection);
            tui.frame_requester().schedule_frame();
        }
    }

    /// 处理覆盖层回溯预览中的 Esc：已激活则步进选择，否则转发。
    fn overlay_step_backtrack(&mut self, tui: &mut tui::Tui, event: TuiEvent) -> Result<()> {
        if self.backtrack.base_id.is_some() {
            self.step_backtrack_and_highlight(tui);
        } else {
            self.overlay_forward_event(tui, event)?;
        }
        Ok(())
    }

    /// 处理覆盖层回溯预览中的 Right：已激活则向前步进选择，否则转发。
    fn overlay_step_backtrack_forward(
        &mut self,
        tui: &mut tui::Tui,
        event: TuiEvent,
    ) -> Result<()> {
        if self.backtrack.base_id.is_some() {
            self.step_forward_backtrack_and_highlight(tui);
        } else {
            self.overlay_forward_event(tui, event)?;
        }
        Ok(())
    }

    /// 从主视图（无可见覆盖层）确认已激活的回溯。
    /// 从选定的用户消息计算 prompt 状态。
    pub(crate) fn confirm_backtrack_from_main(&mut self) -> Option<BacktrackSelection> {
        let selection = self.backtrack_selection(self.backtrack.nth_user_message);
        self.reset_backtrack_state();
        selection
    }

    /// 清除所有回溯相关状态和编辑器提示。
    pub(crate) fn reset_backtrack_state(&mut self) {
        self.backtrack.primed = false;
        self.backtrack.base_id = None;
        self.backtrack.nth_user_message = usize::MAX;
        // 以防提示由于某种原因仍可见（例如与覆盖层打开/关闭的竞态）。
        self.chat_widget.clear_esc_backtrack_hint();
    }

    fn backtrack_selection(&self, nth_user_message: usize) -> Option<BacktrackSelection> {
        let base_id = self.backtrack.base_id.clone()?;
        if self
            .chat_widget
            .thread_id()
            .map(|id| id.to_string())
            .as_ref()
            != Some(&base_id)
        {
            return None;
        }

        let selected = nth_user_position(&self.transcript_cells, nth_user_message)
            .and_then(|idx| self.transcript_cells.get(idx))
            .and_then(|cell| cell.as_any().downcast_ref::<UserHistoryCell>())?;
        let local_images = selected
            .local_image_paths
            .iter()
            .enumerate()
            .map(|(index, path)| LocalImageAttachment {
                placeholder: local_image_label_text(index + 1),
                path: path.clone(),
            })
            .collect();

        Some(BacktrackSelection {
            thread_id: crate::protocol_compat::ThreadId::from_string(&base_id).unwrap_or_default(),
            nth_user_message,
            prompt: UserMessage {
                text: selected.message.clone(),
                local_images,
                remote_image_urls: selected.remote_image_urls.clone(),
                text_elements: selected.text_elements.clone(),
                mention_bindings: Vec::new(),
            },
        })
    }
}

/// 查找包含选定转录 prompt 的已持久化回合。
///
/// 回放会隐藏 review prompt 和其他显示为空的输入，因此选定的序数必须
/// 在恢复其规范提及绑定之前针对相同的可见投影进行解析。
///
/// 一个回合在被 steer 时可以包含多条用户消息。只有其初始 prompt 可以
/// 被独立重新打开，因为 app-server 无法在回合中间分叉。
pub(crate) fn backtrack_fork_before_turn_id(
    turns: &[Turn],
    nth_user_message: usize,
    prompt: &mut UserMessage,
) -> Result<Option<String>> {
    let mut visible_user_messages_seen = 0_usize;
    let mut review_mode = false;
    for (turn_index, turn) in turns.iter().enumerate() {
        let hidden_nested_review_turn = turn_index
            .checked_sub(/*rhs*/ 1)
            .and_then(|index| turns.get(index))
            .is_some_and(|previous| is_hidden_nested_review_turn(previous, turn));
        let mut user_messages_in_turn = 0_usize;
        for item in &turn.items {
            let content = match item {
                ThreadItem::EnteredReviewMode { .. } => {
                    review_mode = true;
                    continue;
                }
                ThreadItem::ExitedReviewMode { .. } => {
                    review_mode = false;
                    continue;
                }
                ThreadItem::UserMessage { content, .. } => content,
                _ => continue,
            };
            let is_steer = user_messages_in_turn > 0;
            user_messages_in_turn = user_messages_in_turn.saturating_add(/*rhs*/ 1);
            if review_mode {
                continue;
            }

            let display = ChatWidget::user_message_display_from_inputs(content);
            if hidden_nested_review_turn {
                continue;
            }
            if display.message.trim().is_empty()
                && display.text_elements.is_empty()
                && display.local_images.is_empty()
                && display.remote_image_urls.is_empty()
            {
                continue;
            }
            if visible_user_messages_seen != nth_user_message {
                visible_user_messages_seen =
                    visible_user_messages_seen.saturating_add(/*rhs*/ 1);
                continue;
            }

            if is_steer {
                bail!("the selected prompt is a steer and cannot be branched independently");
            }
            if matches!(turn.status, TurnStatus::InProgress) {
                bail!("the selected prompt belongs to a turn that is still in progress");
            }

            let selected_local_images = prompt.local_images.iter().map(|image| &image.path);
            if prompt.text != display.message
                || prompt.text_elements != display.text_elements
                || prompt.remote_image_urls != display.remote_image_urls
                || !selected_local_images.eq(display.local_images.iter())
            {
                bail!("the selected transcript prompt no longer matches the persisted thread");
            }
            prompt.mention_bindings = mention_bindings_from_user_inputs(content, &display.message);

            return Ok((turn_index > 0).then(|| turn.id.clone()));
        }
    }

    bail!("the selected prompt was not found in the persisted thread")
}

/// 返回一个回合是否是带有重复 prompt 输入的重构内联 review 子回合。
pub(crate) fn is_hidden_nested_review_turn(previous: &Turn, turn: &Turn) -> bool {
    if previous.status != TurnStatus::Completed
        || turn.status != TurnStatus::Interrupted
        || turn.completed_at.is_some()
        || !previous
            .items
            .iter()
            .any(|item| matches!(item, ThreadItem::EnteredReviewMode { .. }))
        || !previous
            .items
            .iter()
            .any(|item| matches!(item, ThreadItem::ExitedReviewMode { .. }))
    {
        return false;
    }

    let mut user_messages = turn.items.iter().filter_map(|item| match item {
        ThreadItem::UserMessage { content, .. } => Some(content),
        _ => None,
    });
    matches!(
        (
            user_messages.next(),
            user_messages.next(),
            user_messages.next(),
        ),
        (Some(first), Some(second), None) if first == second
    )
}

pub(crate) fn user_count(cells: &[Arc<dyn crate::tui_core::history_cell::HistoryCell>]) -> usize {
    user_positions_iter(cells).count()
}

fn has_backtrack_target(cells: &[Arc<dyn crate::tui_core::history_cell::HistoryCell>]) -> bool {
    user_count(cells) > 0
}

fn nth_user_position(
    cells: &[Arc<dyn crate::tui_core::history_cell::HistoryCell>],
    nth: usize,
) -> Option<usize> {
    user_positions_iter(cells)
        .enumerate()
        .find_map(|(i, idx)| (i == nth).then_some(idx))
}

fn user_positions_iter(
    cells: &[Arc<dyn crate::tui_core::history_cell::HistoryCell>],
) -> impl Iterator<Item = usize> + '_ {
    let session_start_type = TypeId::of::<SessionInfoCell>();
    let user_type = TypeId::of::<UserHistoryCell>();
    let type_of =
        |cell: &Arc<dyn crate::tui_core::history_cell::HistoryCell>| cell.as_any().type_id();

    let start = cells
        .iter()
        .rposition(|cell| type_of(cell) == session_start_type)
        .map_or(0, |idx| idx + 1);

    cells
        .iter()
        .enumerate()
        .skip(start)
        .filter_map(move |(idx, cell)| (type_of(cell) == user_type).then_some(idx))
}

#[cfg(test)]
fn agent_group_count(cells: &[Arc<dyn crate::tui_core::history_cell::HistoryCell>]) -> usize {
    agent_group_positions_iter(cells).count()
}

#[cfg(test)]
fn agent_group_positions_iter(
    cells: &[Arc<dyn crate::tui_core::history_cell::HistoryCell>],
) -> impl Iterator<Item = usize> + '_ {
    let session_start_type = TypeId::of::<SessionInfoCell>();
    let type_of =
        |cell: &Arc<dyn crate::tui_core::history_cell::HistoryCell>| cell.as_any().type_id();

    let start = cells
        .iter()
        .rposition(|cell| type_of(cell) == session_start_type)
        .map_or(0, |idx| idx + 1);

    cells
        .iter()
        .enumerate()
        .skip(start)
        .filter_map(move |(idx, cell)| {
            let is_agent = cell.as_any().downcast_ref::<AgentMessageCell>().is_some();
            let is_copy_source_group = is_agent && !cell.is_stream_continuation();
            is_copy_source_group.then_some(idx)
        })
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
