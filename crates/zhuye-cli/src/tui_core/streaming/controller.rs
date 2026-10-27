//! 用于代理消息和提议计划的双区域流控制器。
//!
//! 每个流将渲染的 markdown 划分为*稳定区域*（通过 `StreamState` 中的动画队列
//! 提交到回滚）和*尾部区域*（可变，在活动单元格槽位中显示为临时流尾部单元格）。
//!
//! `StreamCore` 拥有共享簿记：源累积、重新渲染、
//! 稳定/尾部分区、提交动画队列管理和终端
//! 调整大小处理。`StreamController` 和 `PlanStreamController` 是薄
//! 包装器，仅添加它们的 `emit()` 样式和 finalize 返回类型。
//!
//! ## 表保持
//!
//! 表渲染本质上是非增量的：添加新行可以更改
//! 每列的宽度并重塑所有先前行。保持机制
//! （`table_holdback_state`）检测累积源中的管道表模式（标题 + 分隔符
//! 对），并将内容从表标题
//! 开始保持为可变尾部，直到流最终确定。保持功能在
//! 代理和提议计划流中启用。`Outside` 和 `Markdown` fence
//! 上下文中的行被扫描；非 markdown fence 内部的行被跳过。
//!
//! ## 调整大小处理
//!
//! 在终端宽度更改时，`StreamCore::set_width` 在新
//! 宽度处重新渲染，并从当前发出的行
//! 数重建排队稳定区域。这故意避免在
//! 流处于活动状态时的字节级重映射复杂性；最终确定的内容通过转录
//! 合并规范化为源支持的 markdown 单元格。
//!
//! ## 不变量
//!
//! - `emitted_stable_len <= enqueued_stable_len <= render.lines.len()`。
//! - 提交的源是追加的，直到 `reset()`；在流中间不会被修改。
//! - 尾部正好在 `enqueued_stable_len` 处开始。
//! - 在确认的表流期间，只有从表标题开始的行
//!   被强制进入尾部；表之前的行可能保持稳定。

use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::history_cell::HistoryRenderMode;
use crate::tui_core::history_cell::{self};
use crate::tui_core::inline_visualization::InlineVisualizationContext;
use crate::tui_core::markdown::render_markdown_agent_with_links_cwd_and_visualizations;
use crate::tui_core::style::proposed_plan_style;
use crate::tui_core::terminal_hyperlinks::HyperlinkLine;
use crate::tui_core::terminal_hyperlinks::prefix_hyperlink_lines;
use ratatui::prelude::Stylize;
use ratatui::text::Line;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use super::StreamState;
use super::render::StreamingRender;
use super::render::render_source;
use super::table_holdback::TableHoldbackScanner;
use super::table_holdback::TableHoldbackState;
#[cfg(test)]
use super::table_holdback::table_holdback_state;

// ---------------------------------------------------------------------------
// StreamCore — 两个流控制器的共享簿记
// ---------------------------------------------------------------------------

/// 双区域流模型的共享状态和逻辑。
///
/// [`StreamController`]（代理消息）和 [`PlanStreamController`]
/// （提议计划）在此委托其核心簿记：源
/// 累积、重新渲染、稳定/尾部分区、提交动画
/// 队列管理和终端调整大小处理。
///
/// 包装控制器仅添加自己的 `emit()` 样式和
/// finalize 返回类型。
struct StreamCore {
    state: StreamState,
    /// 当前渲染宽度（可用于 markdown 内容的列数）。
    width: Option<usize>,
    /// 在 `width` 下对已提交源码的增量渲染。
    render: StreamingRender,
    /// 已入队到提交动画队列的行数。
    enqueued_stable_len: usize,
    /// 实际已发射到 scrollback 的行数。
    emitted_stable_len: usize,
    /// 会话 cwd：用于在流重新渲染期间保持本地文件链接显示稳定。
    cwd: PathBuf,
    inline_visualization_context: Option<InlineVisualizationContext>,
    render_mode: HistoryRenderMode,
    /// 以源起点和宽度为键缓存的「表之前前缀」的渲染行数。
    stable_prefix_len_cache: Option<StablePrefixLenCache>,
    /// 针对追加式源码更新的增量保持（holdback）扫描器状态。
    holdback_scanner: TableHoldbackScanner,
}

