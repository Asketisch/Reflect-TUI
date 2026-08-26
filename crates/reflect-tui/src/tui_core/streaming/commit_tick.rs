//! 协调跨流控制器的提交滴漏排水。
//!
//! 此模块桥接基于队列的分块策略（`chunking`）与具体流控制器（`controller`）。调用者提供当前控制器和滴漏范围；模块
//! 计算队列压力、选择排水计划、应用它并返回发出的历史单元格。
//!
//! 模块通过仅从控制器队列头部排水来保持顺序。它不调度
//! 滴漏也不直接改变 UI 状态；调用者仍负责动画事件
//! 和历史插入副作用。
//!
//! 主要流程是：
//! [`run_commit_tick`] -> [`stream_queue_snapshot`] -> [`QueueSnapshot`] ->
//! [`resolve_chunking_plan`] -> [`ChunkingDecision`]/[`DrainPlan`] ->
//! [`apply_commit_tick_plan`] -> [`CommitTickOutput`]。

use std::time::Duration;
use std::time::Instant;

use crate::tui_core::history_cell::HistoryCell;

use super::chunking::AdaptiveChunkingPolicy;
use super::chunking::ChunkingDecision;
use super::chunking::ChunkingMode;
use super::chunking::DrainPlan;
use super::chunking::QueueSnapshot;
use super::controller::PlanStreamController;
use super::controller::StreamController;

/// 描述提交滴漏是否可以在所有模式下运行或仅在追赶模式下运行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitTickScope {
    /// 始终运行滴漏，无论当前分块模式如何。
    AnyMode,
    /// 运行模型转换和策略更新，但仅在 `CatchUp` 中提交行。
    CatchUpOnly,
}

/// 描述单个提交滴漏产生的内容。
pub(crate) struct CommitTickOutput {
    /// 在此滴漏期间由排水的行产生的单元格。
    pub(crate) cells: Vec<Box<dyn HistoryCell>>,
    /// 是否至少有一个流控制器在此滴漏期间存在。
    pub(crate) has_controller: bool,
    /// 是否所有存在的控制器在此滴漏后处于空闲状态。
    pub(crate) all_idle: bool,
}

impl Default for CommitTickOutput {
    /// 创建表示“未执行提交”的输出。
    ///
    /// 当滴漏被故意抑制时使用，例如当
    /// 范围是 [`CommitTickScope::CatchUpOnly`] 且策略不在追赶模式时。
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            has_controller: false,
            all_idle: true,
        }
    }
}

/// 对提供的流控制器运行一次提交滴漏。
///
/// 此函数收集 [`QueueSnapshot`]，向 [`AdaptiveChunkingPolicy`] 询问
/// [`ChunkingDecision`]，然后将结果 [`DrainPlan`] 应用到两个控制器。
/// 如果调用者传递过时的控制器引用（例如，未绑定到当前回合的引用），
/// 队列年龄可能被误读，策略可能在追赶模式下停留的时间比预期更长。
pub(crate) fn run_commit_tick(
    policy: &mut AdaptiveChunkingPolicy,
    stream_controller: Option<&mut StreamController>,
    plan_stream_controller: Option<&mut PlanStreamController>,
    scope: CommitTickScope,
    now: Instant,
) -> CommitTickOutput {
    let snapshot = stream_queue_snapshot(
        stream_controller.as_deref(),
        plan_stream_controller.as_deref(),
        now,
    );
    let decision = resolve_chunking_plan(policy, snapshot, now);
    if scope == CommitTickScope::CatchUpOnly && decision.mode != ChunkingMode::CatchUp {
        return CommitTickOutput::default();
    }

    apply_commit_tick_plan(
        decision.drain_plan,
        stream_controller,
        plan_stream_controller,
    )
}

