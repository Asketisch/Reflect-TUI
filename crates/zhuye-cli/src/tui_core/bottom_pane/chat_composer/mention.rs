//! Mention 令牌边界检测。从 chat_composer.rs 提取。
//!
//! 提供 `@name` 和 `$name` mention 的文本边界判断，用于在输入缓冲中定位 mention 令牌范围、
//! 执行重绑定（rebinding）以及避免将邮箱地址或路径片段误识别为 mention。

use std::ops::Range;

/// 返回某个字节字符是否在 mention 名称中合法。
///
/// mention 名称由字母、数字以及下划线和连字符组成。
pub(super) fn is_mention_name_char(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-')
}

/// 返回 `index` 之后紧邻的文本边界是否适合 `@` mention。
///
/// 下一个字符必须是空白符、字符串结尾、后跟空白符或非名称字符的句点，
/// 或者是一个不属于名称延续的字符（即不是 `.`、`/`、`\\`、字母数字、`_` 或 `-`）。
pub(super) fn ends_plaintext_at_mention(bytes: &[u8], index: usize) -> bool {
    bytes.get(index).is_none_or(|byte| {
        byte.is_ascii_whitespace()
            || *byte == b'.'
                && bytes.get(index + 1).is_none_or(|next| {
                    next.is_ascii_whitespace()
                        || !next.is_ascii_alphanumeric() && *next != b'_' && *next != b'-'
                })
            || !matches!(*byte, b'.' | b'/' | b'\\')
                && !byte.is_ascii_alphanumeric()
                && *byte != b'_'
                && *byte != b'-'
    })
}

/// 返回 `index` 之后紧邻的文本边界是否适合 `$` mention。
///
/// `$` mention 的限制更宽松：下一个字符只要不是合法的名称字符即可。
pub(super) fn ends_plaintext_at_dollar_mention(bytes: &[u8], index: usize) -> bool {
    bytes
        .get(index)
        .is_none_or(|byte| !is_mention_name_char(*byte))
}

/// 返回 `index` 之前紧邻的文本边界是否适合作为 mention 的起点。
///
/// 该位置必须位于文本开头，或者其前一个字符是空白符，
/// 或者不是合法的 mention 名称字符。
fn starts_plaintext_at_mention(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }

    text.get(..index)
        .and_then(|prefix| prefix.chars().next_back())
        .is_some_and(|ch| ch.is_whitespace() || !is_mention_name_char_char(ch))
}

/// 返回某个字符是否在 mention 名称中合法（char 版本）。
fn is_mention_name_char_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')
}

/// 从 `from` 开始，在遵守 mention 边界的前提下查找 `token` 在 `text` 中的下一次出现。
///
/// 对于 `@` mention，通过 `starts_plaintext_at_mention` 和 `ends_plaintext_at_mention`
/// 检查首尾边界，确保令牌不是邮箱地址或路径中嵌入的子串。
/// 对于 `$` mention，则使用更简单的 `ends_plaintext_at_dollar_mention` 边界检查。
///
/// 找到时返回令牌的字节范围；不存在合法出现位置时返回 `None`。
pub(super) fn find_next_mention_token_range(
    text: &str,
    token: &str,
    from: usize,
) -> Option<Range<usize>> {
    if token.is_empty() || from >= text.len() {
        return None;
    }
    let bytes = text.as_bytes();
    let token_bytes = token.as_bytes();
    let sigil = *token_bytes.first()?;
    let mut index = from;

    while index < bytes.len() {
        if bytes[index] != sigil {
            index += 1;
            continue;
        }

        let end = index.saturating_add(token_bytes.len());
        if end > bytes.len() {
            return None;
        }
        if &bytes[index..end] != token_bytes {
            index += 1;
            continue;
        }

        // 针对恢复的 `@` mention 的修复：重绑定不得附着到邮箱地址之类的嵌入式子串，
        // 同时保留现有的 `$` mention 匹配行为。
        let starts_plaintext_mention = if sigil == b'@' {
            starts_plaintext_at_mention(text, index)
        } else {
            true
        };
        // 针对恢复的 `@` mention 的修复：对齐历史编码的尾部边界，
        // 避免 `@sample/pkg` 这类路径样式文本被重绑定为普通的 `@sample` mention。
        let ends_plaintext_mention = if sigil == b'@' {
            ends_plaintext_at_mention(bytes, end)
        } else {
            ends_plaintext_at_dollar_mention(bytes, end)
        };

        if starts_plaintext_mention && ends_plaintext_mention {
            return Some(index..end);
        }

        index = end;
    }

    None
}