struct StablePrefixLenCache {
    /// 候选表/标题起点在已提交源码中的字节偏移。
    source_start: usize,
    /// 产生 `stable_prefix_len` 的宽度。
    width: Option<usize>,
    /// 在 `width` 下、`source_start` 之前的已提交前缀的渲染行数。
    ///
    /// 当活跃的表格尾部仍在不断变化时，流控制器利用该值避免反复重新渲染
    /// 同一个稳定前缀。
    stable_prefix_len: usize,
}

impl StreamCore {
    fn new(
        width: Option<usize>,
        cwd: &Path,
        render_mode: HistoryRenderMode,
        inline_visualization_context: Option<InlineVisualizationContext>,
    ) -> Self {
        Self {
            state: StreamState::new(width, cwd),
            width,
            render: StreamingRender::new(),
            enqueued_stable_len: 0,
            emitted_stable_len: 0,
            cwd: cwd.to_path_buf(),
            inline_visualization_context,
            render_mode,
            stable_prefix_len_cache: None,
            holdback_scanner: TableHoldbackScanner::new(),
        }
    }

    /// 推送一个流式 delta，并将任何新稳定的渲染行入队。
    ///
    /// 只有以换行符结尾的源码才会被提交用于渲染。这对表格很重要，因为
    /// 未终止的不完整行在结构明确之前必须暂留在稳定队列和 live 尾部之外；
    /// 否则用户会短暂看到畸形的列，并在下一个 delta 到来时立刻消失。
    fn push_delta(&mut self, delta: &str) -> bool {
        if !delta.is_empty() {
            self.state.has_seen_delta = true;
        }
        self.state.collector.push_delta(delta);

        let mut enqueued = false;
        if delta.contains('\n')
            && let Some(range) = self.state.collector.commit_complete_source()
        {
            let source = self.state.collector.committed_source();
            let committed_source = &source[range];
            self.holdback_scanner.push_source_chunk(committed_source);
            self.render.append(
                source,
                committed_source,
                self.width,
                self.cwd.as_path(),
                self.render_mode,
                self.inline_visualization_context.as_ref(),
            );
            enqueued = self.sync_stable_queue();
        }
        enqueued
    }

    /// 清空收集器，渲染最终的源码快照，并返回尚未发出的行。
    ///
    /// 这里刻意从完整原始源码重新渲染，而不是尝试把已入队的稳定行与当前
    /// 尾部拼接起来。最终的渲染是用于合并的规范转录表示，因此跳过
    /// `reset()` 的调用方可能会意外地把已结束的流重放到下一个回答中。
    fn finalize_remaining(&mut self) -> (Vec<HyperlinkLine>, String) {
        let source = self.state.collector.finalize_and_take_source();
        let mut rendered = render_source(
            &source,
            self.width,
            self.cwd.as_path(),
            self.render_mode,
            self.inline_visualization_context.as_ref(),
        );
        let remaining = rendered.split_off(self.emitted_stable_len.min(rendered.len()));
        (remaining, source)
    }

    /// 步进动画：出队一行，更新已发射计数。
    fn tick(&mut self) -> Vec<HyperlinkLine> {
        let step = self.state.step();
        self.emitted_stable_len += step.len();
        step
    }

    /// 批量清空：出队最多 `max_lines` 行，更新已发射计数。
    fn tick_batch(&mut self, max_lines: usize) -> Vec<HyperlinkLine> {
        if max_lines == 0 {
            return Vec::new();
        }
        let step = self.state.drain_n(max_lines);
        if step.is_empty() {
            return step;
        }
        self.emitted_stable_len += step.len();
        step
    }

    // 简单的 StreamCore 访问器被内联——在活动流式传输期间，每次动画
    // tick 和渲染帧都会被调用。

    #[inline]
    fn is_idle(&self) -> bool {
        self.state.is_idle()
    }

    #[inline]
    fn queued_lines(&self) -> usize {
        self.state.queued_len()
    }

