//! TUI transcript 的底层 markdown 事件渲染器。
//!
//! 本模块消费 `pulldown-cmark` 事件并输出带样式的 `ratatui`
//! 行，包括表格布局、宽度感知折行和本地文件链接显示。
//! 它是 `markdown.rs` 中更高级辅助函数使用的最终渲染阶段。
//!
//! 本渲染器有意区别对待本地文件链接与普通网页链接。对于
//! 本地路径，显示文本来自目标而非 markdown 标签，因此
//! transcript 显示真实文件目标（包括归一化的位置后缀），并可将绝对路径
//! 相对于已知工作目录缩短。
//!
//! ## 表格渲染管线
//!
//! 当解析器发出 `Tag::Table` .. `TagEnd::Table` 时，writer
//! 将表头和表体行累积到 `TableState`，再交给执行以下管线的
//! `render_table_lines`：
//!
//! 1. **过滤溢出行** —— 启发式提取 pulldown-cmark 宽松解析
//!    产生的行。
//! 2. **归一化列数** —— 填充或截断，使每行与
//!    对齐计数一致。
//! 3. **计算列宽** —— 按内容感知的优先级分配宽度并迭代收缩。
//! 4. **选择呈现方式** —— 在值仍然可扫描时渲染主题强调色的行分隔列，
//!    否则把表体行转置为以柔和分隔线分隔的
//!    键/值记录。
//! 5. **追加溢出** —— 提取的溢出行在表格之后
//!    以纯文本渲染。
//!
//! ## 宽度分配
//!
//! 列被分类为 Narrative（长散文）、TokenHeavy（路径、URL、
//! 或哈希）或 Compact（计数、状态标签等短值）。
//! Token 密集列先于叙事列让出多余宽度，这样过大的路径不会
//! 挤压可读的散文；紧凑值最后保留。当紧凑值分裂为
//! 不可用的短块、过度膨胀的单元格形成横跨足够表体行的
//! 高瘦窄条，或连 3 字符宽的列都无法容纳时，表体行将渲染为
//! 键/值记录。

use crate::tui_core::markdown_text_merge::DecodedTextMerge;
use crate::tui_core::render::highlight::foreground_style_for_scopes;
use crate::tui_core::render::highlight::highlight_code_to_lines;
use crate::tui_core::render::line_utils::line_to_static;
use crate::tui_core::style::table_separator_style;
use crate::tui_core::terminal_hyperlinks::HyperlinkLine;
use crate::tui_core::terminal_hyperlinks::annotate_web_urls_in_line;
use crate::tui_core::terminal_hyperlinks::remap_wrapped_line;
use crate::tui_core::terminal_hyperlinks::visible_lines;
use crate::tui_core::terminal_hyperlinks::web_destination;
use crate::tui_core::wrapping::RtOptions;
use crate::tui_core::wrapping::adaptive_wrap_line;
use crate::tui_core::wrapping::word_wrap_line;
use crate::utils_string::normalize_markdown_hash_location_suffix;
use dirs::home_dir;
use pulldown_cmark::Alignment;
use pulldown_cmark::CodeBlockKind;
use pulldown_cmark::CowStr;
use pulldown_cmark::Event;
use pulldown_cmark::HeadingLevel;
use pulldown_cmark::Options;
use pulldown_cmark::Parser;
use pulldown_cmark::Tag;
use pulldown_cmark::TagEnd;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use regex_lite::Regex;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;
use url::Url;

mod streaming;
mod table_key_value;

pub(crate) use streaming::StreamingMarkdownRender;
pub(crate) use streaming::render_streaming_markdown_lines_with_width_and_cwd;

const TABLE_COLUMN_GAP: usize = 2;
const TABLE_CELL_PADDING: usize = 1;
const TABLE_HEADER_SEPARATOR_CHAR: char = '━';
const TABLE_BODY_SEPARATOR_CHAR: char = '─';

// ── Writer 核心状态机（外移子模块） ──
mod writer;
use writer::*;

// ── Markdown 渲染子系统（外移子模块） ──
mod heading_palette;
mod link_state;
mod local_link;
mod table;
pub(crate) use link_state::*;
use local_link::*;
use table::*;

pub fn render_markdown_text(input: &str) -> Text<'static> {
    render_markdown_text_with_width(input, /*width*/ None)
}

/// 在已知终端宽度约束下渲染 markdown。
///
/// 渲染器在值仍可扫描时保留列式表格结构，
/// 当表体行无法可读地容纳时回退为键/值记录。传 `None` 保持
/// 固有行宽，并在 markdown writer 中禁用宽度驱动的折行。本地文件链接
/// 相对于当前进程工作目录渲染。
pub(crate) fn render_markdown_text_with_width(input: &str, width: Option<usize>) -> Text<'static> {
    let cwd = std::env::current_dir().ok();
    render_markdown_text_with_width_and_cwd(input, width, cwd.as_deref())
}

/// 使用显式工作目录渲染 markdown，用于本地文件链接。
///
/// `cwd` 参数控制本地绝对目标在显示前如何缩短。传入
/// 会话 cwd 可使完整渲染、历史单元格和流式增量即使在
/// 离开进程 cwd 渲染时也保持视觉对齐。
pub(crate) fn render_markdown_text_with_width_and_cwd(
    input: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
) -> Text<'static> {
    Text::from(visible_lines(render_markdown_lines_with_width_and_cwd(
        input, width, cwd,
    )))
}

pub(crate) fn render_markdown_lines_with_width_and_cwd(
    input: &str,
    width: Option<usize>,
    cwd: Option<&Path>,
) -> Vec<HyperlinkLine> {
    render_markdown_lines_with_width_cwd_and_hidden_link_destinations(
        input,
        width,
        cwd,
        &never_hide_link_destination,
    )
}

#[cfg(test)]
mod markdown_render_tests {
    include!("markdown_render_tests.rs");
}

#[cfg(test)]
mod tests;
