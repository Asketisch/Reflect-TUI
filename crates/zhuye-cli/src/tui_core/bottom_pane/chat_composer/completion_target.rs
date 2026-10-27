//! 解析带 sigil 前缀的编辑器补全时,光标邻域内的目标解析。

use super::mention::ends_plaintext_at_dollar_mention;
use super::mention::ends_plaintext_at_mention;
use super::mention::is_mention_name_char;
use crate::tui_core::bottom_pane::textarea::TextArea;
use crate::tui_core::mention_codec::is_common_env_var;
use std::ops::Range;

/// 将某个以空白分隔的候选词收窄到 `anchor` 周围的可编辑片段。
///
/// 原子元素构成硬边界。如果 anchor 位于某个元素内部,只有当需要判断该候选词是否已绑定(bound)时,
/// 才会保留原始候选词。
fn prefixed_candidate_range(
    textarea: &TextArea,
    range: Range<usize>,
    anchor: usize,
    prefix: char,
    allow_empty: bool,
) -> Option<(Range<usize>, String)> {
    let text = textarea.text();
    let prefix_len = prefix.len_utf8();
    let raw_prefixed = text
        .get(range.clone())
        .filter(|token| token.starts_with(prefix))
        .map(|token| (range.clone(), token[prefix_len..].to_string()));
    let mut segment_start = range.start;
    let mut segment_end = range.end;
    let mut has_element_boundary = false;
    let anchor = anchor.clamp(range.start, range.end);

    for element in textarea.text_element_ranges_overlapping(range) {
        has_element_boundary = true;
        if element.end <= anchor {
            segment_start = element.end;
            continue;
        }
        if anchor <= element.start {
            segment_end = element.start;
        } else {
            segment_start = anchor;
            segment_end = anchor;
        }
        break;
    }

    if !has_element_boundary {
        return raw_prefixed;
    }

    let segmented_prefixed = text
        .get(segment_start..segment_end)
        .and_then(|token| token.strip_prefix(prefix))
        .filter(|query| allow_empty || !query.is_empty())
        .map(|query| (segment_start..segment_end, query.to_string()));
    if segmented_prefixed.is_some()
        || raw_prefixed.as_ref().is_none_or(|(range, query)| {
            prefixed_token_range_is_editable(textarea, prefix, range, query)
        })
    {
        segmented_prefixed
    } else {
        raw_prefixed
    }
}

/// 提取光标处带有 `prefix` 前缀的令牌(token),如果有的话。
///
/// 返回的字符串不包含前缀。解析会考虑同行分隔空白的左右两侧令牌,并保留它们的范围以用于跨 sigil 的仲裁。
/// 可编辑的候选词会在原子文本元素处截断,而换行从不提供亲和性(affinity)。
pub(super) fn current_prefixed_token_range(
    textarea: &TextArea,
    prefix: char,
    allow_empty: bool,
) -> Option<(Range<usize>, String)> {
    current_prefixed_token_range_with_dollar_predicate(
        textarea,
        prefix,
        allow_empty,
        dollar_query_is_completable,
    )
}