    #[inline]
    fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.state.oldest_queued_age(now)
    }

    /// 属于可变尾部、尚未入队进行稳定提交的行。
    ///
    /// 尾部从 `enqueued_stable_len` 开始，因此这里返回当前渲染快照中
    /// 仍允许变化、且不会破坏 scrollback 顺序的部分。如果调用方改从
    /// `emitted_stable_len` 推导尾部，那么已入队但尚未发出的行可能会
    /// 重新出现在活动单元格中，在屏幕上造成内容重复。
    #[inline]
    fn current_tail_lines(&self) -> Vec<HyperlinkLine> {
        let start = self.enqueued_stable_len.min(self.render.lines.len());
        self.render.lines[start..].to_vec()
    }

    #[inline]
    fn has_tail(&self) -> bool {
        self.enqueued_stable_len < self.render.lines.len()
    }

    /// 更新渲染宽度，并为新布局重建已入队的稳定行。
    ///
    /// 在新宽度下重新渲染一次，并根据当前已发射行数重建队列状态。
    ///
    /// 调整大小是源码驱动渲染最关键的时机：先前已发射的正文必须保持
    /// scrollback 顺序，而任何活跃的表格尾部都可以在新宽度下自由重塑。
    /// 本方法在保留这一划分的同时，不尝试逐字节的行重映射。
    fn set_width(&mut self, width: Option<usize>) {
        if self.width == width {
            return;
        }
        let had_pending_queue = self.state.queued_len() > 0;
        let had_live_tail = self.has_tail();
        self.width = width;
        self.state.collector.set_width(width);
        let source = self.state.collector.committed_source();
        if source.is_empty() {
            return;
        }

        self.render.recompute(
            source,
            self.width,
            self.cwd.as_path(),
            self.render_mode,
            self.inline_visualization_context.as_ref(),
        );
        self.emitted_stable_len = self.emitted_stable_len.min(self.render.lines.len());
        if had_pending_queue
            && self.emitted_stable_len == self.render.lines.len()
            && self.emitted_stable_len > 0
        {
            // 如果换行后的剩余部分在新宽度下压缩成更少的行，
            // 则至少保留一行未发出，以免调整大小前待处理的内容
            // 被永久跳过。
            self.emitted_stable_len -= 1;
        }
        self.state.clear_queue();
        if self.emitted_stable_len > 0 && !had_pending_queue && !had_live_tail {
            // 当队列中没有等待的稳定行、且没有需要保留的可变尾部时，
            // 避免在调整大小后重放已经发出的内容。
            self.enqueued_stable_len = self.render.lines.len();
            return;
        }
        self.rebuild_stable_queue_from_render();
    }

    /// 清除当前流的所有累积状态。
    fn reset(&mut self) {
        self.state.clear();
        self.render.clear();
        self.enqueued_stable_len = 0;
        self.emitted_stable_len = 0;
        self.stable_prefix_len_cache = None;
        self.holdback_scanner.reset();
    }

    fn set_render_mode(&mut self, render_mode: HistoryRenderMode) {
        if self.render_mode == render_mode {
            return;
        }

        let had_pending_queue = self.state.queued_len() > 0;
        let had_live_tail = self.has_tail();
        self.render_mode = render_mode;
        let source = self.state.collector.committed_source();
        if source.is_empty() {
            return;
        }

        self.render.recompute(
            source,
            self.width,
            self.cwd.as_path(),
            self.render_mode,
            self.inline_visualization_context.as_ref(),
        );
        self.emitted_stable_len = self.emitted_stable_len.min(self.render.lines.len());
        if had_pending_queue
            && self.emitted_stable_len == self.render.lines.len()
            && self.emitted_stable_len > 0
        {
            self.emitted_stable_len -= 1;
        }
        self.state.clear_queue();
        if self.emitted_stable_len > 0 && !had_pending_queue && !had_live_tail {
            self.enqueued_stable_len = self.render.lines.len();
            return;
        }
        self.rebuild_stable_queue_from_render();
    }

    /// 计算稳定区域应包含多少渲染行。
    fn compute_target_stable_len(&mut self) -> usize {
        let tail_budget = self.active_tail_budget_lines();
        self.render
            .lines
            .len()
            .saturating_sub(tail_budget)
            .max(self.emitted_stable_len)
    }

    /// 将 `enqueued_stable_len` 向目标稳定边界推进，并入队任何
    /// 新稳定的行。如果入队了新行，则返回 `true`。
    fn sync_stable_queue(&mut self) -> bool {
        let target_stable_len = self.compute_target_stable_len();

        // 结构性重写把稳定边界向后移进了「已入队但未发出」的行。
        // 从最新的快照重建队列。
        if target_stable_len < self.enqueued_stable_len {
            self.state.clear_queue();
            if self.emitted_stable_len < target_stable_len {
                self.state.enqueue(
                    self.render.lines[self.emitted_stable_len..target_stable_len].to_vec(),
                );
            }
            self.enqueued_stable_len = target_stable_len;
            return self.state.queued_len() > 0;
        }

        if target_stable_len == self.enqueued_stable_len {
            return false;
        }

        self.state
            .enqueue(self.render.lines[self.enqueued_stable_len..target_stable_len].to_vec());
        self.enqueued_stable_len = target_stable_len;
        true
    }

    /// 从当前渲染快照重建稳定队列。
    ///
    /// 在 `set_width()` 之后使用：彼时任何已入队的行都是按旧宽度计算的，
    /// 不能再保证与当前渲染对齐。
    fn rebuild_stable_queue_from_render(&mut self) {
        let target_stable_len = self.compute_target_stable_len();
        self.state.clear_queue();
        if self.emitted_stable_len < target_stable_len {
            self.state
                .enqueue(self.render.lines[self.emitted_stable_len..target_stable_len].to_vec());
        }
        self.enqueued_stable_len = target_stable_len;
    }

    /// 作为可变尾部保留多少渲染行。
    ///
    /// 检测到表格时（`Confirmed` 或 `PendingHeader`），整个表格区域都会被
    /// 保留为尾部，因为添加一行可能会重塑表格的列宽。对于
    /// `PendingHeader`，只有从推测的标题行开始的内容保持可变，以便更早的
    /// 正文可以继续流式输出。未检测到表格时，所有内容直接流入稳定区。
    /// 这是保持（holdback）机制的核心决策点。
    fn active_tail_budget_lines(&mut self) -> usize {
        if self.render_mode == HistoryRenderMode::Raw {
            return 0;
        }
        let scan_start = Instant::now();
        let holdback_state = self.holdback_scanner.state();
        let tail_budget = match holdback_state {
            TableHoldbackState::Confirmed { table_start: start }
            | TableHoldbackState::PendingHeader {
                header_start: start,
            } => self.tail_budget_from_source_start(start),
            TableHoldbackState::None => 0,
        };
        tracing::trace!(
            state = ?holdback_state,
            tail_budget,
            elapsed_us = scan_start.elapsed().as_micros(),
            "table holdback decision",
        );
        tail_budget
    }

    /// 把原始源码边界转换为渲染尾部的行数。
    ///
    /// 这里的重要约定是：保持扫描器以字节偏移为单位进行推理，而队列在
    /// 渲染行上运作。本辅助函数是这两个坐标系唯一被桥接的地方。
    fn tail_budget_from_source_start(&mut self, source_start: usize) -> usize {
        if source_start == 0 {
            return self.render.lines.len();
        }
        let source_start = source_start.min(self.state.collector.committed_source().len());
        let stable_prefix_len = self.stable_prefix_len_for_source_start(source_start);
        self.render.lines.len().saturating_sub(stable_prefix_len)
    }

    /// 渲染 `source_start` 之前的稳定前缀并返回其行数。
    ///
    /// 该值会被缓存，因为密集的表格流在标题/分隔符/正文仍在增量到达时，
    /// 可能为每一行已提交的行都会调用此路径。
    fn stable_prefix_len_for_source_start(&mut self, source_start: usize) -> usize {
        if let Some(cache) = &self.stable_prefix_len_cache
            && cache.source_start == source_start
            && cache.width == self.width
        {
            tracing::trace!(
                source_start,
                width = ?self.width,
                stable_prefix_len = cache.stable_prefix_len,
                "table holdback stable-prefix cache hit",
            );
            return cache.stable_prefix_len;
        }

        let render_start = Instant::now();
        let source = self.state.collector.committed_source();
        let stable_prefix_render = render_markdown_agent_with_links_cwd_and_visualizations(
            &source[..source_start.min(source.len())],
            self.width,
            Some(self.cwd.as_path()),
            self.inline_visualization_context.as_ref(),
        );
        let stable_prefix_len = stable_prefix_render.len();
        tracing::trace!(
            source_start,
            width = ?self.width,
            stable_prefix_len,
            elapsed_us = render_start.elapsed().as_micros(),
            "table holdback stable-prefix render",
        );
        self.stable_prefix_len_cache = Some(StablePrefixLenCache {
            source_start,
            width: self.width,
            stable_prefix_len,
        });
        stable_prefix_len
    }
}