/// 构建分块策略消耗的合并队列压力快照。
///
/// 快照跨控制器求和队列深度并保持最大最老年龄，
/// 以便策略决策反映当前可见的延迟最严重的排队行。
fn stream_queue_snapshot(
    stream_controller: Option<&StreamController>,
    plan_stream_controller: Option<&PlanStreamController>,
    now: Instant,
) -> QueueSnapshot {
    let mut queued_lines = 0usize;
    let mut oldest_age: Option<Duration> = None;

    if let Some(controller) = stream_controller {
        queued_lines += controller.queued_lines();
        oldest_age = max_duration(oldest_age, controller.oldest_queued_age(now));
    }
    if let Some(controller) = plan_stream_controller {
        queued_lines += controller.queued_lines();
        oldest_age = max_duration(oldest_age, controller.oldest_queued_age(now));
    }

    QueueSnapshot {
        queued_lines,
        oldest_age,
    }
}

/// 计算一个策略决策并在模式转换时发出跟踪日志。
///
/// 这将策略转换日志保持在同一位置，以便调用者可以依赖
/// [`run_commit_tick`] 提供一致的可见性。
fn resolve_chunking_plan(
    policy: &mut AdaptiveChunkingPolicy,
    snapshot: QueueSnapshot,
    now: Instant,
) -> ChunkingDecision {
    let prior_mode = policy.mode();
    let decision = policy.decide(snapshot, now);
    if decision.mode != prior_mode {
        tracing::trace!(
            prior_mode = ?prior_mode,
            new_mode = ?decision.mode,
            queued_lines = snapshot.queued_lines,
            oldest_queued_age_ms = snapshot.oldest_age.map(|age| age.as_millis() as u64),
            entered_catch_up = decision.entered_catch_up,
            "stream chunking mode transition"
        );
    }
    decision
}

/// 将 [`DrainPlan`] 应用到所有可用的流控制器。
///
/// 返回的 [`CommitTickOutput`] 报告发出的单元格以及所有
/// 存在的控制器在排水后是否处于空闲状态。
fn apply_commit_tick_plan(
    drain_plan: DrainPlan,
    stream_controller: Option<&mut StreamController>,
    plan_stream_controller: Option<&mut PlanStreamController>,
) -> CommitTickOutput {
    let mut output = CommitTickOutput::default();

    if let Some(controller) = stream_controller {
        output.has_controller = true;
        let (cell, is_idle) = drain_stream_controller(controller, drain_plan);
        if let Some(cell) = cell {
            output.cells.push(cell);
        }
        output.all_idle &= is_idle;
    }
    if let Some(controller) = plan_stream_controller {
        output.has_controller = true;
        let (cell, is_idle) = drain_plan_stream_controller(controller, drain_plan);
        if let Some(cell) = cell {
            output.cells.push(cell);
        }
        output.all_idle &= is_idle;
    }

    output
}

/// 对主流控制器应用一个排水步骤。
///
/// [`DrainPlan::Single`] 映射到单行排水；[`DrainPlan::Batch`] 映射到
/// 多行排水（包括策略请求完整排队积压时的即时追赶）。
fn drain_stream_controller(
    controller: &mut StreamController,
    drain_plan: DrainPlan,
) -> (Option<Box<dyn HistoryCell>>, bool) {
    match drain_plan {
        DrainPlan::Single => controller.on_commit_tick(),
        DrainPlan::Batch(max_lines) => controller.on_commit_tick_batch(max_lines),
    }
}

/// 对计划流控制器应用一个排水步骤。
///
/// 这与 [`drain_stream_controller`] 镜像，以便两种控制器类型遵循
/// 相同的分块策略决策。
fn drain_plan_stream_controller(
    controller: &mut PlanStreamController,
    drain_plan: DrainPlan,
) -> (Option<Box<dyn HistoryCell>>, bool) {
    match drain_plan {
        DrainPlan::Single => controller.on_commit_tick(),
        DrainPlan::Batch(max_lines) => controller.on_commit_tick_batch(max_lines),
    }
}

/// 返回两个可选持续时间中较大的一个。
///
/// 此辅助函数在只有一个持续时间存在时保留存在的一侧。
fn max_duration(lhs: Option<Duration>, rhs: Option<Duration>) -> Option<Duration> {
    match (lhs, rhs) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}
