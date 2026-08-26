//! 带 URL 感知启发式的单词换行。
//!
//! TUI 渲染的文本经常包含 URL——命令输出、markdown、agent 消息、工具调用结果。
//! 标准 `textwrap` 连字符换行将 `/` 和 `-` 视为分割点，这会将 URL 跨行断开
//! 并在终端模拟器中使其不可点击。
//!
//! 本模块提供两条换行路径：
//!
//! - **标准**（`word_wrap_line`、`word_wrap_lines`）：委托给 `textwrap`，
//!   调用者的选项不变。用于内容已知为纯散文的情况。
//! - **自适应**（`adaptive_wrap_line`、`adaptive_wrap_lines`）：检查行中是否有类似 URL 的标记；
//!   如果找到，换行保持 URL 标记完整。混合 URL/散文行仍在单词边界处换行普通散文，
//!   仅当非 URL 标记本身比可用行宽更宽时才分割非 URL 标记。
//!
//! *可能*遇到 URL 的调用者应使用 `adaptive_*` 函数。肯定不会遇到（代码块、纯数字输出）的调用者
//! 可以使用标准路径以获得速度。
//!
//! URL 检测是启发式的——见 [`text_contains_url_like`] 了解规则。假阳性抑制该行的连字符换行；
//! 假阴性让 URL 被分割。启发式故意保守：像 `src/main.rs` 这样的文件路径不匹配。

use ratatui::text::Line;
use ratatui::text::Span;
use std::borrow::Cow;
use std::ops::Range;
use textwrap::Options;
use textwrap::WordSeparator;
use textwrap::core::Word;
use textwrap::core::display_width;

use crate::tui_core::render::line_utils::push_owned_lines;

/// 返回每个换行行的字节范围，包括尾部空白和 +1 哨兵字节。
/// 用于文本区域光标位置逻辑。
pub(crate) fn wrap_ranges<'a, O>(text: &str, width_or_options: O) -> Vec<Range<usize>>
where
    O: Into<Options<'a>>,
{
    let opts = width_or_options.into();
    let mut lines: Vec<Range<usize>> = Vec::new();
    let mut cursor = 0usize;
    for (line_index, line) in textwrap::wrap(text, &opts).iter().enumerate() {
        match line {
            std::borrow::Cow::Borrowed(slice) => {
                let range = borrowed_slice_range(text, slice).unwrap_or_else(|| {
                    let synthetic_prefix = if line_index == 0 {
                        opts.initial_indent
                    } else {
                        opts.subsequent_indent
                    };
                    map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix)
                });
                let start = range.start;
                let end = range.end;
                let trailing_spaces = text[end..].chars().take_while(|c| *c == ' ').count();
                lines.push(start..end + trailing_spaces + 1);
                cursor = end + trailing_spaces;
            }
            std::borrow::Cow::Owned(slice) => {
                let synthetic_prefix = if line_index == 0 {
                    opts.initial_indent
                } else {
                    opts.subsequent_indent
                };
                let mapped = map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix);
                let trailing_spaces = text[mapped.end..].chars().take_while(|c| *c == ' ').count();
                lines.push(mapped.start..mapped.end + trailing_spaces + 1);
                cursor = mapped.end + trailing_spaces;
            }
        }
    }
    lines
}

/// 与 `wrap_ranges` 类似，但返回不带尾部空白和哨兵额外字节的范围。
/// 适用于不应保留尾部空格的一般换行。
pub(crate) fn wrap_ranges_trim<'a, O>(text: &str, width_or_options: O) -> Vec<Range<usize>>
where
    O: Into<Options<'a>>,
{
    let opts = width_or_options.into();
    let mut lines: Vec<Range<usize>> = Vec::new();
    let mut cursor = 0usize;
    for (line_index, line) in textwrap::wrap(text, &opts).iter().enumerate() {
        match line {
            std::borrow::Cow::Borrowed(slice) => {
                let range = borrowed_slice_range(text, slice).unwrap_or_else(|| {
                    let synthetic_prefix = if line_index == 0 {
                        opts.initial_indent
                    } else {
                        opts.subsequent_indent
                    };
                    map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix)
                });
                cursor = range.end;
                lines.push(range);
            }
            std::borrow::Cow::Owned(slice) => {
                let synthetic_prefix = if line_index == 0 {
                    opts.initial_indent
                } else {
                    opts.subsequent_indent
                };
                let mapped = map_owned_wrapped_line_to_range(text, cursor, slice, synthetic_prefix);
                lines.push(mapped.clone());
                cursor = mapped.end;
            }
        }
    }
    lines
}