/// 用于流式传输代理消息内容的控制器，支持表感知保持。
///
/// 包装 [`StreamCore`] 并添加 `AgentMessageCell` 发射样式。
pub(crate) struct StreamController {
    core: StreamCore,
    header_emitted: bool,
}

impl StreamController {
    /// 创建一个控制器，其 markdown 渲染器会将本地文件链接相对 `cwd` 缩短。
    ///
    /// `width` 是可供 markdown 渲染使用的内容宽度，未必等于完整的
    /// 终端宽度。调整大小后如果仍在传入陈旧的宽度，已入队的实时输出将
    /// 继续按旧视口折行，直到应用层重排修复最终确定的转录。
    #[cfg(test)]
    pub(crate) fn new(width: Option<usize>, cwd: &Path, render_mode: HistoryRenderMode) -> Self {
        Self::new_with_inline_visualizations(
            width,
            cwd,
            render_mode,
            /*inline_visualization_context*/ None,
        )
    }

    pub(crate) fn new_with_inline_visualizations(
        width: Option<usize>,
        cwd: &Path,
        render_mode: HistoryRenderMode,
        inline_visualization_context: Option<InlineVisualizationContext>,
    ) -> Self {
        Self {
            core: StreamCore::new(width, cwd, render_mode, inline_visualization_context),
            header_emitted: false,
        }
    }

