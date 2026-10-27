//! 在备用屏幕中渲染的覆盖层 UI。
//!
//! 此模块实现 TUI 使用的分页器风格覆盖层，包括
//! 转录覆盖层（`Ctrl+T`），它渲染独立于主视口的完整历史视图。
//!
//! 转录覆盖层渲染已提交的转录单元格，外加从当前进行中的活动单元格
//! 派生的可选只渲染实时尾部。因为在每次绘制时重建折行 `Line` 的
//! 开销可能很大，实时尾部会被缓存，仅在其缓存键变化时才重新计算；
//! 缓存键派生自终端宽度（折行）、活动单元格修订版本（就地修改）、
//! 流延续标志（间距）和动画刻度（基于时间的 spinner/shimmer 输出）。
//!
//! 转录覆盖层的实时尾部由 `App` 在绘制期间保持同步：`App` 提供
//! `ActiveCellTranscriptKey` 和一个计算活动单元格转录行的函数，
//! `TranscriptOverlay::sync_live_tail` 使用该键决定缓存尾部何时必须
//! 重新计算。`ChatWidget` 负责生成一个在活动单元格就地修改或其
//! 转录输出与时间相关时会发生变化的键。

use std::io::Result;
use std::sync::Arc;

use crate::tui_core::chatwidget::ActiveCellTranscriptKey;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::history_cell::UserHistoryCell;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::keymap::PagerKeymap;
use crate::tui_core::render::Insets;
use crate::tui_core::render::renderable::InsetRenderable;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::style::user_message_style;
use crate::tui_core::terminal_hyperlinks::HyperlinkLine;
use crate::tui_core::terminal_hyperlinks::mark_buffer_hyperlinks;
use crate::tui_core::terminal_hyperlinks::visible_lines_ref;
use crate::tui_core::tui;
use crate::tui_core::tui::TuiEvent;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::buffer::Cell;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

pub(crate) enum Overlay {
    Transcript(TranscriptOverlay),
    Static(StaticOverlay),
}

impl Overlay {
    pub(crate) fn new_transcript(cells: Vec<Arc<dyn HistoryCell>>, keymap: PagerKeymap) -> Self {
        Self::Transcript(TranscriptOverlay::new(cells, keymap))
    }

    pub(crate) fn new_static_with_lines(
        lines: Vec<Line<'static>>,
        title: String,
        keymap: PagerKeymap,
    ) -> Self {
        Self::Static(StaticOverlay::with_title(lines, title, keymap))
    }

    pub(crate) fn new_static_with_renderables(
        renderables: Vec<Box<dyn Renderable>>,
        title: String,
        keymap: PagerKeymap,
    ) -> Self {
        Self::Static(StaticOverlay::with_renderables(renderables, title, keymap))
    }

    pub(crate) fn handle_event(&mut self, tui: &mut tui::Tui, event: TuiEvent) -> Result<()> {
        match self {
            Overlay::Transcript(o) => o.handle_event(tui, event),
            Overlay::Static(o) => o.handle_event(tui, event),
        }
    }

    pub(crate) fn is_done(&self) -> bool {
        match self {
            Overlay::Transcript(o) => o.is_done(),
            Overlay::Static(o) => o.is_done(),
        }
    }
}

fn first_or_empty(bindings: &[KeyBinding]) -> Vec<KeyBinding> {
    bindings.first().copied().into_iter().collect()
}

// 从（键，描述）对渲染单行按键提示。
fn render_key_hints(area: Rect, buf: &mut Buffer, pairs: &[(Vec<KeyBinding>, &str)]) {
    let mut spans: Vec<Span<'static>> = vec![" ".into()];
    let mut first = true;
    for (keys, desc) in pairs {
        if !first {
            spans.push("   ".into());
        }
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                spans.push("/".into());
            }
            spans.push(Span::from(key));
        }
        spans.push(" ".into());
        spans.push(Span::from(desc.to_string()));
        first = false;
    }
    Paragraph::new(vec![Line::from(spans).dim()]).render_ref(area, buf);
}

/// 用于渲染分页器视图的通用组件。
// ── pager 视图（外移子模块） ──
mod pager_view;
use pager_view::*;

// ── 渲染适配器（外移子模块） ──
mod renderables;
use renderables::*;

