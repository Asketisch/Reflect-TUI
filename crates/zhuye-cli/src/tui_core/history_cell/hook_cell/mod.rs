//! 用于钩子执行的历史单元格。
//!
//! 钩子有意比普通工具调用更安静。一个开始并成功结束、且没有输出的钩子不应留下任何转录痕迹，非常快的
//! 钩子也不应在视口中闪现。本单元格通过把每次钩子运行视为一个小小的渲染状态机，将这条策略局部化：
//!
//! 1. 新运行开始时隐藏在 `PendingReveal` 中。
//! 2. 超过显示延迟的运行变为可见，并且可以与相邻运行合并。
//! 3. 可见的静默成功会短暂停留，以免在首次绘制的同一帧内就消失。
//! 4. 完成的运行只有在有输出或非成功状态时才持久保留。
use super::HistoryCell;
use super::plain_lines;
use crate::app_server_protocol::HookEventName;
use crate::app_server_protocol::HookOutputEntry;
use crate::app_server_protocol::HookOutputEntryKind;
use crate::app_server_protocol::HookRunStatus;
use crate::app_server_protocol::HookRunSummary;
use crate::tui_core::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::tui_core::motion::MotionMode;
use crate::tui_core::motion::ReducedMotionIndicator;
use crate::tui_core::motion::activity_indicator;
use crate::tui_core::motion::shimmer_text;
use crate::tui_core::render::line_utils::push_owned_lines;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::ui_consts::TRANSCRIPT_HINT;
use crate::tui_core::wrapping::RtOptions;
use crate::tui_core::wrapping::word_wrap_line;
use ratatui::prelude::*;
use ratatui::style::Stylize;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::time::Duration;
use std::time::Instant;

#[derive(Debug)]
pub(crate) struct HookCell {
    /// 处于活动、停留或具有需要渲染的持久输出的钩子运行。
    runs: Vec<HookRunCell>,
    /// 镜像全局动画设置，使转录渲染与视口渲染保持一致。
    animations_enabled: bool,
}

/// 钩子被允许绘制前的最短运行时间。
///
/// 有助于避免那些实际上瞬间完成的工作出现文字闪烁。
const HOOK_RUN_REVEAL_DELAY: Duration = Duration::from_millis(300);

/// 静默成功在变为可见后停留在屏幕上的最短时间。
///
/// 这与 `HOOK_RUN_REVEAL_DELAY` 配合使用：一旦用户看到了钩子行，就要让它在足够长的时间内保持稳定
/// 以便阅读，而不是在成功事件到达时立即将其移除。
const QUIET_HOOK_MIN_VISIBLE: Duration = Duration::from_millis(600);

const HOOK_OUTPUT_INDENT: &str = "  ";
const HOOK_OUTPUT_BODY_INDENT: &str = "    ";
const HOOK_CONTEXT_MAX_DISPLAY_ROWS: usize = 3;

#[derive(Debug)]
struct HookRunCell {
    /// 用于匹配同一次钩子调用 begin/end 更新的稳定协议 id。
    id: String,
    /// 钩子事件类型，放在 `state` 之外，以便 begin 更新可以就地刷新元数据。
    event_name: HookEventName,
    /// 可选的钩子提供的细节文本，显示在运行中标题的旁边。
    status_message: Option<String>,
    /// 本次运行的渲染生命周期。
    state: HookRunState,
}

#[derive(Debug)]
enum HookRunState {
    /// 一个刚刚开始、处于活动状态，但在 `reveal_deadline` 之前被有意隐藏的运行。
    PendingReveal {
        /// 原始开始时间，用于揭示后的加载动画相位与分组。
        start_time: Instant,
        /// 运行可能变为可见的第一个时刻。
        reveal_deadline: Instant,
    },
    /// 一个度过了显示延迟、当前以“运行中”状态展示的运行。
    VisibleRunning {
        /// 原始开始时间，用于在状态转换之间保持动画计时稳定。
        start_time: Instant,
        /// 运行实际被渲染的第一个时刻，供“静默成功停留”逻辑使用。
        visible_since: Instant,
    },
    /// 一个已可见、成功完成且没有输出，但仍会短暂停留的运行。
    QuietLinger {
        /// 原始开始时间，予以保留，以便加载动画在停留帧内不会跳动。
        start_time: Instant,
        /// 此后静默成功可以被完全移除的时刻。
        removal_deadline: Instant,
    },
    /// 一个有输出或状态值得在历史中保留的已完成运行。
    Completed {
        /// 钩子调用的最终协议状态。
        status: HookRunStatus,
        /// 渲染在完成标题下方的钩子输出条目。
        entries: Vec<HookOutputEntry>,
    },
}

