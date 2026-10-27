//! 提交动画刻度的自适应流分块策略。
//!
//! 此策略在保持基线用户体验的同时适应突发性流输入。在 [`ChunkingMode::Smooth`] 模式下，每个
//! 基线提交刻度排空一行排队内容。当队列压力上升时，切换到 [`ChunkingMode::CatchUp`] 并立即
//! 排空排队积压，使显示延迟尽快收敛。
//!
//! 策略与源无关：仅依赖于 [`QueueSnapshot`] 中的队列深度和队列年龄。它不根据源标识或显式
//! 吞吐量目标进行分支。
//!
//! # 心智模型
//!
//! 将此视为两档系统：
//!
//! - [`ChunkingMode::Smooth`]：稳定的基线显示节奏。
//! - [`ChunkingMode::CatchUp`]：存在积压时全队列排空。
//!
//! 转换逻辑有意使用滞后：
//!
//! - 在高压力阈值进入追赶
//! - 在低压力阈值退出追赶，保持 [`EXIT_HOLD`]
//! - 退出后，除非积压严重，否则抑制 [`REENTER_CATCH_UP_HOLD`] 内的立即重新进入
//!
//! 这避免了阈值边界附近的快速档位抖动。
//!
//! # 策略流程
//!
//! 在每个决策刻度上，[`AdaptiveChunkingPolicy::decide`] 执行：
//!
//! 1. 如果队列为空，重置为 [`ChunkingMode::Smooth`]。
//! 2. 如果当前为平滑模式，调用 [`AdaptiveChunkingPolicy::maybe_enter_catch_up`]。
//! 3. 如果当前为追赶模式，调用 [`AdaptiveChunkingPolicy::maybe_exit_catch_up`]。
//! 4. 构建 [`DrainPlan`]（平滑为 `Single`，追赶为 `Batch(queued_lines)`）。
//!
//! # 具体示例
//!
//! 使用当前默认值：
//!
//! - `Smooth` 每个提交刻度排空一行。
//! - `CatchUp` 在每个刻度排空所有当前排队的行。
//!
//! # 调优指南（代码术语）
//!
//! 按此顺序调优以保持原因清晰：
//!
//! 1. 进入/退出阈值：[`ENTER_QUEUE_DEPTH_LINES`]、[`ENTER_OLDEST_AGE`]、
//!    [`EXIT_QUEUE_DEPTH_LINES`]、[`EXIT_OLDEST_AGE`]
//! 2. 滞后窗口：[`EXIT_HOLD`]、[`REENTER_CATCH_UP_HOLD`]
//! 3. 严重门控：[`SEVERE_QUEUE_DEPTH_LINES`]、[`SEVERE_OLDEST_AGE`]
//!
//! 面向症状的调整：
//!
//! - 延迟启动太晚：降低进入阈值
//! - 频繁平滑/追赶聊天：增加保持窗口或收紧退出阈值
//! - 追赶退出后重新进入太急切：增加重新进入保持时间
//!
//! # 职责
//!
//! - 跟踪模式和滞后状态
//! - 从队列快照生成确定性的 [`ChunkingDecision`] 值
//! - 仅从队头排空以保留队列顺序
//!
//! # 非职责
//!
//! - 调度提交刻度
//! - 重排序流行
//! - 传输/源特定的语义
//!
//! Markdown 文档保持补充：
//!
//! - `docs/tui-stream-chunking-review.md`
//! - `docs/tui-stream-chunking-tuning.md`
//! - `docs/tui-stream-chunking-validation.md`

use std::time::Duration;
use std::time::Instant;

/// 允许进入追赶模式的队列深度阈值。
///
/// 单独越过此阈值就足以离开平滑模式。
const ENTER_QUEUE_DEPTH_LINES: usize = 8;

/// 允许进入追赶模式的最旧行年龄阈值。
///
/// 单独越过此阈值就足以离开平滑模式。
const ENTER_OLDEST_AGE: Duration = Duration::from_millis(120);

/// 评估追赶退出滞后时使用的队列深度阈值。
///
/// 深度必须达到或低于此值，退出保持计时才能开始。
const EXIT_QUEUE_DEPTH_LINES: usize = 2;

/// 评估追赶退出滞后时使用的最旧行年龄阈值。
///
/// 年龄必须达到或低于此值，退出保持计时才能开始。
const EXIT_OLDEST_AGE: Duration = Duration::from_millis(40);