pub(crate) struct TranscriptOverlay {
    /// 分页器 UI 状态和当前显示的 renderable。
    ///
    /// 不变量是 `view.renderables` 为 `render_cells(cells)` 加上一个可选的
    /// 追加在已提交单元格之后的实时尾部 renderable。
    view: PagerView,
    /// 已提交的转录单元格（不包括实时尾部）。
    cells: Vec<Arc<dyn HistoryCell>>,
    highlight_cell: Option<usize>,
    /// 追加在已提交单元格之后的只渲染实时尾部的缓存键。
    live_tail_key: Option<LiveTailKey>,
    is_done: bool,
}

/// 追加到转录覆盖层的活动单元格"实时尾部"的缓存键。
///
/// 更改任何字段都意味着不同的渲染尾部。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LiveTailKey {
    /// 当前终端宽度，影响折行。
    width: u16,
    /// 在活动单元格转录就地更新时变化的修订版本。
    revision: u64,
    /// 尾部是否应被视为间距上的延续。
    is_stream_continuation: bool,
    /// 用于刷新 spinner/进度指示器的可选动画刻度。
    animation_tick: Option<u64>,
}

impl TranscriptOverlay {
    /// 为一组固定的已提交单元格创建转录覆盖层。
    ///
    /// 此覆盖层不拥有"活动单元格"；调用方可以在绘制期间通过
    /// `sync_live_tail` 选择性地追加实时尾部以反映进行中的活动。
    pub(crate) fn new(transcript_cells: Vec<Arc<dyn HistoryCell>>, keymap: PagerKeymap) -> Self {
        Self {
            view: PagerView::new(
                Self::render_cells(&transcript_cells, /*highlight_cell*/ None),
                "T R A N S C R I P T".to_string(),
                usize::MAX,
                keymap,
            ),
            cells: transcript_cells,
            highlight_cell: None,
            live_tail_key: None,
            is_done: false,
        }
    }

    fn render_cells(
        cells: &[Arc<dyn HistoryCell>],
        highlight_cell: Option<usize>,
    ) -> Vec<Box<dyn Renderable>> {
        cells
            .iter()
            .enumerate()
            .map(|(i, cell)| Self::render_cell(cell, i, highlight_cell))
            .collect()
    }

    /// 为已提交单元格构建 renderable，并在单元格稳定时缓存其高度。
    fn render_cell(
        cell: &Arc<dyn HistoryCell>,
        index: usize,
        highlight_cell: Option<usize>,
    ) -> Box<dyn Renderable> {
        let cell_renderable = CellRenderable {
            cell: cell.clone(),
            highlighted: highlight_cell == Some(index),
        };
        let mut cell_renderable: Box<dyn Renderable> = if cell.has_stable_transcript_height() {
            Box::new(CachedRenderable::new(cell_renderable))
        } else {
            Box::new(cell_renderable)
        };
        if !cell.is_stream_continuation() && index > 0 {
            cell_renderable = Box::new(InsetRenderable::new(
                cell_renderable,
                Insets::tlbr(
                    /*top*/ 1, /*left*/ 0, /*bottom*/ 0, /*right*/ 0,
                ),
            ));
        }
        cell_renderable
    }

    /// 插入一个已提交的历史单元格，同时保留任何缓存的实时尾部。
    ///
    /// 实时尾部被暂时移除，新的已提交单元格被追加，
    /// 然后尾部被重新附加。如果尾部之前因为是唯一的 renderable
    /// 而没有前导间距，我们在第一个已提交单元格到达时补上缺失的 inset。
    ///
    /// 这期望 `cell` 是一个已提交的转录单元格（而非进行中的活动单元格）。
    /// 如果覆盖层在插入前滚动到底部，插入后仍保持固定在底部，
    /// 以保留"跟随滚动"行为。
    pub(crate) fn insert_cell(&mut self, cell: Arc<dyn HistoryCell>) {
        let follow_bottom = self.view.is_scrolled_to_bottom();
        let had_prior_cells = !self.cells.is_empty();
        let tail_renderable = self.take_live_tail_renderable();
        let cell_renderable = Self::render_cell(&cell, self.cells.len(), self.highlight_cell);
        self.cells.push(cell);
        self.view.renderables.push(cell_renderable);
        if let Some(tail) = tail_renderable {
            let tail = if !had_prior_cells
                && self
                    .live_tail_key
                    .is_some_and(|key| !key.is_stream_continuation)
            {
                // 尾部之前被渲染为唯一条目，因此缺少顶部
                // inset；现在它跟随一个已提交单元格，补上一个。
                Box::new(InsetRenderable::new(
                    tail,
                    Insets::tlbr(
                        /*top*/ 1, /*left*/ 0, /*bottom*/ 0, /*right*/ 0,
                    ),
                )) as Box<dyn Renderable>
            } else {
                tail
            };
            self.view.renderables.push(tail);
        }
        if follow_bottom {
            self.view.scroll_offset = usize::MAX;
        }
    }