    pub(crate) fn push(&mut self, delta: &str) -> bool {
        self.core.push_delta(delta)
    }

    /// 终结当前流。返回最终的单元格（如果还有剩余行）以及用于合并的原始
    /// markdown 源码。
    pub(crate) fn finalize(&mut self) -> (Option<Box<dyn HistoryCell>>, Option<String>) {
        let (remaining, source) = self.core.finalize_remaining();
        if source.is_empty() {
            self.core.reset();
            return (None, None);
        }

        let out = self.emit(remaining);
        self.core.reset();
        (out, Some(source))
    }

    pub(crate) fn on_commit_tick(&mut self) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.core.tick();
        (self.emit(step), self.core.is_idle())
    }

    pub(crate) fn on_commit_tick_batch(
        &mut self,
        max_lines: usize,
    ) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.core.tick_batch(max_lines);
        (self.emit(step), self.core.is_idle())
    }

    // 精简的 StreamController 访问器被内联——在每次渲染帧和动画
    // tick 时调用的一行式委托。

    #[inline]
    pub(crate) fn queued_lines(&self) -> usize {
        self.core.queued_lines()
    }

    pub(crate) fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.core.oldest_queued_age(now)
    }

    #[inline]
    pub(crate) fn current_tail_lines(&self) -> Vec<HyperlinkLine> {
        self.core.current_tail_lines()
    }

    #[inline]
    pub(crate) fn tail_starts_stream(&self) -> bool {
        !self.header_emitted && self.core.enqueued_stable_len == 0
    }

    #[inline]
    pub(crate) fn has_live_tail(&self) -> bool {
        self.core.has_tail()
    }

    /// 已流式提交（含换行）的源码字节长度，对应 `state.live` 的前缀。
    ///
    /// 主循环据此把 `state.live` 切成「已进 scrollback」与「仍在 live 区」两段，
    /// live 区只显示后段，避免与滚屏重复。`drain_committed_to_lines` 后调用即可。
    #[inline]
    pub(crate) fn committed_source_len(&self) -> usize {
        self.core.state.collector.committed_source().len()
    }

    pub(crate) fn clear_queue(&mut self) {
        self.core.state.clear_queue();
        self.core.enqueued_stable_len = self.core.emitted_stable_len;
    }

    pub(crate) fn set_width(&mut self, width: Option<usize>) {
        self.core.set_width(width);
    }

    pub(crate) fn set_render_mode(&mut self, render_mode: HistoryRenderMode) {
        self.core.set_render_mode(render_mode);
    }

    fn emit(&mut self, lines: Vec<HyperlinkLine>) -> Option<Box<dyn HistoryCell>> {
        if lines.is_empty() {
            return None;
        }
        Some(Box::new(
            history_cell::AgentMessageCell::new_hyperlink_lines(lines, {
                let header_emitted = self.header_emitted;
                self.header_emitted = true;
                !header_emitted
            }),
        ))
    }

    // ── 反射 TUI 主循环接线:把内部 commit 队列 / tail 转成可直接写进 ──
    // ── scrollback 的 `Vec<Line>`(经 AgentMessageCell/StreamingAgentTailCell ──
    // ── 的 display_lines 统一缩进与字形,保证与既有 history 渲染一致)。  ──

    /// 一次性把 commit 队列里**已稳定**的行全部吐出并渲染为 `Vec<Line>`。
    ///
    /// 用于 agent 文本流式进入 scrollback:每收到 delta 调 `push` 后,主循环
    /// 调本方法把「可安全提交的行」按行序取走(经 `display_lines` 应用宽度
    /// 折行与首行 `• ` 前缀)。这样文字逐行流进滚屏,工具调用落滚屏时,它
    /// 前面的文字已在上面 —— 自然交错,不再割裂成两块。
    ///
    /// 内部走 `on_commit_tick_batch`(批量 drain,而非逐行动画),避免在快速
    /// 流式时 commit 队列无界堆积。`width` 即 scrollback 内容宽度(通常
    /// `终端宽度 - 2`)。
    pub(crate) fn drain_committed_to_lines(&mut self, width: u16) -> Vec<Line<'static>> {
        let mut out: Vec<Line<'static>> = Vec::new();
        // 队列非空就一轮一轮 batch drain,直到空闲。每轮上限用当前队列深度,
        // 保证至多两轮即可清空(第一轮取走全部入队时的快照)。
        while self.core.queued_lines() > 0 {
            let batch = self.core.queued_lines();
            let step = self.core.tick_batch(batch);
            if step.is_empty() {
                break;
            }
            if let Some(cell) = self.emit(step) {
                out.extend(cell.display_lines(width));
            }
        }
        out
    }

    /// 终结当前流:把队列里残留稳定行 + 可变 tail 全部 commit 为 `Vec<Line>`。
    ///
    /// 在 `TurnCompleted` 时调用:先 `drain_committed_to_lines` 取走稳定行,
    /// 再 `finalize()` 把 tail(如进行中的表格末尾)整段渲染出来。返回值即
    /// 整段未落 scrollback 的文本行;调用方据此直接 `insert_history_lines`,
    /// **不再**把整段塞回 `state.history` 作为一条 `Agent`。
    ///
    /// `width` 含义同 `drain_committed_to_lines`。返回 `(lines, source)`:
    /// `lines` 用于渲染,`source` 是原始 markdown(供 transcript/复制 overlay)。
    pub(crate) fn finalize_to_lines(&mut self, width: u16) -> (Vec<Line<'static>>, Option<String>) {
        let mut out = self.drain_committed_to_lines(width);
        let (cell, source) = self.finalize();
        if let Some(cell) = cell {
            out.extend(cell.display_lines(width));
        }
        (out, source)
    }

    /// 当前「可变 tail」渲染为 `Vec<Line>`(供 live 区展示正在变的最后几行)。
    ///
    /// 经 `StreamingAgentTailCell` 应用首行 `• ` 前缀;返回空表示无 tail
    /// (全部文本已稳定提交)。当 live 区用本方法渲染时,它只显示 tail,
    /// 而非整段累积文本,从而与 scrollback 的已提交前缀不重复。
    pub(crate) fn tail_display_lines(&mut self, width: u16) -> Vec<Line<'static>> {
        if !self.has_live_tail() {
            return Vec::new();
        }
        let is_first = self.tail_starts_stream();
        let cell = history_cell::StreamingAgentTailCell::new(self.current_tail_lines(), is_first);
        cell.display_lines(width)
    }
}
// ---------------------------------------------------------------------------
// PlanStreamController — 提议计划流
// ---------------------------------------------------------------------------