/// 队列压力必须保持在退出阈值以下的持续时间，才能离开追赶模式。
const EXIT_HOLD: Duration = Duration::from_millis(250);

/// 追赶退出后的冷却窗口，抑制立即重新进入。
///
/// 严重积压仍绕过此保持，以避免无界队列年龄增长。
const REENTER_CATCH_UP_HOLD: Duration = Duration::from_millis(250);

/// 将积压标记为严重以便更快收敛的队列深度截断。
///
/// 此阈值用于在最近的追赶退出后绕过重新进入保持。
const SEVERE_QUEUE_DEPTH_LINES: usize = 64;

/// 将积压标记为严重以便更快收敛的最旧行年龄截断。
const SEVERE_OLDEST_AGE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ChunkingMode {
    /// 每个基线提交刻度排空一行。
    #[default]
    Smooth,
    /// 根据队列压力每个刻度排空多行。
    CatchUp,
}

/// 捕获自适应分块决策使用的队列压力输入。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct QueueSnapshot {
    /// 等待显示的排队流行数。
    pub(crate) queued_lines: usize,
    /// 决策时最旧排队行的年龄。
    pub(crate) oldest_age: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrainPlan {
    /// 精确发出一个排队行。
    Single,
    /// 发出最多 `usize` 个排队行。
    Batch(usize),
}

/// 表示针对特定队列快照的一项策略决策。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChunkingDecision {
    /// 应用本次决策的滞后转换后的模式。
    pub(crate) mode: ChunkingMode,
    /// 本次决策是否从 `Smooth` 转换为 `CatchUp`。
    pub(crate) entered_catch_up: bool,
    /// 当前提交刻度要执行的排空计划。
    pub(crate) drain_plan: DrainPlan,
}

/// 跨刻度维护自适应分块模式和滞后状态。
#[derive(Debug, Default)]
pub(crate) struct AdaptiveChunkingPolicy {
    mode: ChunkingMode,
    below_exit_threshold_since: Option<Instant>,
    last_catch_up_exit_at: Option<Instant>,
}

impl AdaptiveChunkingPolicy {
    /// 返回最近一次决策使用的策略模式。
    pub(crate) fn mode(&self) -> ChunkingMode {
        self.mode
    }

    /// 将状态重置为基线平滑模式。
    pub(crate) fn reset(&mut self) {
        self.mode = ChunkingMode::Smooth;
        self.below_exit_threshold_since = None;
        self.last_catch_up_exit_at = None;
    }

    /// 从当前队列快照计算排空决策。
    ///
    /// 决策对给定的 `(mode, snapshot, now)` 三元组是确定性的。调用方应
    /// 避免发明合成快照；过期的队列年龄数据可能导致过早的追赶退出。
    pub(crate) fn decide(&mut self, snapshot: QueueSnapshot, now: Instant) -> ChunkingDecision {
        if snapshot.queued_lines == 0 {
            self.note_catch_up_exit(now);
            self.mode = ChunkingMode::Smooth;
            self.below_exit_threshold_since = None;
            return ChunkingDecision {
                mode: self.mode,
                entered_catch_up: false,
                drain_plan: DrainPlan::Single,
            };
        }

        let entered_catch_up = match self.mode {
            ChunkingMode::Smooth => self.maybe_enter_catch_up(snapshot, now),
            ChunkingMode::CatchUp => {
                self.maybe_exit_catch_up(snapshot, now);
                false
            }
        };

        let drain_plan = match self.mode {
            ChunkingMode::Smooth => DrainPlan::Single,
            ChunkingMode::CatchUp => DrainPlan::Batch(snapshot.queued_lines.max(1)),
        };

        ChunkingDecision {
            mode: self.mode,
            entered_catch_up,
            drain_plan,
        }
    }

    /// 当进入阈值被越过时从 `Smooth` 切换到 `CatchUp`。
    ///
    /// 仅在转换刻度上返回 `true`，以便调用方可以发出一次性
    /// 转换可观测性。
    fn maybe_enter_catch_up(&mut self, snapshot: QueueSnapshot, now: Instant) -> bool {
        if !should_enter_catch_up(snapshot) {
            return false;
        }
        if self.reentry_hold_active(now) && !is_severe_backlog(snapshot) {
            return false;
        }
        self.mode = ChunkingMode::CatchUp;
        self.below_exit_threshold_since = None;
        self.last_catch_up_exit_at = None;
        true
    }