    /// 替换已提交的转录单元格，同时保留当前显示在覆盖层末尾的
    /// 任何缓存的进行中输出。
    ///
    /// 这在现有历史被裁剪时使用（例如回滚之后），使
    /// 转录覆盖层立即反映与主转录相同的已提交单元格。
    pub(crate) fn replace_cells(&mut self, cells: Vec<Arc<dyn HistoryCell>>) {
        let follow_bottom = self.view.is_scrolled_to_bottom();
        self.cells = cells;
        if self
            .highlight_cell
            .is_some_and(|idx| idx >= self.cells.len())
        {
            self.highlight_cell = None;
        }
        self.rebuild_renderables();
        if follow_bottom {
            self.view.scroll_offset = usize::MAX;
        }
    }

    /// 用单个合并后的单元格替换一段范围内的已提交单元格。
    ///
    /// 镜像 `ConsolidateAgentMessage` 期间在 `App::transcript_cells` 上
    /// 执行的拼接，使 Ctrl+T 覆盖层与主转录保持
    /// 同步。范围被防御性地截断：覆盖层打开后可能插入了
    /// 单元格，使其条目少于主转录。
    pub(crate) fn consolidate_cells(
        &mut self,
        range: std::ops::Range<usize>,
        consolidated: Arc<dyn HistoryCell>,
    ) {
        let follow_bottom = self.view.is_scrolled_to_bottom();
        // 将范围截断到覆盖层的单元格数，以避免覆盖层单元格
        // 少于主转录时发生 panic（例如覆盖层打开后插入了单元格）。
        let clamped_end = range.end.min(self.cells.len());
        let clamped_start = range.start.min(clamped_end);
        if clamped_start < clamped_end {
            let removed = clamped_end - clamped_start;
            if let Some(highlight_cell) = self.highlight_cell.as_mut()
                && *highlight_cell >= clamped_start
            {
                if *highlight_cell < clamped_end {
                    *highlight_cell = clamped_start;
                } else {
                    *highlight_cell = highlight_cell.saturating_sub(removed.saturating_sub(1));
                }
            }
            self.cells
                .splice(clamped_start..clamped_end, std::iter::once(consolidated));
            if self
                .highlight_cell
                .is_some_and(|highlight_cell| highlight_cell >= self.cells.len())
            {
                self.highlight_cell = None;
            }
            self.rebuild_renderables();
        }
        if follow_bottom {
            self.view.scroll_offset = usize::MAX;
        }
    }

    /// 将活动单元格实时尾部与当前宽度和单元格状态同步。
    ///
    /// 仅在缓存键变化时重新计算尾部，保留滚动
    /// 位置，并在没有可渲染内容时丢弃尾部。
    ///
    /// 覆盖层拥有已提交的转录单元格，而实时尾部派生自当前的
    /// 活动单元格，后者可以在流式传输期间就地修改。`App` 在
    /// `Overlay::Transcript` 的 `TuiEvent::Draw` 期间调用此方法，
    /// 传入一个在活动单元格修改或动画化时变化的键，
    /// 使缓存的尾部保持新鲜。
    ///
    /// 传入一个在活动单元格就地修改时不变化的键，将使尾部在
    /// `Ctrl+T` 中冻结，而主视口继续更新。
    pub(crate) fn sync_live_tail(
        &mut self,
        width: u16,
        active_key: Option<ActiveCellTranscriptKey>,
        compute_lines: impl FnOnce(u16) -> Option<Vec<HyperlinkLine>>,
    ) {
        let next_key = active_key.map(|key| LiveTailKey {
            width,
            revision: key.revision,
            is_stream_continuation: key.is_stream_continuation,
            animation_tick: key.animation_tick,
        });

        if self.live_tail_key == next_key {
            return;
        }
        let follow_bottom = self.view.is_scrolled_to_bottom();

        self.take_live_tail_renderable();
        self.live_tail_key = next_key;

        if let Some(key) = next_key {
            let lines = compute_lines(width).unwrap_or_default();
            if !lines.is_empty() {
                self.view.renderables.push(Self::live_tail_renderable(
                    lines,
                    !self.cells.is_empty(),
                    key.is_stream_continuation,
                ));
            }
        }
        if follow_bottom {
            self.view.scroll_offset = usize::MAX;
        }
    }