#[derive(Debug, PartialEq, Eq)]
struct RunningHookGroupKey {
    event_name: HookEventName,
    status_message: Option<String>,
}

/// 用于相邻运行中钩子的累加器，这些钩子可以共享一行状态显示。
///
/// 分组只在构建显示行时进行，底层运行仍保持独立，因此它们的协议 id 与完成转换互不影响。
struct RunningHookGroup {
    /// 本显示组中每个运行共享的事件/状态对。
    key: RunningHookGroupKey,
    /// 组内最早的开始时间，使合并后的加载动画反映最久远的工作。
    start_time: Option<Instant>,
    /// 该组行所代表的相邻运行数量。
    count: usize,
}

impl HookCell {
    /// 围绕一个刚刚开始的钩子创建单元格。
    fn new_active(run: HookRunSummary, animations_enabled: bool) -> Self {
        let mut cell = Self {
            runs: Vec::new(),
            animations_enabled,
        };
        cell.start_run(run);
        cell
    }

    /// 根据转录/历史数据，围绕一个已完成的钩子创建单元格。
    fn new_completed(run: HookRunSummary, animations_enabled: bool) -> Self {
        let mut cell = Self {
            runs: Vec::new(),
            animations_enabled,
        };
        cell.add_completed_run(run);
        cell
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// 只要仍有任何运行可能因结束事件或计时器而发生变化，就返回 true。
    pub(crate) fn is_active(&self) -> bool {
        self.runs.iter().any(|run| run.state.is_active())
    }

    /// 一旦没有剩余计时器，已完成的钩子单元格就会从活动槽位中被刷新出去。
    pub(crate) fn should_flush(&self) -> bool {
        !self.is_active() && !self.is_empty()
    }

    /// 返回该单元格当前是否至少有一行值得绘制。
    pub(crate) fn should_render(&self) -> bool {
        self.runs.iter().any(|run| run.state.should_render())
    }

    /// 将持久的完成运行与临时性的活动单元格记账状态分离开来。
    ///
    /// 静默成功会被留在原处，以便它们可以从活动单元格中消失；而失败、被阻塞/停止的钩子以及有输出
    /// 产生的钩子则会成为一个持久的历史单元格。
    pub(crate) fn take_completed_persistent_runs(&mut self) -> Option<Self> {
        let mut completed = Vec::new();
        let mut remaining = Vec::new();
        for run in self.runs.drain(..) {
            if run.state.has_persistent_output() {
                completed.push(run);
            } else {
                remaining.push(run);
            }
        }
        self.runs = remaining;
        (!completed.is_empty()).then_some(Self {
            runs: completed,
            animations_enabled: self.animations_enabled,
        })
    }

    /// 供需要了解活动单元格当前是否占据视口空间的调用方使用。
    pub(crate) fn has_visible_running_run(&self) -> bool {
        self.runs.iter().any(|run| run.state.is_running_visible())
    }

    /// 推进显示/移除计时器，并报告渲染是否应被刷新。
    pub(crate) fn advance_time(&mut self, now: Instant) -> bool {
        let old_len = self.runs.len();
        let mut changed = false;
        for run in &mut self.runs {
            changed |= run.state.reveal_if_due(now);
        }
        self.runs.retain(|run| !run.state.quiet_linger_expired(now));
        changed || self.runs.len() != old_len
    }

    /// 插入或刷新一条已开始的钩子运行。
    ///
    /// 重复的 begin 事件会重置显示计时器，而不是新增一行，因为按 id 匹配是保持 begin/end 事件
    /// 成对的不变式。
    pub(crate) fn start_run(&mut self, run: HookRunSummary) {
        let now = Instant::now();
        if let Some(existing) = self.runs.iter_mut().find(|existing| existing.id == run.id) {
            existing.event_name = run.event_name;
            existing.status_message = run.status_message;
            existing.state = HookRunState::pending(now);
            return;
        }
        self.runs.push(HookRunCell {
            id: run.id,
            event_name: run.event_name,
            status_message: run.status_message,
            state: HookRunState::pending(now),
        });
    }

    /// 完成一条运行，并返回该运行是否已存在于本单元格中。
    ///
    /// 静默成功会刻意避免持久输出。如果它们从未可见，就会立即消失；如果它们已经绘制过，则会进入
    /// `QuietLinger` 状态。
    pub(crate) fn complete_run(&mut self, run: HookRunSummary) -> bool {
        let Some(index) = self.runs.iter().position(|existing| existing.id == run.id) else {
            return false;
        };
        if hook_run_is_quiet_success(&run) {
            if !self.runs[index]
                .state
                .complete_quiet_success(Instant::now())
            {
                self.runs.remove(index);
            }
            return true;
        }
        let HookRunSummary {
            event_name,
            status_message,
            status,
            entries,
            ..
        } = run;
        let existing = &mut self.runs[index];
        existing.event_name = event_name;
        existing.status_message = status_message;
        existing.state = HookRunState::completed(status, entries);
        true
    }

    /// 添加一条未经过此活动单元格的已完成钩子。
    ///
    /// 这用于最终运行摘要已知的回放/恢复路径。
    pub(crate) fn add_completed_run(&mut self, run: HookRunSummary) {
        if hook_run_is_quiet_success(&run) {
            return;
        }
        let HookRunSummary {
            id,
            event_name,
            status_message,
            status,
            entries,
            ..
        } = run;
        self.runs.push(HookRunCell {
            id,
            event_name,
            status_message,
            state: HookRunState::completed(status, entries),
        });
    }

    pub(crate) fn next_timer_deadline(&self) -> Option<Instant> {
        self.runs
            .iter()
            .filter_map(|run| run.state.next_timer_deadline())
            .min()
    }

    #[cfg(test)]
    pub(crate) fn expire_quiet_runs_now_for_test(&mut self) {
        for run in &mut self.runs {
            run.expire_quiet_linger_now_for_test();
        }
    }

    #[cfg(test)]
    pub(crate) fn reveal_running_runs_now_for_test(&mut self) {
        let now = Instant::now();
        for run in &mut self.runs {
            run.reveal_running_now_for_test(now);
        }
    }

    #[cfg(test)]
    pub(crate) fn reveal_running_runs_after_delayed_redraw_for_test(&mut self) {
        let now = Instant::now();
        for run in &mut self.runs {
            run.reveal_running_after_delayed_redraw_for_test(now);
        }
    }

    /// 为受限的主视口或完整的转录覆盖层构建钩子行。
    fn output_lines(&self, width: u16, render_full_context: bool) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let mut running_group: Option<RunningHookGroup> = None;
        for run in &self.runs {
            if !run.state.should_render() {
                continue;
            }

            let Some(key) = run.running_group_key() else {
                // 已完成的运行保留自己的输出行，因此在绘制完成运行之前，必须先输出任何待处理的
                // 运行中分组。
                if let Some(group) = running_group.take() {
                    push_running_hook_group(&mut lines, &group, self.animations_enabled);
                }
                push_hook_line_separator(&mut lines);
                run.push_display_lines(
                    &mut lines,
                    self.animations_enabled,
                    width,
                    render_full_context,
                );
                continue;
            };

            if let Some(group) = running_group.as_mut()
                && group.key == key
            {
                group.count += 1;
                // 保留最早的开始时间，这样当后续相邻的钩子并入同一行时，分组后的加载动画不会重置。
                group.start_time = earliest_instant(group.start_time, run.state.start_time());
                continue;
            }

            if let Some(group) =
                running_group.replace(RunningHookGroup::new(key, run.state.start_time()))
            {
                push_running_hook_group(&mut lines, &group, self.animations_enabled);
            }
        }
        if let Some(group) = running_group {
            push_running_hook_group(&mut lines, &group, self.animations_enabled);
        }
        lines
    }
}