    /// 在 `CatchUp` 模式下应用退出滞后。
    ///
    /// 策略要求队列压力持续低于退出阈值
    /// 完整的 `EXIT_HOLD` 窗口，然后才能返回 `Smooth`。
    fn maybe_exit_catch_up(&mut self, snapshot: QueueSnapshot, now: Instant) {
        if !should_exit_catch_up(snapshot) {
            self.below_exit_threshold_since = None;
            return;
        }

        match self.below_exit_threshold_since {
            Some(since) if now.saturating_duration_since(since) >= EXIT_HOLD => {
                self.mode = ChunkingMode::Smooth;
                self.below_exit_threshold_since = None;
                self.last_catch_up_exit_at = Some(now);
            }
            Some(_) => {}
            None => {
                self.below_exit_threshold_since = Some(now);
            }
        }
    }

    fn note_catch_up_exit(&mut self, now: Instant) {
        if self.mode == ChunkingMode::CatchUp {
            self.last_catch_up_exit_at = Some(now);
        }
    }

    fn reentry_hold_active(&self, now: Instant) -> bool {
        self.last_catch_up_exit_at
            .is_some_and(|exit| now.saturating_duration_since(exit) < REENTER_CATCH_UP_HOLD)
    }
}

/// 返回当前队列压力是否值得进入追赶模式。
///
/// 深度或年龄压力中的任何一个都足以触发追赶。
fn should_enter_catch_up(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines >= ENTER_QUEUE_DEPTH_LINES
        || snapshot
            .oldest_age
            .is_some_and(|oldest| oldest >= ENTER_OLDEST_AGE)
}

/// 返回队列压力是否足够低以开始退出滞后。
///
/// 深度和年龄都必须低于阈值；当
/// 一个信号仍处于负载下时，这可防止振荡。
fn should_exit_catch_up(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines <= EXIT_QUEUE_DEPTH_LINES
        && snapshot
            .oldest_age
            .is_some_and(|oldest| oldest <= EXIT_OLDEST_AGE)
}

