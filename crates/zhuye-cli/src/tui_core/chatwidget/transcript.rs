//! `ChatWidget` 的会话记录与活跃单元格簿记。

use super::HistoryCell;

#[derive(Default)]
pub(super) struct TranscriptState {
    pub(super) active_cell: Option<Box<dyn HistoryCell>>,
    /// 准单调计数器，用于使 transcript overlay 缓存失效。
    pub(super) active_cell_revision: u64,
    /// 最近一次完成的 agent 响应的原始 markdown。
    pub(super) last_agent_markdown: Option<String>,
    /// 最近一次完成的提议计划的原始 markdown。
    pub(super) latest_proposed_plan_markdown: Option<String>,
    /// 本回合是否已产生可复制的响应。
    pub(super) saw_copy_source_this_turn: bool,
    /// 下一条流式助手内容之前是否应加上最终消息分隔符。
    pub(super) needs_final_message_separator: bool,
    /// 当前回合是否执行了“工作”（exec 命令、MCP 工具调用、补丁应用）。
    pub(super) had_work_activity: bool,
    /// 当前回合是否发出了计划更新。
    pub(super) saw_plan_update_this_turn: bool,
    /// 当前回合是否发出过尚未被
    /// 后续转向取代的提议计划项。
    pub(super) saw_plan_item_this_turn: bool,
    /// 最新 `update_plan` 检查清单任务数，用于终端标题渲染。
    pub(super) last_plan_progress: Option<(usize, usize)>,
    /// 流式计划内容的增量缓冲区。
    pub(super) plan_delta_buffer: String,
    /// 计划项正在流式输出时为 true。
    pub(super) plan_item_active: bool,
}

impl TranscriptState {
    pub(super) fn new(active_cell: Option<Box<dyn HistoryCell>>) -> Self {
        Self {
            active_cell,
            ..Self::default()
        }
    }

    pub(super) fn bump_active_cell_revision(&mut self) {
        // 回绕可避免溢出；需要 2^64 次递增才会回绕，
        // 最坏情况也只是一次性的缓存键冲突。
        self.active_cell_revision = self.active_cell_revision.wrapping_add(1);
    }

    pub(super) fn record_agent_markdown(&mut self, markdown: String) {
        self.last_agent_markdown = Some(markdown);
        self.saw_copy_source_this_turn = true;
    }

    pub(super) fn reset_copy_history(&mut self) {
        self.last_agent_markdown = None;
        self.saw_copy_source_this_turn = false;
    }

    pub(super) fn reset_turn_flags(&mut self) {
        self.saw_copy_source_this_turn = false;
        self.saw_plan_update_this_turn = false;
        self.saw_plan_item_this_turn = false;
        self.had_work_activity = false;
        self.latest_proposed_plan_markdown = None;
        self.plan_delta_buffer.clear();
        self.plan_item_active = false;
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn active_cell_revision_wraps() {
        let mut state = TranscriptState {
            active_cell_revision: u64::MAX,
            ..TranscriptState::default()
        };

        state.bump_active_cell_revision();

        assert_eq!(state.active_cell_revision, 0);
    }
}
