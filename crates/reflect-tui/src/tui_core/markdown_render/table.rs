//! Markdown 表格渲染子系统。从 markdown_render/mod.rs 抽出。

use super::*;
use ratatui::style::Color;

pub(super) struct MarkdownStyles {
    pub(super) h1: Style,
    pub(super) h2: Style,
    pub(super) h3: Style,
    pub(super) h4: Style,
    pub(super) h5: Style,
    pub(super) h6: Style,
    pub(super) code: Style,
    pub(super) emphasis: Style,
    pub(super) strong: Style,
    pub(super) strikethrough: Style,
    pub(super) ordered_list_marker: Style,
    pub(super) unordered_list_marker: Style,
    pub(super) link: Style,
    pub(super) blockquote: Style,
    /// 水平分隔线 `---` 的样式。
    pub(super) rule: Style,
    /// 引用块前缀 `> ` 的样式，与 blockquote 内容同色系但更柔和。
    pub(super) blockquote_prefix: Style,
}

impl Default for MarkdownStyles {
    fn default() -> Self {
        Self {
            h1: Style::new().bold().underlined(),
            h2: Style::new().bold(),
            h3: Style::new().bold().italic(),
            h4: Style::new().italic(),
            h5: Style::new().italic(),
            h6: Style::new().italic(),
            code: Style::new().cyan(),
            emphasis: Style::new().italic(),
            strong: Style::new().bold(),
            strikethrough: Style::new().crossed_out(),
            ordered_list_marker: Style::new().light_blue(),
            unordered_list_marker: Style::new(),
            link: Style::new().cyan().underlined(),
            blockquote: Style::new().green(),
            // `rule`/`blockquote_prefix` 是新增字段，默认无样式以保持与原行为一致；
            // 运行时由 `for_terminal()` 注入带修饰的样式。
            rule: Style::new(),
            blockquote_prefix: Style::new(),
        }
    }
}

impl MarkdownStyles {
    /// 根据当前终端背景生成 Blue-Topaz 风格的彩色样式。
    ///
    /// 标题使用 `heading_palette::heading_styles_for_terminal()` 的彩虹渐变；
    /// 无序列表标记复用 H3 的青色；水平分隔线和引用块前缀降为 `dim()`，保持低对比度。
    /// 颜色通过 `best_color()` 降级到终端支持的最佳级别；低颜色环境下 `best_color()` 返回
    /// `Color::Reset`，`apply_fg` 跳过 `fg` 设置，标题退化为仅修饰符（bold/italic），
    /// 因此不需要再做 color level 门控。
    pub(crate) fn for_terminal() -> Self {
        let [h1, h2, h3, h4, h5, h6] = super::heading_palette::heading_styles_for_terminal();
        Self {
            h1,
            h2,
            h3,
            h4,
            h5,
            h6,
            // 无序列表标记复用 H3 颜色，但保持无修饰符
            unordered_list_marker: h3_color_only(),
            rule: Style::new().dim(),
            // 引用块前缀只用 `dim()` 修饰符（避免 `Color::Green` 不经 `best_color()` 降级
            // 导致低颜色终端下行为不一致）
            blockquote_prefix: Style::new().dim(),
            ..Self::default()
        }
    }
}

/// 返回仅含 H3 前缀色（无修饰符）的样式，用于无序列表标记。
///
/// 直接调用 `best_color` 而非复用 `h3` 样式，避免因测试期望 `Style::new()` 而带入 `bold().italic()`。
fn h3_color_only() -> Style {
    use crate::tui_core::color::is_light;
    use crate::tui_core::terminal_palette::best_color;
    use crate::tui_core::terminal_palette::default_bg;

    let bg = default_bg();
    let is_light_bg = bg.is_some_and(is_light);
    let levels = super::heading_palette::heading_rgb_levels(is_light_bg);
    match best_color(levels[2]) {
        Color::Reset => Style::new(),
        color => Style::new().fg(color),
    }
}

#[derive(Clone, Debug)]
pub(super) struct IndentContext {
    pub(super) prefix: Vec<Span<'static>>,
    pub(super) marker: Option<Vec<Span<'static>>>,
    pub(super) is_list: bool,
}

impl IndentContext {
    pub(crate) fn new(
        prefix: Vec<Span<'static>>,
        marker: Option<Vec<Span<'static>>>,
        is_list: bool,
    ) -> Self {
        Self {
            prefix,
            marker,
            is_list,
        }
    }
}