impl HistoryCell for HookCell {
    /// 在合并相邻的可见运行中钩子的同时构建视口行。
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.output_lines(width, /*render_full_context*/ false)
    }

    /// 转录覆盖层会保留被视口预览隐藏的完整钩子上下文。
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.output_lines(width, /*render_full_context*/ true)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.output_lines(u16::MAX, /*render_full_context*/ true))
    }

    /// 在钩子动画处于活动状态时，为转录覆盖层生成一个粗略的缓存键。
    fn transcript_animation_tick(&self) -> Option<u64> {
        if !self.animations_enabled {
            return None;
        }
        let elapsed = self
            .runs
            .iter()
            .filter(|run| run.state.is_running_visible())
            .find_map(|run| run.state.start_time())?
            .elapsed();
        Some(elapsed.as_millis() as u64 / 600)
    }
}

impl Renderable for HookCell {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let lines = self.display_lines(area.width);
        let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        paragraph.render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        HistoryCell::desired_height(self, width)
    }
}

impl HookRunCell {
    #[cfg(test)]
    fn expire_quiet_linger_now_for_test(&mut self) {
        if let HookRunState::QuietLinger {
            removal_deadline, ..
        } = &mut self.state
        {
            *removal_deadline = Instant::now();
        }
    }

    #[cfg(test)]
    fn reveal_running_now_for_test(&mut self, now: Instant) {
        if let HookRunState::PendingReveal {
            reveal_deadline, ..
        } = &mut self.state
        {
            *reveal_deadline = now;
        }
    }