/// 提取带前缀的令牌,并使用给定的 dollar 查询谓词进行分隔符仲裁。
pub(super) fn current_prefixed_token_range_with_dollar_predicate(
    textarea: &TextArea,
    prefix: char,
    allow_empty: bool,
    dollar_query_is_completable: impl Fn(&str) -> bool,
) -> Option<(Range<usize>, String)> {
    let cursor_offset = textarea.cursor();
    let text = textarea.text();

    // 将给定的字节偏移调整到其位置处或之前最近的合法字符边界。
    let mut safe_cursor = cursor_offset.min(text.len());
    if safe_cursor < text.len() && !text.is_char_boundary(safe_cursor) {
        safe_cursor = text
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= cursor_offset)
            .last()
            .unwrap_or(0);
    }

    let before_cursor = &text[..safe_cursor];
    let after_cursor = &text[safe_cursor..];
    let is_horizontal_whitespace = |c: char| {
        c.is_whitespace()
            && !matches!(
                c,
                '\n' | '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
            )
    };

    let at_whitespace = after_cursor.chars().next().is_some_and(char::is_whitespace);
    let after_horizontal_whitespace = before_cursor
        .chars()
        .next_back()
        .is_some_and(is_horizontal_whitespace);
    let cursor_starts_token = after_horizontal_whitespace && !at_whitespace;
    let next_non_separator = after_cursor.chars().find(|c| !is_horizontal_whitespace(*c));
    let separator_precedes_token = next_non_separator.is_some_and(|c| !c.is_whitespace());
    let separator_precedes_completion = next_non_separator.is_some_and(|c| matches!(c, '$' | '@'));
    let at_separator = (at_whitespace || after_horizontal_whitespace) && separator_precedes_token;

    let end_left = if at_separator {
        before_cursor
            .trim_end_matches(is_horizontal_whitespace)
            .len()
    } else {
        let end_left_rel = after_cursor
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(after_cursor.len());
        safe_cursor + end_left_rel
    };
    let start_left = text[..end_left]
        .char_indices()
        .rfind(|(_, c)| c.is_whitespace())
        .map(|(idx, c)| idx + c.len_utf8())
        .unwrap_or(0);

    let ws_len_right: usize = after_cursor
        .chars()
        .take_while(|c| is_horizontal_whitespace(*c))
        .map(char::len_utf8)
        .sum();
    let start_right = safe_cursor + ws_len_right;
    let end_right_rel = text[start_right..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(text.len() - start_right);
    let end_right = start_right + end_right_rel;
    let token_right = if start_right < end_right {
        Some(&text[start_right..end_right])
    } else {
        None
    };

    let left_prefixed = prefixed_candidate_range(
        textarea,
        start_left..end_left,
        safe_cursor,
        prefix,
        allow_empty,
    );
    let right_prefixed = prefixed_candidate_range(
        textarea,
        start_right..end_right,
        start_right,
        prefix,
        allow_empty,
    );

    if cursor_starts_token {
        let right_is_bound = token_right
            .and_then(|token| {
                let prefix = token.chars().next()?;
                matches!(prefix, '$' | '@').then_some((prefix, &token[prefix.len_utf8()..]))
            })
            .is_some_and(|(prefix, query)| {
                !prefixed_token_range_is_editable(
                    textarea,
                    prefix,
                    &(start_right..end_right),
                    query,
                )
            });
        return if right_is_bound {
            left_prefixed
        } else {
            right_prefixed
        };
    }

    if allow_empty && after_cursor.starts_with(prefix) {
        let left_fragment = &text[start_left..safe_cursor];
        if let Some(left_token) = left_fragment.strip_prefix(prefix)
            && left_token
                .as_bytes()
                .iter()
                .all(|byte| is_mention_name_char(*byte))
        {
            let left_range = start_left..safe_cursor;
            let left_is_editable =
                prefixed_token_range_is_editable(textarea, prefix, &left_range, left_token);
            if left_token.is_empty() || prefix == '$' && left_is_editable {
                return Some((left_range, left_token.to_string()));
            }
            if !left_is_editable {
                return left_prefixed.or(right_prefixed);
            }
        }
    }

    if at_separator {
        if after_horizontal_whitespace && !separator_precedes_completion {
            return right_prefixed;
        }
        if prefix == '$'
            && left_prefixed
                .as_ref()
                .is_some_and(|(_, query)| !dollar_query_is_completable(query))
            && right_prefixed
                .as_ref()
                .is_some_and(|(_, query)| dollar_query_is_completable(query))
        {
            return right_prefixed;
        }
        if prefix == '@'
            && right_prefixed.as_ref().is_some_and(|(range, query)| {
                prefixed_token_range_is_editable(textarea, prefix, range, query)
            })
        {
            return right_prefixed;
        }
        if left_prefixed.as_ref().is_some_and(|(range, token)| {
            !prefixed_token_range_is_editable(textarea, prefix, range, token)
        }) {
            return right_prefixed.or(left_prefixed);
        }
        if left_prefixed
            .as_ref()
            .is_some_and(|(_, token)| token.is_empty())
            && !allow_empty
        {
            return right_prefixed;
        }
        return left_prefixed.or(right_prefixed);
    }
    if after_cursor.starts_with(prefix) {
        let prefix_starts_token = before_cursor
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        return if prefix_starts_token {
            right_prefixed.or(left_prefixed)
        } else {
            left_prefixed
        };
    }
    left_prefixed.or(right_prefixed)
}

