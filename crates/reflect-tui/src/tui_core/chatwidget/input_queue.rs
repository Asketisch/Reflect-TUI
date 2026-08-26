//! `ChatWidget` 的排队用户输入与待处理转向状态。
//!
//! 本模块把可变输入队列集中在一起，使 `ChatWidget` 能
//! 围绕一个聚焦的 reducer 风格状态袋施加 UI/协议效果。

use std::collections::VecDeque;

use super::PendingSteer;
use super::QueuedUserMessage;
use super::UserMessage;
use super::UserMessageHistoryRecord;
use super::user_message_preview_text;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct PendingInputPreview {
    pub(super) queued_messages: Vec<String>,
    pub(super) pending_steers: Vec<String>,
    pub(super) rejected_steers: Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct InputQueueState {
    /// 回合进行中排队等待的用户输入。
    pub(super) queued_user_messages: VecDeque<QueuedUserMessage>,
    /// 排队用户消息的历史记录。`/goal` 等斜杠命令
    /// 渲染出的历史可能与提交给核心的文本不同，因此它与
    /// `queued_user_messages` 保持同步，缺失条目
    /// 按用户消息文本处理。
    pub(super) queued_user_message_history_records: VecDeque<UserMessageHistoryRecord>,
    /// 用户回合已提交给核心，但 `TurnStarted` 尚未到达。
    pub(super) user_turn_pending_start: bool,
    /// 试图转向非常规回合、必须先重试的用户消息。
    pub(super) rejected_steers_queue: VecDeque<UserMessage>,
    /// 被拒绝转向的历史记录。`/goal` 等斜杠命令可以
    /// 渲染出与提交给核心的文本不同的历史，因此它与
    /// `rejected_steers_queue` 保持同步，缺失条目按
    /// 用户消息文本处理。
    pub(super) rejected_steer_history_records: VecDeque<UserMessageHistoryRecord>,
    /// 已提交给核心但尚未写入历史的转向。
    pub(super) pending_steers: VecDeque<PendingSteer>,
    /// 设置后，下次中断应把全部待处理转向作为一轮
    /// 新用户回合重新提交，而不是恢复到 composer。
    pub(super) submit_pending_steers_after_interrupt: bool,
    pub(super) suppress_queue_autosend: bool,
}

impl InputQueueState {
    pub(super) fn has_queued_follow_up_messages(&self) -> bool {
        !self.rejected_steers_queue.is_empty() || !self.queued_user_messages.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.queued_user_messages.clear();
        self.queued_user_message_history_records.clear();
        self.user_turn_pending_start = false;
        self.rejected_steers_queue.clear();
        self.rejected_steer_history_records.clear();
        self.pending_steers.clear();
        self.submit_pending_steers_after_interrupt = false;
    }

    pub(super) fn preview(&self) -> PendingInputPreview {
        let queued_messages = self
            .queued_user_messages
            .iter()
            .enumerate()
            .map(|(idx, message)| {
                user_message_preview_text(
                    message,
                    self.queued_user_message_history_records.get(idx),
                )
            })
            .collect();
        let pending_steers = self
            .pending_steers
            .iter()
            .map(|steer| {
                user_message_preview_text(&steer.user_message, Some(&steer.history_record))
            })
            .collect();
        let rejected_steers = self
            .rejected_steers_queue
            .iter()
            .enumerate()
            .map(|(idx, message)| {
                user_message_preview_text(message, self.rejected_steer_history_records.get(idx))
            })
            .collect();

        PendingInputPreview {
            queued_messages,
            pending_steers,
            rejected_steers,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn preview_keeps_queue_categories_separate() {
        let mut state = InputQueueState::default();
        state
            .queued_user_messages
            .push_back(UserMessage::from("queued").into());
        state
            .rejected_steers_queue
            .push_back(UserMessage::from("rejected"));
        state.pending_steers.push_back(PendingSteer {
            user_message: UserMessage::from("pending"),
            history_record: UserMessageHistoryRecord::UserMessageText,
            compare_key: crate::tui_core::chatwidget::user_messages::PendingSteerCompareKey {
                message: "pending".to_string(),
                image_count: 0,
            },
        });

        assert_eq!(
            state.preview(),
            PendingInputPreview {
                queued_messages: vec!["queued".to_string()],
                pending_steers: vec!["pending".to_string()],
                rejected_steers: vec!["rejected".to_string()],
            }
        );
    }

    #[test]
    fn clear_resets_all_input_queues() {
        let mut state = InputQueueState::default();
        state
            .queued_user_messages
            .push_back(UserMessage::from("queued").into());
        state
            .rejected_steers_queue
            .push_back(UserMessage::from("rejected"));
        state.user_turn_pending_start = true;
        state.submit_pending_steers_after_interrupt = true;

        state.clear();

        assert!(state.queued_user_messages.is_empty());
        assert!(state.queued_user_message_history_records.is_empty());
        assert!(!state.user_turn_pending_start);
        assert!(state.rejected_steers_queue.is_empty());
        assert!(state.rejected_steer_history_records.is_empty());
        assert!(state.pending_steers.is_empty());
        assert!(!state.submit_pending_steers_after_interrupt);
    }
}