fn borrowed_slice_range(text: &str, slice: &str) -> Option<Range<usize>> {
    let text_start = text.as_ptr() as usize;
    let text_end = text_start.checked_add(text.len())?;
    let slice_start = slice.as_ptr() as usize;
    let slice_end = slice_start.checked_add(slice.len())?;

    if slice_start < text_start || slice_end > text_end {
        return None;
    }

    Some((slice_start - text_start)..(slice_end - text_start))
}

/// 将拥有的（实例化的）换行行映射回 `text` 中的字节范围。
///
/// `textwrap` 在插入连字符惩罚字符（通常是 `-`）时返回 `Cow::Owned`，
/// 该字符在源中不存在。此函数逐个字符遍历拥有的字符串与源，
/// 跳过尾部惩罚字符，并返回从 `cursor` 开始的相应源字节范围。
fn map_owned_wrapped_line_to_range(
    text: &str,
    cursor: usize,
    wrapped: &str,
    synthetic_prefix: &str,
) -> Range<usize> {
    let wrapped = if synthetic_prefix.is_empty() {
        wrapped
    } else {
        wrapped.strip_prefix(synthetic_prefix).unwrap_or(wrapped)
    };

    let mut start = cursor;
    while start < text.len() && !wrapped.starts_with(' ') {
        let Some(ch) = text[start..].chars().next() else {
            break;
        };
        if ch != ' ' {
            break;
        }
        start += ch.len_utf8();
    }

    let mut end = start;
    let mut saw_source_char = false;
    let mut chars = wrapped.chars().peekable();
    while let Some(ch) = chars.next() {
        if end < text.len() {
            let Some(src) = text[end..].chars().next() else {
                unreachable!("checked end < text.len()");
            };
            if ch == src {
                end += src.len_utf8();
                saw_source_char = true;
                continue;
            }
        }

        // textwrap 在插入惩罚字符时会物化（实例化）拥有的行。
        // 默认惩罚字符是尾部的 '-'；它不对应源文本中的字节，
        // 因此我们跳过它，同时确保范围仍对应源文本中的字节。
        if ch == '-' && chars.peek().is_none() {
            continue;
        }

        // textwrap 在拥有输出中可能合成一些非源文本字符
        // （例如非空格缩进前缀）。继续前进，映射我们确定能匹配的源字节，
        // 而不是让应用崩溃。
        if !saw_source_char {
            continue;
        }

        tracing::warn!(
            wrapped = %wrapped,
            cursor,
            end,
            "wrap_ranges: could not fully map owned line; returning partial source range"
        );
        break;
    }

    start..end
}

/// 如果 `line` 中任何空白分隔的标记看起来像 URL，则返回 `true`。
///
/// 连接所有 span 内容并委托给 [`text_contains_url_like`]。
pub(crate) fn line_contains_url_like(line: &Line<'_>) -> bool {
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    text_contains_url_like(&text)
}

/// 如果 `line` 包含 URL 类标记和至少一个实质性的非 URL 标记，则返回 `true`。
///
/// 装饰性标记令牌（例如列表前缀 `-`、`1.`、`|`、`│`）在此检查的非 URL 侧被忽略。
pub(crate) fn line_has_mixed_url_and_non_url_tokens(line: &Line<'_>) -> bool {
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    text_has_mixed_url_and_non_url_tokens(&text)
}

