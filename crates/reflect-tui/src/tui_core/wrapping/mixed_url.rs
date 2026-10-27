//! 混合 URL/文本换行辅助：MixedUrlWord 类型与混合换行算法。从 wrapping/mod.rs 抽出。

use super::*;

#[derive(Clone, Debug)]
pub(super) struct MixedUrlWord {
    pub(super) range: Range<usize>,
    pub(super) is_url: bool,
}

impl MixedUrlWord {
    fn width(&self, text: &str) -> usize {
        display_width(&text[self.range.clone()])
    }
}

pub(super) fn mixed_url_wrap_line<'a>(
    line: &'a Line<'a>,
    flat: &str,
    span_bounds: &[(Range<usize>, ratatui::style::Style)],
    rt_opts: RtOptions<'a>,
) -> Vec<Line<'a>> {
    let initial_width_available = rt_opts
        .width
        .saturating_sub(rt_opts.initial_indent.width())
        .max(1);
    let subsequent_width_available = rt_opts
        .width
        .saturating_sub(rt_opts.subsequent_indent.width())
        .max(1);
    let ranges = mixed_url_wrap_ranges(flat, initial_width_available, subsequent_width_available);

    let mut out = Vec::new();
    for (idx, range) in ranges.iter().enumerate() {
        let mut wrapped_line = if idx == 0 {
            rt_opts.initial_indent.clone()
        } else {
            rt_opts.subsequent_indent.clone()
        }
        .style(line.style);
        let sliced = slice_line_spans(line, span_bounds, range);
        let mut spans = wrapped_line.spans;
        spans.extend(
            sliced
                .spans
                .into_iter()
                .map(|span| span.patch_style(line.style)),
        );
        wrapped_line.spans = spans;
        out.push(wrapped_line);
    }

    if out.is_empty() {
        vec![rt_opts.initial_indent.clone()]
    } else {
        out
    }
}

pub(super) fn mixed_url_wrap_ranges(
    text: &str,
    initial_width: usize,
    subsequent_width: usize,
) -> Vec<Range<usize>> {
    let leading_space_width = text.chars().take_while(|ch| *ch == ' ').count();
    let mut words = Vec::new();
    let mut cursor = 0usize;
    for word in WordSeparator::AsciiSpace.find_words(text) {
        let word_start = cursor;
        let word_end = word_start + word.word.len();
        let trailing_space_end = word_end + word.whitespace.len();
        if !word.word.is_empty() {
            words.push(MixedUrlWord {
                range: word_start..word_end,
                is_url: is_url_like_token(word.word),
            });
        }
        cursor = trailing_space_end;
    }

    let mut lines = Vec::new();
    let mut line_start = None;
    let mut line_end = 0usize;
    let mut line_width = 0usize;
    let mut line_limit = initial_width.max(1);

    for word in words {
        let mut pending = split_mixed_url_word(text, word, line_limit);
        let mut pending_idx = 0usize;

        while let Some(piece) = pending.get(pending_idx).cloned() {
            let empty_line_prefix_width = if line_start.is_none() && lines.is_empty() {
                leading_space_width
            } else {
                0
            };
            let empty_line_piece_limit = line_limit.saturating_sub(empty_line_prefix_width).max(1);
            // 空行上的超宽非 URL 片段先尝试再拆分。
            let mut force_place = false;
            if line_start.is_none() && !piece.is_url && piece.width(text) > empty_line_piece_limit {
                let split = split_mixed_url_word(text, piece.clone(), empty_line_piece_limit);
                // 拆分必须有净进展才 splice:textwrap `break_apart` 对「单个
                // 字符宽于可用宽度」(CJK/emoji 宽 2、limit 1)会原样返回该
                // 片段,splice 回自身 + continue 是无限循环 → 渲染线程永久
                // 挂死(混合 URL 行 + 极窄可用宽度时可复现)。
                let made_progress = split.len() > 1
                    || split
                        .first()
                        .is_some_and(|first| first.width(text) < piece.width(text));
                if made_progress {
                    pending.splice(pending_idx..=pending_idx, split);
                    continue;
                }
                // 不可再分:与 URL 片段同理按「不可拆分整体」强制放置,
                // 否则 fits=false 会对同一片段无限重试(见下方 fits 逻辑)。
                force_place = true;
            }

            let piece_width = piece.width(text);
            let inter_word_space = line_start
                .map(|_| text[line_end..piece.range.start].len())
                .unwrap_or(0);
            let fits = force_place
                || if line_start.is_none() {
                    piece.is_url
                        || empty_line_prefix_width + piece_width <= line_limit
                        || empty_line_prefix_width >= line_limit
                } else {
                    line_width + inter_word_space + piece_width <= line_limit
                };

            if fits {
                if line_start.is_none() {
                    let is_first_output_line = lines.is_empty();
                    let start = if is_first_output_line {
                        0
                    } else {
                        piece.range.start
                    };
                    line_start = Some(start);
                    line_width = if is_first_output_line {
                        leading_space_width + piece_width
                    } else {
                        piece_width
                    };
                } else {
                    line_width += inter_word_space + piece_width;
                }
                line_end = piece.range.end;
                pending_idx += 1;
                continue;
            }

            if let Some(start) = line_start.take() {
                lines.push(start..line_end);
            }
            line_end = 0;
            line_width = 0;
            line_limit = subsequent_width.max(1);
        }
    }

    if let Some(start) = line_start {
        lines.push(start..line_end);
    }

    lines
}

pub(super) fn split_mixed_url_word(
    text: &str,
    word: MixedUrlWord,
    line_limit: usize,
) -> Vec<MixedUrlWord> {
    if word.is_url || word.width(text) <= line_limit {
        return vec![word];
    }

    let source = Word::from(&text[word.range.clone()]);
    let mut offset = word.range.start;
    let mut pieces = Vec::new();
    for piece in source.break_apart(line_limit.max(1)) {
        let end = offset + piece.word.len();
        pieces.push(MixedUrlWord {
            range: offset..end,
            is_url: false,
        });
        offset = end;
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 超宽单字符窄行不死循环() {
        // 回归:CJK 首字符宽 2 > 可用宽度 1,break_apart 对单字符不可再分,
        // 旧实现把同一片段 splice 回自身 → 无限循环挂死渲染线程。
        // (此测试若回归会直接挂死 cargo test,即失败信号。)
        let text = "你好世界";
        let ranges = mixed_url_wrap_ranges(text, 1, 1);
        assert!(!ranges.is_empty());
        let joined: String = ranges
            .iter()
            .map(|r| &text[r.start..r.end])
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(joined, text, "换行不得丢失字符");
    }

    #[test]
    fn url与cjk混合窄行不死循环且不丢内容() {
        let text = "see https://example.com/aaaaaaaaaaaa 中文说明 end";
        let ranges = mixed_url_wrap_ranges(text, 4, 3);
        assert!(!ranges.is_empty());
        let joined: String = ranges
            .iter()
            .map(|r| &text[r.start..r.end])
            .collect::<Vec<_>>()
            .join("");
        assert!(joined.contains("https://example.com/aaaaaaaaaaaa"));
        assert!(joined.contains("中文"));
        assert!(joined.contains("end"));
    }
}