    #[cfg(test)]
    fn reveal_running_after_delayed_redraw_for_test(&mut self, now: Instant) {
        if let HookRunState::PendingReveal {
            reveal_deadline, ..
        } = &mut self.state
        {
            let delayed_deadline = now
                .checked_sub(QUIET_HOOK_MIN_VISIBLE + Duration::from_millis(100))
                .unwrap_or(now);
            *reveal_deadline = delayed_deadline;
        }
    }

    /// 仅为以“运行中”状态渲染的状态返回分组键。
    fn running_group_key(&self) -> Option<RunningHookGroupKey> {
        self.state
            .is_running_visible()
            .then(|| RunningHookGroupKey {
                event_name: self.event_name,
                status_message: self.status_message.clone(),
            })
    }

    /// 追加单条、未分组的钩子运行的显示行。
    fn push_display_lines(
        &self,
        lines: &mut Vec<Line<'static>>,
        animations_enabled: bool,
        width: u16,
        render_full_context: bool,
    ) {
        let label = hook_event_label(self.event_name);
        match &self.state {
            HookRunState::VisibleRunning { start_time, .. }
            | HookRunState::QuietLinger { start_time, .. } => {
                let hook_text = format!("Running {label} hook");
                push_running_hook_header(
                    lines,
                    &hook_text,
                    Some(*start_time),
                    self.status_message.as_deref(),
                    animations_enabled,
                );
            }
            HookRunState::Completed { status, entries } => {
                let system_message = entries
                    .iter()
                    .find(|entry| entry.kind == HookOutputEntryKind::Warning);
                let mut system_message_lines = system_message.map(|entry| entry.text.split('\n'));
                let status_text = format!("{status:?}").to_lowercase();
                let header_text = if let Some(first_line) =
                    system_message_lines.as_mut().and_then(Iterator::next)
                {
                    format!("{label} ({status_text}) says: {first_line}")
                } else {
                    format!("{label} hook ({status_text})")
                };
                lines.push(
                    vec![
                        hook_completed_bullet(*status, entries),
                        " ".into(),
                        header_text.into(),
                    ]
                    .into(),
                );
                if let Some(system_message_lines) = system_message_lines {
                    for line in system_message_lines {
                        if line.is_empty() {
                            lines.push("".into());
                        } else {
                            lines.push(format!("{HOOK_OUTPUT_BODY_INDENT}{line}").into());
                        }
                    }
                }
                for entry in entries {
                    if entry.kind == HookOutputEntryKind::Warning {
                        continue;
                    }
                    if !render_full_context && entry.kind == HookOutputEntryKind::Context {
                        lines.extend(hook_context_preview_lines(&entry.text, width));
                    } else {
                        push_full_hook_output_entry(lines, entry);
                    }
                }
            }
            HookRunState::PendingReveal { .. } => {}
        }
    }
}

// ── 渲染/状态辅助（外移子模块） ──
mod render_helpers;
pub(crate) use render_helpers::*;

#[cfg(all(test, feature = "tui-upstream-tests"))]
#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