/// 正在解析的表格中单个单元格的带样式内容。
///
/// 一个单元格可以包含多行（单元格内的硬换行）以及富文本内联 span
/// （粗体、代码、链接）。`plain_text()` 投影用于列宽测量；
/// 带样式的 `lines` 用于最终渲染。
#[derive(Clone, Debug, Default)]
pub(super) struct TableCell {
    pub(super) lines: Vec<HyperlinkLine>,
}

// TableCell 的修改方法采用内联——在表格事件解析期间按 span 调用。
impl TableCell {
    #[inline]
    fn ensure_line(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(HyperlinkLine::new(Line::default()));
        }
    }

    #[inline]
    pub(crate) fn push_span(&mut self, span: Span<'static>) {
        self.ensure_line();
        if let Some(line) = self.lines.last_mut() {
            line.line.push_span(span);
        }
    }

    pub(crate) fn push_annotated(&mut self, mut appended: HyperlinkLine) {
        self.ensure_line();
        if let Some(line) = self.lines.last_mut() {
            let shift = line.width();
            line.line.spans.append(&mut appended.line.spans);
            line.hyperlinks
                .extend(appended.hyperlinks.into_iter().map(|mut link| {
                    link.columns = link.columns.start + shift..link.columns.end + shift;
                    link
                }));
        }
    }

    #[inline]
    pub(crate) fn hard_break(&mut self) {
        self.lines.push(HyperlinkLine::new(Line::default()));
    }

    pub(crate) fn plain_text(&self) -> String {
        use std::fmt::Write;
        let mut buf = String::new();
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                buf.push(' ');
            }
            for span in &line.line.spans {
                let _ = write!(buf, "{}", span.content);
            }
        }
        buf
    }
}

/// 将 pulldown-cmark 的表格事件累积为结构化表示。
///
/// `TableState` 在 `Tag::Table` 时创建，在 `TagEnd::Table` 时消费。在这两个事件之间，
/// Writer 会将单元格内容（文本、代码、html、换行）委托给 `current_cell`，
/// 在 `TagEnd::TableCell` 时刷入 `current_row`，然后在行/表头结束事件时
/// 刷入 `header`/`rows`。
#[derive(Debug)]
pub(super) struct TableBodyRow {
    pub(super) cells: Vec<TableCell>,
    pub(super) has_table_pipe_syntax: bool,
}

#[derive(Debug)]
pub(super) struct TableState {
    pub(super) alignments: Vec<Alignment>,
    pub(super) header: Option<Vec<TableCell>>,
    pub(super) rows: Vec<TableBodyRow>,
    pub(super) current_row: Option<Vec<TableCell>>,
    pub(super) current_row_has_table_pipe_syntax: bool,
    pub(super) current_cell: Option<TableCell>,
    pub(super) in_header: bool,
}

impl TableState {
    pub(crate) fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            header: None,
            rows: Vec::new(),
            current_row: None,
            current_row_has_table_pipe_syntax: false,
            current_cell: None,
            in_header: false,
        }
    }
}

/// 按折行行为拆分的表格渲染输出。
///
/// `table_lines` 是已预折行的对齐行或键值记录，
/// 但仅含表头的表格可能保留竖线回退行以走正常折行流程。
/// `spillover_lines` 是从解析器伪影中提取的散文行，
/// 应通过正常折行流程处理。
pub(super) struct RenderedTableLines {
    pub(super) table_lines: Vec<HyperlinkLine>,
    pub(super) table_lines_prewrapped: bool,
    pub(super) spillover_lines: Vec<HyperlinkLine>,
}

/// 表格列的类别划分，用于确定宽度分配优先级。
///
/// 路径和 URL 等 token 密集列允许在正文变得不可读之前先折行。
/// 计数或状态词等紧凑列则抵抗折行，使其值保持可快速扫读。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableColumnKind {
    /// 长篇散文内容（平均每格 >= 4 个单词，或平均字符宽度 >= 28）。
    Narrative,
    /// 以长 token 为主的内容，例如路径、URL 和哈希值。
    TokenHeavy,
    /// 应抵抗折行的短值，例如计数和状态标签。
    Compact,
}

/// 用于驱动宽度分配算法的每列统计信息。
///
/// 在做出任何收缩决策之前，通过对表头和正文行的单次遍历收集。
#[derive(Clone, Debug)]
pub(super) struct TableColumnMetrics {
    /// 表头和所有正文行中最宽的单元格内容（显示宽度）。
    pub(super) max_width: usize,
    /// 表头中以空白分隔的最长 token 的显示宽度。
    pub(super) header_token_width: usize,
    /// 所有正文行中以空白分隔的最长 token 的显示宽度。
    pub(super) body_token_width: usize,
    /// 由正文 token 密度和平均单元格内容推导出的类别。
    pub(super) kind: TableColumnKind,
}
