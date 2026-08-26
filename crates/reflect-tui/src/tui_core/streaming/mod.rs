//! TUI 转录流水线使用的流式原语。
//!
//! `StreamState` 持有按换行门控的 markdown 收集器，以及已提交渲染行的 FIFO 队列。
//! 更上层的模块基于该状态构建：
//! - `controller` 将排队行适配为消息流与计划流所需的 `HistoryCell` 发射规则。
//! - `chunking` 根据队列压力计算自适应排空方案。
//! - `commit_tick` 将策略决策绑定到具体的控制器排空操作上。
//!
//! 关键不变量是队列顺序。所有排空都从队首弹出，而入队会记录到达时间戳，
//! 这样策略代码无需窥探文本内容即可推断最旧排队项的年龄。

use std::collections::VecDeque;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use crate::tui_core::markdown_stream::MarkdownStreamCollector;
use crate::tui_core::terminal_hyperlinks::HyperlinkLine;
pub(crate) mod chunking;
pub(crate) mod commit_tick;
pub(crate) mod controller;
mod render;
mod table_holdback;

struct QueuedLine {
    line: HyperlinkLine,
    enqueued_at: Instant,
}

/// 保存进行中的 markdown 流状态以及已排队并提交的行。
pub(crate) struct StreamState {
    pub(crate) collector: MarkdownStreamCollector,
    queued_lines: VecDeque<QueuedLine>,
    pub(crate) has_seen_delta: bool,
}

impl StreamState {
    /// 创建流状态，其 markdown 收集器会将本地文件链接渲染为相对于 `cwd` 的路径。
    ///
    /// 控制器预期在此传入会话的 cwd，并在活动流的整个生命周期内保持其稳定。
    pub(crate) fn new(width: Option<usize>, cwd: &Path) -> Self {
        Self {
            collector: MarkdownStreamCollector::new(width, cwd),
            queued_lines: VecDeque::new(),
            has_seen_delta: false,
        }
    }
    /// 为下一个流的生命周期重置收集器与队列状态。
    pub(crate) fn clear(&mut self) {
        self.collector.clear();
        self.queued_lines.clear();
        self.has_seen_delta = false;
    }
    /// 从队列前端排出一条已排队的行。
    pub(crate) fn step(&mut self) -> Vec<HyperlinkLine> {
        self.queued_lines
            .pop_front()
            .map(|queued| queued.line)
            .into_iter()
            .collect()
    }
    /// 从队列前端排出最多 `max_lines` 条已排队的行。
    ///
    /// 即使调用方传入非常大的值，该方法也会钳制到当前可用队列长度，因此行为仍然有界。
    pub(crate) fn drain_n(&mut self, max_lines: usize) -> Vec<HyperlinkLine> {
        let end = max_lines.min(self.queued_lines.len());
        self.queued_lines
            .drain(..end)
            .map(|queued| queued.line)
            .collect()
    }
    /// 清空已排队的行，同时保持收集器/回合生命周期状态不变。
    pub(crate) fn clear_queue(&mut self) {
        self.queued_lines.clear();
    }
    /// 返回是否没有任何已排队等待提交的行。
    pub(crate) fn is_idle(&self) -> bool {
        self.queued_lines.is_empty()
    }
    /// 返回当前队列深度。
    pub(crate) fn queued_len(&self) -> usize {
        self.queued_lines.len()
    }
    /// 返回最旧排队行的年龄。
    pub(crate) fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.queued_lines
            .front()
            .map(|queued| now.saturating_duration_since(queued.enqueued_at))
    }
    /// 以共享的入队时间戳将已提交的行追加到队列中。
    pub(crate) fn enqueue(&mut self, lines: Vec<HyperlinkLine>) {
        let now = Instant::now();
        self.queued_lines
            .extend(lines.into_iter().map(|line| QueuedLine {
                line,
                enqueued_at: now,
            }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::text::Line;
    use std::path::PathBuf;

    fn test_cwd() -> PathBuf {
        // 这些测试只需要一个稳定的绝对 cwd；使用 temp_dir() 可避免把 Unix 或 Windows
        // 特有的根路径语义固化到测试夹具中。
        std::env::temp_dir()
    }

    #[test]
    fn drain_n_clamps_to_available_lines() {
        let mut state = StreamState::new(/*width*/ None, &test_cwd());
        state.enqueue(vec![HyperlinkLine::new(Line::from("one"))]);

        let drained = state.drain_n(/*max_lines*/ 8);
        assert_eq!(drained, vec![HyperlinkLine::new(Line::from("one"))]);
        assert!(state.is_idle());
    }
}