    pub(crate) fn set_highlight_cell(&mut self, cell: Option<usize>) {
        self.highlight_cell = cell;
        self.rebuild_renderables();
        if let Some(idx) = self.highlight_cell {
            self.view.scroll_chunk_into_view(idx);
        }
    }

    /// 返回底层分页器视图当前是否固定在底部。
    ///
    /// `App` 绘制循环使用它来决定是否为实时尾部调度动画帧；
    /// 如果用户已向上滚动，我们避免驱动他们看不到的动画工作。
    pub(crate) fn is_scrolled_to_bottom(&self) -> bool {
        self.view.is_scrolled_to_bottom()
    }

    fn rebuild_renderables(&mut self) {
        let tail_renderable = self.take_live_tail_renderable();
        self.view.renderables = Self::render_cells(&self.cells, self.highlight_cell);
        if let Some(tail) = tail_renderable {
            self.view.renderables.push(tail);
        }
    }

    /// 移除并返回缓存的实时尾部 renderable（如果存在）。
    ///
    /// 实时尾部表示为追加在已提交单元格 renderable 之后的单个可选 renderable，
    /// 因此这依赖实时尾部在存在时始终是
    /// `view.renderables` 中的最后一个条目。
    fn take_live_tail_renderable(&mut self) -> Option<Box<dyn Renderable>> {
        (self.view.renderables.len() > self.cells.len()).then(|| self.view.renderables.pop())?
    }

    fn live_tail_renderable(
        lines: Vec<HyperlinkLine>,
        has_prior_cells: bool,
        is_stream_continuation: bool,
    ) -> Box<dyn Renderable> {
        let mut renderable: Box<dyn Renderable> =
            Box::new(CachedRenderable::new(HyperlinkLinesRenderable { lines }));
        if has_prior_cells && !is_stream_continuation {
            renderable = Box::new(InsetRenderable::new(
                renderable,
                Insets::tlbr(
                    /*top*/ 1, /*left*/ 0, /*bottom*/ 0, /*right*/ 0,
                ),
            ));
        }
        renderable
    }

    fn render_hints(&self, area: Rect, buf: &mut Buffer) {
        let line1 = Rect::new(area.x, area.y, area.width, 1);
        let line2 = Rect::new(area.x, area.y.saturating_add(1), area.width, 1);
        render_key_hints(
            line1,
            buf,
            &[
                (
                    first_or_empty(&self.view.keymap.scroll_up)
                        .into_iter()
                        .chain(first_or_empty(&self.view.keymap.scroll_down))
                        .collect(),
                    "to scroll",
                ),
                (
                    first_or_empty(&self.view.keymap.page_up)
                        .into_iter()
                        .chain(first_or_empty(&self.view.keymap.page_down))
                        .collect(),
                    "to page",
                ),
                (
                    first_or_empty(&self.view.keymap.jump_top)
                        .into_iter()
                        .chain(first_or_empty(&self.view.keymap.jump_bottom))
                        .collect(),
                    "to jump",
                ),
            ],
        );

        let mut pairs: Vec<(Vec<KeyBinding>, &str)> =
            vec![(first_or_empty(&self.view.keymap.close), "to quit")];
        if self.highlight_cell.is_some() {
            pairs.push((
                vec![
                    key_hint::plain(KeyCode::Esc),
                    key_hint::plain(KeyCode::Left),
                ],
                "to edit prev",
            ));
            pairs.push((vec![key_hint::plain(KeyCode::Right)], "to edit next"));
            pairs.push((vec![key_hint::plain(KeyCode::Enter)], "to edit message"));
        } else {
            pairs.push((vec![key_hint::plain(KeyCode::Esc)], "to edit prev"));
        }
        render_key_hints(line2, buf, &pairs);
    }

    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let top_h = area.height.saturating_sub(3);
        let top = Rect::new(area.x, area.y, area.width, top_h);
        let bottom = Rect::new(area.x, area.y + top_h, area.width, 3);
        self.view.render(top, buf);
        self.render_hints(bottom, buf);
    }
}