/// 如果 `text` 中任何空白分隔的标记看起来像 URL，则返回 `true`。
///
/// 可识别的模式：
/// - 带 scheme 的绝对 URL（`https://…`、`ftp://…`、自定义 `myapp://…`）。
/// - 裸域名 URL（`example.com/path`、`www.example.com`、`localhost:3000/api`）。
/// - 带路径的 IPv4 主机（`192.168.1.1:8080/health`）。
///
/// 检查前会剥离两侧的标点（`()[]{}< >,.;:!'"`）。看起来像文件路径的标记
/// （`src/main.rs`、`foo/bar`）会被有意拒绝——主机部分必须是合法的域名
/// （带有可识别的 TLD）、IPv4 地址或 `localhost`。
pub(crate) fn text_contains_url_like(text: &str) -> bool {
    text.split_ascii_whitespace().any(is_url_like_token)
}

/// 如果 `text` 至少包含一个 URL 类标记和至少一个实质性的非 URL 标记，
/// 则返回 `true`。
// ── URL 检测辅助（外移子模块） ──
mod url_detection;
use url_detection::*;
// ── 混合 URL/文本换行（外移子模块） ──
mod mixed_url;
use mixed_url::*;
// ── 行输入抽象（外移子模块） ──
mod line_input;
use line_input::*;

/// 重新配置换行选项，使 URL 标记永远不会被分割。
///
/// 设置 `AsciiSpace` 单词分隔（因此 URL 中的 `/` 和 `-` 不被视为断点），
/// 禁用 `break_words`，并防止每单词连字符。混合 URL/散文行使用专用包装器，
/// 因此普通散文仍可以围绕保留的 URL 标记正确换行。
pub(crate) fn url_preserving_wrap_options<'a>(opts: RtOptions<'a>) -> RtOptions<'a> {
    opts.word_separator(textwrap::WordSeparator::AsciiSpace)
        .word_splitter(textwrap::WordSplitter::NoHyphenation)
        .break_words(/*break_words*/ false)
}

/// 包装单个 ratatui `Line`，当行包含 URL 标记时自动切换到
/// URL 保留选项。
///
/// 当未检测到 URL 时，换行行为与 [`word_wrap_line`] 相同。
/// 仅 URL 行使用 [`url_preserving_wrap_options`]，因此终端链接检测保持看到一个完整的标记。
/// 混合 URL/散文行使用标记感知包装器，因此普通散文仍作为完整单词移动，
/// 而真正过长的非 URL 标记在需要时仍可以分割。
#[must_use]
pub(crate) fn adaptive_wrap_line<'a>(line: &'a Line<'a>, base: RtOptions<'a>) -> Vec<Line<'a>> {
    let (flat, span_bounds) = flatten_line(line);
    let mut saw_url = false;
    let mut saw_non_url = false;

    for token in flat.split_ascii_whitespace() {
        if is_url_like_token(token) {
            saw_url = true;
        } else if is_substantive_non_url_token(token) {
            saw_non_url = true;
        }

        if saw_url && saw_non_url {
            break;
        }
    }

    if !saw_url {
        word_wrap_flattened_line(line, &flat, &span_bounds, base)
    } else if saw_non_url {
        mixed_url_wrap_line(line, &flat, &span_bounds, base)
    } else {
        word_wrap_flattened_line(line, &flat, &span_bounds, url_preserving_wrap_options(base))
    }
}

/// 使用 URL 感知启发式换行多条输入行，对第一行应用 `initial_indent`，
/// 对其余行应用 `subsequent_indent`。每一行都会独立检查 URL；某一行的
/// URL 检测结果不会影响其他行的换行。
///
/// 这是 [`adaptive_wrap_line`] 的多行版本，也是大多数历史单元格渲染的
/// 主要换行入口。
#[allow(private_bounds)]
pub(crate) fn adaptive_wrap_lines<'a, I, L>(
    lines: I,
    width_or_options: RtOptions<'a>,
) -> Vec<Line<'static>>
where
    I: IntoIterator<Item = L>,
    L: IntoLineInput<'a>,
{
    let base_opts = width_or_options;
    let mut out: Vec<Line<'static>> = Vec::new();

    for (idx, line) in lines.into_iter().enumerate() {
        let line_input = line.into_line_input();
        let opts = if idx == 0 {
            base_opts.clone()
        } else {
            base_opts
                .clone()
                .initial_indent(base_opts.subsequent_indent.clone())
        };

        let wrapped = adaptive_wrap_line(line_input.as_ref(), opts);
        push_owned_lines(&wrapped, &mut out);
    }

    out
}

