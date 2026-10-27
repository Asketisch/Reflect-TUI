//! TextArea 文本工具函数：可见样式、词分隔符、词拆分、tab 展开等。从 textarea.rs 抽出。

use super::*;

/// 有终端默认前景色探测结果时用 Reset；否则显式 White 兜底。
pub(super) fn visible_text_style() -> Style {
    if crate::tui_core::terminal_palette::default_fg().is_some() {
        Style::default()
    } else {
        Style::default().fg(Color::White)
    }
}

pub(super) const WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

pub(super) fn is_word_separator(ch: char) -> bool {
    WORD_SEPARATORS.contains(ch)
}

pub(super) fn split_word_pieces(run: &str) -> Vec<(usize, &str)> {
    let mut pieces = Vec::new();
    for (segment_start, segment) in run.split_word_bound_indices() {
        let mut piece_start = 0;
        let mut chars = segment.char_indices();
        let Some((_, first_char)) = chars.next() else {
            continue;
        };
        let mut in_separator = is_word_separator(first_char);

        for (idx, ch) in chars {
            let is_separator = is_word_separator(ch);
            if is_separator == in_separator {
                continue;
            }
            pieces.push((segment_start + piece_start, &segment[piece_start..idx]));
            piece_start = idx;
            in_separator = is_separator;
        }

        pieces.push((segment_start + piece_start, &segment[piece_start..]));
    }

    pieces
}

/// 将 tab 替换为用于渲染和换行的单列表示。
///
/// tab 和空格都占一个字节，因此根据此文本计算的范围仍可索引原始可编辑文本。
pub(super) fn text_for_display(text: &str) -> Cow<'_, str> {
    if text.contains('\t') {
        Cow::Owned(text.replace('\t', " "))
    } else {
        Cow::Borrowed(text)
    }
}
