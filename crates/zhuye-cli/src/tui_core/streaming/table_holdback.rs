//! 面向源码型 agent 流的管道表格 holdback（暂留）扫描器。
//!
//! 含 markdown 表格的 agent 流会把当前活动表格保留为可变尾部，这样新增一行时可以
//! 让更早的表格行重新换行排版，而不是把过时的渲染提交到滚动区。
//!
//! 该扫描器刻意保持保守：它只寻找足够确定可变尾部起点所需的结构，并不会去验证
//! 整个表格或预测最终布局。渲染仍由 markdown 渲染器负责。

use std::time::Instant;

use crate::tui_core::table_detect::FenceKind;
use crate::tui_core::table_detect::FenceTracker;
use crate::tui_core::table_detect::is_table_delimiter_line;
use crate::tui_core::table_detect::is_table_header_line;
use crate::tui_core::table_detect::parse_table_segments;
use crate::tui_core::table_detect::strip_blockquote_prefix;

/// 对累积的原始源码扫描管道表格模式的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableHoldbackState {
    /// 未检测到表格——所有渲染行都可流入稳定队列。
    None,
    /// 最后一个非空行看起来像表头行，但分隔行尚未出现。
    /// 暂留该区域，以防下一个增量就是分隔行。
    PendingHeader { header_start: usize },
    /// 已找到表头 + 分隔行对——源码中确实存在表格。
    /// 从表头往后的内容保持可变。
    Confirmed { table_start: usize },
}

/// 关于上一条已提交源码行记住的信息。
///
/// 扫描器只需要一行回看，因为表格的确认依据是表头行之后紧跟分隔行。
#[derive(Clone, Copy)]
struct PreviousLineState {
    source_start: usize,
    fence_kind: FenceKind,
    is_header: bool,
}

/// 在只追加源码流上进行表格 holdback 状态增量扫描的扫描器。
///
/// `push_source_chunk` 必须按照源码追加到流的相同顺序接收源码。
/// 扫描器在逻辑源码缓冲区上存储字节偏移，因此如果块乱序输入，
/// 后续的尾部边界会指向错误的渲染区域。
pub(super) struct TableHoldbackScanner {
    source_offset: usize,
    fence_tracker: FenceTracker,
    previous_line: Option<PreviousLineState>,
    pending_header_start: Option<usize>,
    confirmed_table_start: Option<usize>,
}

