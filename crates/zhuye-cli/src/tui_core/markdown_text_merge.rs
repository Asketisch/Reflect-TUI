//! 合并 markdown 文本事件，同时保留解析器解码后的内容与源码偏移量。

use std::iter::Peekable;
use std::ops::Range;

use pulldown_cmark::Event;

/// 合并相邻的已解析文本事件，且不从 Markdown 源码重新构造它们。
///
/// Markdown 扩展可能会在分隔字符处把视觉上连续的文本切分开。把解码后的事件内容
/// 保持在一起，可以让下游消费者识别跨越这些解析器边界的 token，同时合并后的源码
/// 区间仍然可用于依赖偏移量的渲染。
pub(crate) struct DecodedTextMerge<I: Iterator> {
    iter: Peekable<I>,
}

impl<I: Iterator> DecodedTextMerge<I> {
    pub(crate) fn new(iter: I) -> Self {
        Self {
            iter: iter.peekable(),
        }
    }
}

impl<'a, I> Iterator for DecodedTextMerge<I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    type Item = (Event<'a>, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        let (event, mut range) = self.iter.next()?;
        let Event::Text(text) = event else {
            return Some((event, range));
        };
        if !matches!(self.iter.peek(), Some((Event::Text(_), _))) {
            return Some((Event::Text(text), range));
        }

        let mut merged = text.into_string();
        while matches!(self.iter.peek(), Some((Event::Text(_), _))) {
            let Some((Event::Text(text), next_range)) = self.iter.next() else {
                break;
            };
            merged.push_str(&text);
            range.end = next_range.end;
        }
        Some((Event::Text(merged.into()), range))
    }
}