impl TranscriptOverlay {
    pub(crate) fn handle_event(&mut self, tui: &mut tui::Tui, event: TuiEvent) -> Result<()> {
        match event {
            TuiEvent::Key(key_event) => match key_event {
                e if self.view.keymap.close.is_pressed(e)
                    || self.view.keymap.close_transcript.is_pressed(e) =>
                {
                    self.is_done = true;
                    Ok(())
                }
                other => self.view.handle_key_event(tui, other),
            },
            TuiEvent::Draw | TuiEvent::Resize(_, _) => {
                tui.draw(u16::MAX, |frame| {
                    self.render(frame.area(), frame.buffer);
                })?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
    pub(crate) fn is_done(&self) -> bool {
        self.is_done
    }
}

pub(crate) struct StaticOverlay {
    view: PagerView,
    is_done: bool,
}

impl StaticOverlay {
    pub(crate) fn with_title(
        lines: Vec<Line<'static>>,
        title: String,
        keymap: PagerKeymap,
    ) -> Self {
        let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        Self::with_renderables(
            vec![Box::new(CachedRenderable::new(paragraph))],
            title,
            keymap,
        )
    }

    pub(crate) fn with_renderables(
        renderables: Vec<Box<dyn Renderable>>,
        title: String,
        keymap: PagerKeymap,
    ) -> Self {
        Self {
            view: PagerView::new(renderables, title, /*scroll_offset*/ 0, keymap),
            is_done: false,
        }
    }

    fn render_hints(&self, area: Rect, buf: &mut Buffer) {
        let line1 = Rect::new(area.x, area.y, area.width, 1);
        let line2 = Rect::new(area.x, area.y.saturating_add(1), area.width, 1);
        render_key_hints(
            line1,
            buf,
            &[
                (
                    first_or_empty(&self.view.keymap.scroll_up)
                        .into_iter()
                        .chain(first_or_empty(&self.view.keymap.scroll_down))
                        .collect(),
                    "to scroll",
                ),
                (
                    first_or_empty(&self.view.keymap.page_up)
                        .into_iter()
                        .chain(first_or_empty(&self.view.keymap.page_down))
                        .collect(),
                    "to page",
                ),
                (
                    first_or_empty(&self.view.keymap.jump_top)
                        .into_iter()
                        .chain(first_or_empty(&self.view.keymap.jump_bottom))
                        .collect(),
                    "to jump",
                ),
            ],
        );
        let pairs: Vec<(Vec<KeyBinding>, &str)> =
            vec![(first_or_empty(&self.view.keymap.close), "to quit")];
        render_key_hints(line2, buf, &pairs);
    }

    pub(crate) fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let top_h = area.height.saturating_sub(3);
        let top = Rect::new(area.x, area.y, area.width, top_h);
        let bottom = Rect::new(area.x, area.y + top_h, area.width, 3);
        self.view.render(top, buf);
        self.render_hints(bottom, buf);
    }
}

impl StaticOverlay {
    pub(crate) fn handle_event(&mut self, tui: &mut tui::Tui, event: TuiEvent) -> Result<()> {
        match event {
            TuiEvent::Key(key_event) => match key_event {
                e if self.view.keymap.close.is_pressed(e) => {
                    self.is_done = true;
                    Ok(())
                }
                other => self.view.handle_key_event(tui, other),
            },
            TuiEvent::Draw | TuiEvent::Resize(_, _) => {
                tui.draw(u16::MAX, |frame| {
                    self.render(frame.area(), frame.buffer);
                })?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
    pub(crate) fn is_done(&self) -> bool {
        self.is_done
    }
}

fn render_offset_content(
    area: Rect,
    buf: &mut Buffer,
    renderable: &dyn Renderable,
    scroll_offset: u16,
) -> u16 {
    let height = renderable.desired_height(area.width);
    let mut tall_buf = Buffer::empty(Rect::new(
        0,
        0,
        area.width,
        height.min(area.height + scroll_offset),
    ));
    renderable.render(*tall_buf.area(), &mut tall_buf);
    let copy_height = area
        .height
        .min(tall_buf.area().height.saturating_sub(scroll_offset));
    for y in 0..copy_height {
        let src_y = y + scroll_offset;
        for x in 0..area.width {
            buf[(area.x + x, area.y + y)] = tall_buf[(x, src_y)].clone();
        }
    }

    copy_height
}

#[cfg(test)]
#[cfg(test)]
mod tests;