/// 返回积压是否严重到可以使用更快的追赶目标。
///
/// 严重压力绕过重新进入保持，以避免在
/// 最近追赶退出后队列年龄增长。
fn is_severe_backlog(snapshot: QueueSnapshot) -> bool {
    snapshot.queued_lines >= SEVERE_QUEUE_DEPTH_LINES
        || snapshot
            .oldest_age
            .is_some_and(|oldest| oldest >= SEVERE_OLDEST_AGE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn snapshot(queued_lines: usize, oldest_age_ms: u64) -> QueueSnapshot {
        QueueSnapshot {
            queued_lines,
            oldest_age: Some(Duration::from_millis(oldest_age_ms)),
        }
    }

    #[test]
    fn smooth_mode_is_default() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let now = Instant::now();

        let decision = policy.decide(snapshot(/*queued_lines*/ 1, /*oldest_age_ms*/ 10), now);
        assert_eq!(decision.mode, ChunkingMode::Smooth);
        assert_eq!(decision.entered_catch_up, false);
        assert_eq!(decision.drain_plan, DrainPlan::Single);
    }

    #[test]
    fn enters_catch_up_on_depth_threshold() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let now = Instant::now();

        let decision = policy.decide(snapshot(/*queued_lines*/ 8, /*oldest_age_ms*/ 10), now);
        assert_eq!(decision.mode, ChunkingMode::CatchUp);
        assert_eq!(decision.entered_catch_up, true);
        assert_eq!(decision.drain_plan, DrainPlan::Batch(8));
    }

    #[test]
    fn enters_catch_up_on_age_threshold() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let now = Instant::now();

        let decision = policy.decide(snapshot(/*queued_lines*/ 2, /*oldest_age_ms*/ 120), now);
        assert_eq!(decision.mode, ChunkingMode::CatchUp);
        assert_eq!(decision.entered_catch_up, true);
        assert_eq!(decision.drain_plan, DrainPlan::Batch(2));
    }

    #[test]
    fn severe_backlog_uses_faster_paced_batches() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let now = Instant::now();
        let _ = policy.decide(snapshot(/*queued_lines*/ 9, /*oldest_age_ms*/ 10), now);

        let decision = policy.decide(
            snapshot(/*queued_lines*/ 64, /*oldest_age_ms*/ 10),
            now + Duration::from_millis(5),
        );
        assert_eq!(decision.mode, ChunkingMode::CatchUp);
        assert_eq!(decision.drain_plan, DrainPlan::Batch(64));
    }

    #[test]
    fn catch_up_batch_drains_current_backlog() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let now = Instant::now();
        let decision = policy.decide(snapshot(/*queued_lines*/ 512, /*oldest_age_ms*/ 400), now);
        assert_eq!(decision.mode, ChunkingMode::CatchUp);
        assert_eq!(decision.drain_plan, DrainPlan::Batch(512));
    }

    #[test]
    fn exits_catch_up_after_hysteresis_hold() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let t0 = Instant::now();

        let _ = policy.decide(snapshot(/*queued_lines*/ 9, /*oldest_age_ms*/ 10), t0);
        assert_eq!(policy.mode(), ChunkingMode::CatchUp);

        let pre_hold = policy.decide(
            snapshot(/*queued_lines*/ 2, /*oldest_age_ms*/ 40),
            t0 + Duration::from_millis(200),
        );
        assert_eq!(pre_hold.mode, ChunkingMode::CatchUp);

        let post_hold = policy.decide(
            snapshot(/*queued_lines*/ 2, /*oldest_age_ms*/ 40),
            t0 + Duration::from_millis(460),
        );
        assert_eq!(post_hold.mode, ChunkingMode::Smooth);
        assert_eq!(post_hold.drain_plan, DrainPlan::Single);
    }

    #[test]
    fn drops_back_to_smooth_when_idle() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let now = Instant::now();
        let _ = policy.decide(snapshot(/*queued_lines*/ 9, /*oldest_age_ms*/ 10), now);
        assert_eq!(policy.mode(), ChunkingMode::CatchUp);

        let decision = policy.decide(
            QueueSnapshot {
                queued_lines: 0,
                oldest_age: None,
            },
            now + Duration::from_millis(20),
        );
        assert_eq!(decision.mode, ChunkingMode::Smooth);
        assert_eq!(decision.drain_plan, DrainPlan::Single);
    }

    #[test]
    fn holds_reentry_after_catch_up_exit() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let t0 = Instant::now();

        let entered = policy.decide(snapshot(/*queued_lines*/ 8, /*oldest_age_ms*/ 20), t0);
        assert_eq!(entered.mode, ChunkingMode::CatchUp);

        let drained = policy.decide(
            QueueSnapshot {
                queued_lines: 0,
                oldest_age: None,
            },
            t0 + Duration::from_millis(20),
        );
        assert_eq!(drained.mode, ChunkingMode::Smooth);

        let held = policy.decide(
            snapshot(/*queued_lines*/ 8, /*oldest_age_ms*/ 20),
            t0 + Duration::from_millis(120),
        );
        assert_eq!(held.mode, ChunkingMode::Smooth);
        assert_eq!(held.drain_plan, DrainPlan::Single);

        let reentered = policy.decide(
            snapshot(/*queued_lines*/ 8, /*oldest_age_ms*/ 20),
            t0 + Duration::from_millis(320),
        );
        assert_eq!(reentered.mode, ChunkingMode::CatchUp);
        assert_eq!(reentered.drain_plan, DrainPlan::Batch(8));
    }

    #[test]
    fn severe_backlog_can_reenter_during_hold() {
        let mut policy = AdaptiveChunkingPolicy::default();
        let t0 = Instant::now();

        let _ = policy.decide(snapshot(/*queued_lines*/ 8, /*oldest_age_ms*/ 20), t0);
        let _ = policy.decide(
            QueueSnapshot {
                queued_lines: 0,
                oldest_age: None,
            },
            t0 + Duration::from_millis(20),
        );

        let severe = policy.decide(
            snapshot(/*queued_lines*/ 64, /*oldest_age_ms*/ 20),
            t0 + Duration::from_millis(120),
        );
        assert_eq!(severe.mode, ChunkingMode::CatchUp);
        assert_eq!(severe.drain_plan, DrainPlan::Batch(64));
    }
}