/// 将提议计划 markdown 流式传输到样式化计划块的控制器。
///
/// 包装 [`StreamCore`] 并添加计划特定的标题、缩进和
/// 背景样式。
pub(crate) struct PlanStreamController {
    core: StreamCore,
    header_emitted: bool,
}

impl PlanStreamController {
    /// 创建一个计划流控制器，其 markdown 渲染器会将本地文件链接相对 `cwd` 缩短。
    ///
    /// `width` 的含义与 `StreamController` 中相同：即 markdown 正文宽度，
    /// 调用方必须在终端宽度变化时对其进行更新。
    pub(crate) fn new(width: Option<usize>, cwd: &Path, render_mode: HistoryRenderMode) -> Self {
        Self {
            core: StreamCore::new(
                width,
                cwd,
                render_mode,
                /*inline_visualization_context*/ None,
            ),
            header_emitted: false,
        }
    }

    pub(crate) fn push(&mut self, delta: &str) -> bool {
        self.core.push_delta(delta)
    }

    /// 终结当前流。返回最终的单元格（如果还有剩余行）以及用于合并的
    /// 原始 markdown 源码。
    pub(crate) fn finalize(&mut self) -> (Option<Box<dyn HistoryCell>>, Option<String>) {
        let (remaining, source) = self.core.finalize_remaining();
        if source.is_empty() {
            self.core.reset();
            return (None, None);
        }

        let out = self.emit(remaining, /*include_bottom_padding*/ true);
        self.core.reset();
        (out, Some(source))
    }