impl TableHoldbackScanner {
    pub(super) fn new() -> Self {
        Self {
            source_offset: 0,
            fence_tracker: FenceTracker::new(),
            previous_line: None,
            pending_header_start: None,
            confirmed_table_start: None,
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    /// 返回已提交源码前缀的当前 holdback 决策。
    ///
    /// `PendingHeader` 表示最新的非空行看起来像表头，但分隔行尚未到达，
    /// 因此调用方应乐观地将该区域保持为可变。`Confirmed` 表示已看到表头与
    /// 分隔行对，所有后续的正文数据行在最终确定之前都保留在活动尾部中。
    pub(super) fn state(&self) -> TableHoldbackState {
        if let Some(table_start) = self.confirmed_table_start {
            TableHoldbackState::Confirmed { table_start }
        } else if let Some(header_start) = self.pending_header_start {
            TableHoldbackState::PendingHeader { header_start }
        } else {
            TableHoldbackState::None
        }
    }

    /// 用新提交的源码推进扫描器。
    ///
    /// 块应只包含现在可以安全提交到 `raw_source` 的源码，通常来自流式收集器的
    /// 以换行结尾的行。不完整的行被有意排除，这样扫描器永远不会把未完成的
    /// 表格行当作稳定的结构信号。
    pub(super) fn push_source_chunk(&mut self, source_chunk: &str) {
        if source_chunk.is_empty() {
            return;
        }

        let scan_start = Instant::now();
        let mut lines = 0usize;
        for source_line in source_chunk.split_inclusive('\n') {
            lines += 1;
            self.push_line(source_line);
        }
        tracing::trace!(
            bytes = source_chunk.len(),
            lines,
            state = ?self.state(),
            elapsed_us = scan_start.elapsed().as_micros(),
            "table holdback incremental scan",
        );
    }

    /// 将一条已提交的源码行纳入扫描器状态机。
    fn push_line(&mut self, source_line: &str) {
        let line = source_line.strip_suffix('\n').unwrap_or(source_line);
        let source_start = self.source_offset;
        let fence_kind = self.fence_tracker.kind();

        let candidate_text = if fence_kind == FenceKind::Other {
            None
        } else {
            table_candidate_text(line)
        };
        let is_header = candidate_text.is_some_and(is_table_header_line);
        let is_delimiter = candidate_text.is_some_and(is_table_delimiter_line);

        if self.confirmed_table_start.is_none()
            && let Some(previous_line) = self.previous_line
            && previous_line.fence_kind != FenceKind::Other
            && fence_kind != FenceKind::Other
            && previous_line.is_header
            && is_delimiter
        {
            self.confirmed_table_start = Some(previous_line.source_start);
            self.pending_header_start = None;
        }

        if self.confirmed_table_start.is_none() && !line.trim().is_empty() {
            if fence_kind != FenceKind::Other && is_header {
                self.pending_header_start = Some(source_start);
            } else {
                self.pending_header_start = None;
            }
        }

        self.previous_line = Some(PreviousLineState {
            source_start,
            fence_kind,
            is_header,
        });

        self.fence_tracker.advance(line);
        self.source_offset = self.source_offset.saturating_add(source_line.len());
    }
}

/// 去除块引用前缀，如果文本包含管道表格片段则返回去空白后的文本，
/// 否则返回 `None`。
///
/// 表格 holdback 把引用表格视为真实表格，但去除了引用标记后仍需是
/// 管道表格的形状。
fn table_candidate_text(line: &str) -> Option<&str> {
    let stripped = strip_blockquote_prefix(line).trim();
    parse_table_segments(stripped).map(|_| stripped)
}

/// 一条带注释的源码行，标注其是否位于围栏代码块内部。
#[cfg(test)]
struct ParsedLine<'a> {
    text: &'a str,
    fence_context: FenceKind,
    source_start: usize,
}

/// 将源码解析为带围栏代码上下文的行，用于表格扫描。
#[cfg(test)]
fn parse_lines_with_fence_state(source: &str) -> Vec<ParsedLine<'_>> {
    let mut tracker = FenceTracker::new();
    let mut lines = Vec::new();
    let mut source_start = 0usize;

    for raw_line in source.split('\n') {
        lines.push(ParsedLine {
            text: raw_line,
            fence_context: tracker.kind(),
            source_start,
        });

        tracker.advance(raw_line);
        source_start = source_start
            .saturating_add(raw_line.len())
            .saturating_add(1);
    }

    lines
}

/// 在非 markdown 围栏代码块之外扫描 `source` 中的管道表格模式。
#[cfg(test)]
pub(super) fn table_holdback_state(source: &str) -> TableHoldbackState {
    let lines = parse_lines_with_fence_state(source);
    for pair in lines.windows(2) {
        let [header_line, delimiter_line] = pair else {
            continue;
        };
        if header_line.fence_context == FenceKind::Other
            || delimiter_line.fence_context == FenceKind::Other
        {
            continue;
        }

        let Some(header_text) = table_candidate_text(header_line.text) else {
            continue;
        };
        let Some(delimiter_text) = table_candidate_text(delimiter_line.text) else {
            continue;
        };

        if is_table_header_line(header_text) && is_table_delimiter_line(delimiter_text) {
            return TableHoldbackState::Confirmed {
                table_start: header_line.source_start,
            };
        }
    }

    let pending_header = lines.iter().rev().find(|line| !line.text.trim().is_empty());
    if let Some(line) = pending_header
        && line.fence_context != FenceKind::Other
        && table_candidate_text(line.text).is_some_and(is_table_header_line)
    {
        return TableHoldbackState::PendingHeader {
            header_start: line.source_start,
        };
    }
    TableHoldbackState::None
}