/// 返回令牌候选词的 sigil 和 mention 名称是否是可编辑的纯文本。
///
/// 当候选词的整个范围是原子的,或者其 mention 名称前缀是原子的且其余文本以该 sigil 的终止符开头时,
/// 该候选词被视为已绑定(bound)。
pub(super) fn prefixed_token_range_is_editable(
    textarea: &TextArea,
    prefix: char,
    range: &Range<usize>,
    token: &str,
) -> bool {
    if textarea.element_id_for_exact_range(range.clone()).is_some() {
        return false;
    }

    let name_len = token
        .as_bytes()
        .iter()
        .take_while(|byte| is_mention_name_char(**byte))
        .count();
    let mention_end = range.start + prefix.len_utf8() + name_len;
    let ends_bound_mention = if prefix == '@' {
        ends_plaintext_at_mention(textarea.text().as_bytes(), mention_end)
    } else {
        ends_plaintext_at_dollar_mention(textarea.text().as_bytes(), mention_end)
    };
    !(name_len > 0
        && mention_end < range.end
        && ends_bound_mention
        && textarea
            .element_id_for_exact_range(range.start..mention_end)
            .is_some())
}

/// 拒绝类 shell 的 dollar 语法,同时保留小写以及带插件限定名的 skill 查询。
///
/// 大写形式的环境变量常见拼写、以及以位置参数或特殊 shell 参数开头的令牌会被排除。
/// 检查使用完整的冒号限定名称,因此像 `home` 或 `home:search` 这样的 skill 不会被误认为是 `$HOME`。
pub(super) fn dollar_query_is_completable(query: &str) -> bool {
    matches!(dollar_query_kind(query), DollarQueryKind::Completable)
}

/// 对 `$` 之后的文本进行分类,用于在 shell 语法与 mention 补全之间进行仲裁。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DollarQueryKind {
    Completable,
    ShellVariable,
    DefiniteShellParameter,
    /// 以数字开头且包含非数字字符的查询,或带有后缀的 `-` 查询。
    AmbiguousShellParameter,
    Invalid,
}

/// 在查询 mention 目录之前,对 dollar 查询进行保守分类。
///
/// 纯数字形式以及精确的特殊参数形式仍被视为确定的 shell 语法。以数字开头且包含非数字的查询,
/// 或带有后缀的 `-` 查询,属于有歧义的情况,因为已加载的 mention 可能合法地使用这些拼写。
pub(super) fn dollar_query_kind(query: &str) -> DollarQueryKind {
    let name_end = query
        .as_bytes()
        .iter()
        .take_while(|byte| is_mention_name_char(**byte) || **byte == b':')
        .count();
    let name = &query[..name_end];
    let is_shell_var =
        name.bytes().all(|byte| !byte.is_ascii_lowercase()) && is_common_env_var(name);
    let is_shell_parameter = name
        .as_bytes()
        .first()
        .is_some_and(|byte| *byte == b'-' || byte.is_ascii_digit());
    let is_numeric_parameter = !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit());
    if query.is_empty() {
        DollarQueryKind::Completable
    } else if name_end == 0 {
        DollarQueryKind::Invalid
    } else if is_shell_var {
        DollarQueryKind::ShellVariable
    } else if is_numeric_parameter || matches!(name, "-" | "_") {
        DollarQueryKind::DefiniteShellParameter
    } else if is_shell_parameter {
        DollarQueryKind::AmbiguousShellParameter
    } else {
        DollarQueryKind::Completable
    }
}

#[cfg(test)]
#[path = "completion_target_tests.rs"]
mod tests;