#[derive(Debug, Clone)]
pub struct RtOptions<'a> {
    /// 文本被换行时的列宽。
    pub width: usize,
    /// 用于断行的行尾符。
    pub line_ending: textwrap::LineEnding,
    /// 输出第一行使用的缩进。参见 [`Options::initial_indent`] 方法。
    pub initial_indent: Line<'a>,
    /// 输出后续行使用的缩进。参见 [`Options::subsequent_indent`] 方法。
    pub subsequent_indent: Line<'a>,
    /// 允许在长单词无法放入一行时将其拆分。
    /// 当设为 `false` 时，某些行可能会长于
    /// `self.width`。参见 [`Options::break_words`] 方法。
    pub break_words: bool,
    /// 使用的换行算法，详见 [`WrapAlgorithm`] trait 的实现。
    pub wrap_algorithm: textwrap::WrapAlgorithm,
    /// 使用的断行算法，参见 [`WordSeparator`] trait 以了解概览和可能的实现。
    pub word_separator: textwrap::WordSeparator,
    /// 拆词的方法。可用于禁止在连字符处拆词，也可用于实现
    /// 语言感知的机器连字符处理。
    pub word_splitter: textwrap::WordSplitter,
}
impl From<usize> for RtOptions<'_> {
    fn from(width: usize) -> Self {
        RtOptions::new(width)
    }
}

#[allow(dead_code)]
impl<'a> RtOptions<'a> {
    pub fn new(width: usize) -> Self {
        Self {
            width,
            line_ending: textwrap::LineEnding::LF,
            initial_indent: Line::default(),
            subsequent_indent: Line::default(),
            break_words: true,
            word_separator: textwrap::WordSeparator::new(),
            wrap_algorithm: textwrap::WrapAlgorithm::FirstFit,
            word_splitter: textwrap::WordSplitter::HyphenSplitter,
        }
    }

    pub fn width(self, width: usize) -> Self {
        RtOptions { width, ..self }
    }

    pub fn initial_indent(self, initial_indent: Line<'a>) -> Self {
        RtOptions {
            initial_indent,
            ..self
        }
    }

    pub fn subsequent_indent(self, subsequent_indent: Line<'a>) -> Self {
        RtOptions {
            subsequent_indent,
            ..self
        }
    }

    pub fn break_words(self, break_words: bool) -> Self {
        RtOptions {
            break_words,
            ..self
        }
    }

    pub fn word_separator(self, word_separator: textwrap::WordSeparator) -> RtOptions<'a> {
        RtOptions {
            word_separator,
            ..self
        }
    }

    pub fn wrap_algorithm(self, wrap_algorithm: textwrap::WrapAlgorithm) -> RtOptions<'a> {
        RtOptions {
            wrap_algorithm,
            ..self
        }
    }

    pub fn word_splitter(self, word_splitter: textwrap::WordSplitter) -> RtOptions<'a> {
        RtOptions {
            word_splitter,
            ..self
        }
    }
}

#[must_use]
pub(crate) fn word_wrap_line<'a, O>(line: &'a Line<'a>, width_or_options: O) -> Vec<Line<'a>>
where
    O: Into<RtOptions<'a>>,
{
    let (flat, span_bounds) = flatten_line(line);
    word_wrap_flattened_line(line, &flat, &span_bounds, width_or_options.into())
}