    pub(crate) fn on_commit_tick(&mut self) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.core.tick();
        (
            self.emit(step, /*include_bottom_padding*/ false),
            self.core.is_idle(),
        )
    }

    pub(crate) fn on_commit_tick_batch(
        &mut self,
        max_lines: usize,
    ) -> (Option<Box<dyn HistoryCell>>, bool) {
        let step = self.core.tick_batch(max_lines);
        (
            self.emit(step, /*include_bottom_padding*/ false),
            self.core.is_idle(),
        )
    }

    #[inline]
    pub(crate) fn queued_lines(&self) -> usize {
        self.core.queued_lines()
    }

    #[inline]
    pub(crate) fn has_live_tail(&self) -> bool {
        self.core.has_tail()
    }

    #[inline]
    pub(crate) fn current_tail_lines(&self) -> Vec<HyperlinkLine> {
        self.core.current_tail_lines()
    }

    #[inline]
    pub(crate) fn tail_starts_stream(&self) -> bool {
        !self.header_emitted && self.core.enqueued_stable_len == 0
    }

    pub(crate) fn current_tail_display_lines(&self) -> Vec<HyperlinkLine> {
        let lines = self.current_tail_lines();
        if lines.is_empty() {
            return Vec::new();
        }
        self.render_display_lines(lines, /*include_bottom_padding*/ false)
    }

    pub(crate) fn oldest_queued_age(&self, now: Instant) -> Option<Duration> {
        self.core.oldest_queued_age(now)
    }

    pub(crate) fn clear_queue(&mut self) {
        self.core.state.clear_queue();
        self.core.enqueued_stable_len = self.core.emitted_stable_len;
    }

    pub(crate) fn set_width(&mut self, width: Option<usize>) {
        self.core.set_width(width);
    }

    pub(crate) fn set_render_mode(&mut self, render_mode: HistoryRenderMode) {
        self.core.set_render_mode(render_mode);
    }

    fn emit(
        &mut self,
        lines: Vec<HyperlinkLine>,
        include_bottom_padding: bool,
    ) -> Option<Box<dyn HistoryCell>> {
        if lines.is_empty() && !include_bottom_padding {
            return None;
        }

        let is_stream_continuation = self.header_emitted;
        let out_lines = self.render_display_lines(lines, include_bottom_padding);
        self.header_emitted = true;

        Some(Box::new(history_cell::new_proposed_plan_stream(
            out_lines,
            is_stream_continuation,
        )))
    }

    fn render_display_lines(
        &self,
        lines: Vec<HyperlinkLine>,
        include_bottom_padding: bool,
    ) -> Vec<HyperlinkLine> {
        let mut out_lines: Vec<HyperlinkLine> = Vec::with_capacity(/*capacity*/ 4);
        if !self.header_emitted {
            out_lines.push(HyperlinkLine::new(
                vec!["• ".dim(), "Proposed Plan".bold()].into(),
            ));
            out_lines.push(HyperlinkLine::new(Line::from(" ")));
        }

        let mut plan_lines: Vec<HyperlinkLine> = Vec::with_capacity(/*capacity*/ 4);
        // 顶部留白已由 header 行（"• Proposed Plan"）后的空行分隔符提供，这里不再追加，
        // 否则首次渲染时 header 分隔符与 top_padding 会连成两个连续空行。
        plan_lines.extend(lines);
        if include_bottom_padding {
            plan_lines.push(HyperlinkLine::new(Line::from(" ")));
        }

        let plan_style = proposed_plan_style();
        let plan_lines = prefix_hyperlink_lines(plan_lines, "  ".into(), "  ".into())
            .into_iter()
            .map(|line| line.style(plan_style))
            .collect::<Vec<_>>();
        out_lines.extend(plan_lines);
        out_lines
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