fn word_wrap_flattened_line<'a>(
    line: &'a Line<'a>,
    flat: &str,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    rt_opts: RtOptions<'a>,
) -> Vec<Line<'a>> {
    let opts = Options::new(rt_opts.width)
        .line_ending(rt_opts.line_ending)
        .break_words(rt_opts.break_words)
        .wrap_algorithm(rt_opts.wrap_algorithm)
        .word_separator(rt_opts.word_separator)
        .word_splitter(rt_opts.word_splitter);

    let mut out: Vec<Line<'a>> = Vec::new();

    // 由于首行缩进，使用减小的宽度计算第一行的范围。
    let initial_width_available = opts
        .width
        .saturating_sub(rt_opts.initial_indent.width())
        .max(1);
    let initial_wrapped = wrap_ranges_trim(flat, opts.clone().width(initial_width_available));
    let Some(first_line_range) = initial_wrapped.first() else {
        return vec![rt_opts.initial_indent.clone()];
    };

    // 使用首行缩进构建第一条换行行。
    let mut first_line = rt_opts.initial_indent.clone().style(line.style);
    {
        let sliced = slice_line_spans(line, span_bounds, first_line_range);
        let mut spans = first_line.spans;
        spans.append(
            &mut sliced
                .spans
                .into_iter()
                .map(|s| s.patch_style(line.style))
                .collect(),
        );
        first_line.spans = spans;
        out.push(first_line);
    }

    // 使用后续行缩进的宽度换行剩余部分，并映射回原始索引。
    let base = first_line_range.end;
    let skip_leading_spaces = flat[base..].chars().take_while(|c| *c == ' ').count();
    let base = base + skip_leading_spaces;
    let subsequent_width_available = opts
        .width
        .saturating_sub(rt_opts.subsequent_indent.width())
        .max(1);
    let remaining_wrapped = wrap_ranges_trim(&flat[base..], opts.width(subsequent_width_available));
    for r in &remaining_wrapped {
        if r.is_empty() {
            continue;
        }
        let mut subsequent_line = rt_opts.subsequent_indent.clone().style(line.style);
        let offset_range = (r.start + base)..(r.end + base);
        let sliced = slice_line_spans(line, span_bounds, &offset_range);
        let mut spans = subsequent_line.spans;
        spans.append(
            &mut sliced
                .spans
                .into_iter()
                .map(|s| s.patch_style(line.style))
                .collect(),
        );
        subsequent_line.spans = spans;
        out.push(subsequent_line);
    }

    out
}

fn flatten_line(line: &Line<'_>) -> (String, Vec<(Range<usize>, ratatui::style::Style)>) {
    let mut flat = String::new();
    let mut span_bounds = Vec::new();
    let mut acc = 0usize;
    for span in &line.spans {
        let text = span.content.as_ref();
        let start = acc;
        flat.push_str(text);
        acc += text.len();
        span_bounds.push((start..acc, span.style));
    }
    (flat, span_bounds)
}

pub(crate) fn word_wrap_lines<'a, I, O, L>(lines: I, width_or_options: O) -> Vec<Line<'static>>
where
    I: IntoIterator<Item = L>,
    L: IntoLineInput<'a>,
    O: Into<RtOptions<'a>>,
{
    let base_opts: RtOptions<'a> = width_or_options.into();
    let mut out: Vec<Line<'static>> = Vec::new();

    for (idx, line) in lines.into_iter().enumerate() {
        let line_input = line.into_line_input();
        let opts = if idx == 0 {
            base_opts.clone()
        } else {
            let mut o = base_opts.clone();
            let sub = o.subsequent_indent.clone();
            o = o.initial_indent(sub);
            o
        };
        let wrapped = word_wrap_line(line_input.as_ref(), opts);
        push_owned_lines(&wrapped, &mut out);
    }

    out
}

fn slice_line_spans<'a>(
    original: &'a Line<'a>,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    range: &Range<usize>,
) -> Line<'a> {
    let start_byte = range.start;
    let end_byte = range.end;
    let mut acc: Vec<Span<'a>> = Vec::new();
    for (i, (range, style)) in span_bounds.iter().enumerate() {
        let s = range.start;
        let e = range.end;
        if e <= start_byte {
            continue;
        }
        if s >= end_byte {
            break;
        }
        let seg_start = start_byte.max(s);
        let seg_end = end_byte.min(e);
        if seg_end > seg_start {
            let local_start = seg_start - s;
            let local_end = seg_end - s;
            let content = original.spans[i].content.as_ref();
            let slice = &content[local_start..local_end];
            acc.push(Span {
                style: *style,
                content: std::borrow::Cow::Borrowed(slice),
            });
        }
        if e >= end_byte {
            break;
        }
    }
    Line {
        style: original.style,
        alignment: original.alignment,
        spans: acc,
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
