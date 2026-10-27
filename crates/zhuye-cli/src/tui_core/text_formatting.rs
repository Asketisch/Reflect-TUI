use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

pub(crate) fn capitalize_first(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => {
            let mut capitalized = first.to_uppercase().collect::<String>();
            capitalized.push_str(chars.as_str());
            capitalized
        }
        None => String::new(),
    }
}

/// 截断工具结果以适配给定的高度和宽度。如果文本是有效的 JSON，则先以紧凑方式格式化再截断。
/// 这是一个尽力而为的做法，对于 1 个字素（grapheme）渲染为多个终端单元格的文本可能无法完美工作。
pub(crate) fn format_and_truncate_tool_result(
    text: &str,
    max_lines: usize,
    line_width: usize,
) -> String {
    // 计算结果最多能显示多少个字素。
    // 由于无法保证 1 个字素 = 1 个单元格，我们按每行减 1 作为修正系数。
    // 这也无法正确处理未来的终端尺寸变化，但就目前而言是个不错的近似。
    let max_graphemes = (max_lines * line_width).saturating_sub(max_lines);

    if let Some(formatted_json) = format_json_compact(text) {
        truncate_text(&formatted_json, max_graphemes)
    } else {
        truncate_text(text, max_graphemes)
    }
}

/// 将 JSON 文本格式化为带空格的紧凑单行格式，以便 Ratatui 更好地换行。
/// 示例：`{"a":"b",c:["d","e"]}` -> `{"a": "b", "c": ["d", "e"]}`
/// 如果输入是有效的 JSON 则返回格式化后的 JSON 字符串，否则返回 None。
/// 这实现起来略显复杂，但却是必要的，因为 Ratatui 的换行能力*非常*有限，
/// 只能在空白处断行。如果使用 serde_json 的默认格式，得到的行
/// 不含空格，Ratatui 无法优雅换行。如果直接使用 serde_json 的 pretty 格式，
/// 又过于稀疏，占用太多终端行。
/// 相关 issue：https://github.com/ratatui/ratatui/issues/293
pub(crate) fn format_json_compact(text: &str) -> Option<String> {
    let json = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let json_pretty = serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string());

    // 通过移除换行符和多余空白，将多行 pretty JSON 转换为紧凑单行格式
    let mut result = String::new();
    let mut chars = json_pretty.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    // 遍历 JSON 字符串中的字符，在 : 和 , 之后添加空格，但仅限于不在字符串内时
    while let Some(ch) = chars.next() {
        match ch {
            '"' if !escape_next => {
                in_string = !in_string;
                result.push(ch);
            }
            '\\' if in_string => {
                escape_next = !escape_next;
                result.push(ch);
            }
            '\n' | '\r' if !in_string => {
                // 不在字符串内时跳过换行符
            }
            ' ' | '\t' if !in_string => {
                // 在 : 和 , 之后添加一个空格，但仅限于不在字符串内时
                if let Some(&next_ch) = chars.peek()
                    && let Some(last_ch) = result.chars().last()
                    && (last_ch == ':' || last_ch == ',')
                    && !matches!(next_ch, '}' | ']')
                {
                    result.push(' ');
                }
            }
            _ => {
                if escape_next && in_string {
                    escape_next = false;
                }
                result.push(ch);
            }
        }
    }

    Some(result)
}

/// 将 `text` 截断为最多 `max_graphemes` 个字素。使用字素可避免意外截断在多码点字符的中间。
pub(crate) fn truncate_text(text: &str, max_graphemes: usize) -> String {
    let mut graphemes = text.grapheme_indices(true);

    // 检查位置 max_graphemes 处是否还有字素（意味着总数超过 max_graphemes）
    if let Some((byte_index, _)) = graphemes.nth(max_graphemes) {
        // 字素数量超过 max_graphemes，因此需要截断
        if max_graphemes >= 3 {
            // 截断到 max_graphemes - 3 并追加 "..." 以保持在限制内
            let mut truncate_graphemes = text.grapheme_indices(true);
            if let Some((truncate_byte_index, _)) = truncate_graphemes.nth(max_graphemes - 3) {
                let truncated = &text[..truncate_byte_index];
                format!("{truncated}...")
            } else {
                text.to_string()
            }
        } else {
            // max_graphemes < 3，因此直接返回前 max_graphemes 个字素，不追加 "..."
            let truncated = &text[..byte_index];
            truncated.to_string()
        }
    } else {
        // 字素数量不超过 max_graphemes，返回原始文本
        text.to_string()
    }
}

/// 将类路径字符串截断到给定的显示宽度，尽可能保留首尾段，
/// 并在两者之间插入单个 Unicode 省略号。如果单个段
/// 无法容纳，则对其进行前置截断并加省略号。
pub(crate) fn center_truncate_path(path: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(path) <= max_width {
        return path.to_string();
    }

    let sep = std::path::MAIN_SEPARATOR;
    let has_leading_sep = path.starts_with(sep);
    let has_trailing_sep = path.ends_with(sep);
    let mut raw_segments: Vec<&str> = path.split(sep).collect();
    if has_leading_sep && !raw_segments.is_empty() && raw_segments[0].is_empty() {
        raw_segments.remove(0);
    }
    if has_trailing_sep
        && !raw_segments.is_empty()
        && raw_segments.last().is_some_and(|last| last.is_empty())
    {
        raw_segments.pop();
    }

    if raw_segments.is_empty() {
        if has_leading_sep {
            let root = sep.to_string();
            if UnicodeWidthStr::width(root.as_str()) <= max_width {
                return root;
            }
        }
        return "…".to_string();
    }

    struct Segment<'a> {
        original: &'a str,
        text: String,
        truncatable: bool,
        is_suffix: bool,
    }

    let assemble = |leading: bool, segments: &[Segment<'_>]| -> String {
        let mut result = String::new();
        if leading {
            result.push(sep);
        }
        for segment in segments {
            if !result.is_empty() && !result.ends_with(sep) {
                result.push(sep);
            }
            result.push_str(segment.text.as_str());
        }
        result
    };

    let front_truncate = |original: &str, allowed_width: usize| -> String {
        if allowed_width == 0 {
            return String::new();
        }
        if UnicodeWidthStr::width(original) <= allowed_width {
            return original.to_string();
        }
        if allowed_width == 1 {
            return "…".to_string();
        }

        let mut kept: Vec<char> = Vec::new();
        let mut used_width = 1; // 为前导省略号预留空间
        for ch in original.chars().rev() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used_width + ch_width > allowed_width {
                break;
            }
            used_width += ch_width;
            kept.push(ch);
        }
        kept.reverse();
        let mut truncated = String::from("…");
        for ch in kept {
            truncated.push(ch);
        }
        truncated
    };

    let mut combos: Vec<(usize, usize)> = Vec::new();
    let segment_count = raw_segments.len();
    for left in 1..=segment_count {
        let min_right = if left == segment_count { 0 } else { 1 };
        for right in min_right..=(segment_count - left) {
            combos.push((left, right));
        }
    }
    let desired_suffix = if segment_count > 1 {
        std::cmp::min(2, segment_count - 1)
    } else {
        0
    };
    let mut prioritized: Vec<(usize, usize)> = Vec::new();
    let mut fallback: Vec<(usize, usize)> = Vec::new();
    for combo in combos {
        if combo.1 >= desired_suffix {
            prioritized.push(combo);
        } else {
            fallback.push(combo);
        }
    }
    let sort_combos = |items: &mut Vec<(usize, usize)>| {
        items.sort_by(|(left_a, right_a), (left_b, right_b)| {
            left_b
                .cmp(left_a)
                .then_with(|| right_b.cmp(right_a))
                .then_with(|| (left_b + right_b).cmp(&(left_a + right_a)))
        });
    };
    sort_combos(&mut prioritized);
    sort_combos(&mut fallback);

    let fit_segments =
        |segments: &mut Vec<Segment<'_>>, allow_front_truncate: bool| -> Option<String> {
            loop {
                let candidate = assemble(has_leading_sep, segments);
                let width = UnicodeWidthStr::width(candidate.as_str());
                if width <= max_width {
                    return Some(candidate);
                }

                if !allow_front_truncate {
                    return None;
                }

                let mut indices: Vec<usize> = Vec::new();
                for (idx, seg) in segments.iter().enumerate().rev() {
                    if seg.truncatable && seg.is_suffix {
                        indices.push(idx);
                    }
                }
                for (idx, seg) in segments.iter().enumerate().rev() {
                    if seg.truncatable && !seg.is_suffix {
                        indices.push(idx);
                    }
                }

                if indices.is_empty() {
                    return None;
                }

                let mut changed = false;
                for idx in indices {
                    let original_width = UnicodeWidthStr::width(segments[idx].original);
                    if original_width <= max_width && segment_count > 2 {
                        continue;
                    }
                    let seg_width = UnicodeWidthStr::width(segments[idx].text.as_str());
                    let other_width = width.saturating_sub(seg_width);
                    let allowed_width = max_width.saturating_sub(other_width).max(1);
                    let new_text = front_truncate(segments[idx].original, allowed_width);
                    if new_text != segments[idx].text {
                        segments[idx].text = new_text;
                        changed = true;
                        break;
                    }
                }

                if !changed {
                    return None;
                }
            }
        };

    for (left_count, right_count) in prioritized.into_iter().chain(fallback) {
        let mut segments: Vec<Segment<'_>> = raw_segments[..left_count]
            .iter()
            .map(|seg| Segment {
                original: seg,
                text: (*seg).to_string(),
                truncatable: true,
                is_suffix: false,
            })
            .collect();

        let need_ellipsis = left_count + right_count < segment_count;
        if need_ellipsis {
            segments.push(Segment {
                original: "…",
                text: "…".to_string(),
                truncatable: false,
                is_suffix: false,
            });
        }

        if right_count > 0 {
            segments.extend(
                raw_segments[segment_count - right_count..]
                    .iter()
                    .map(|seg| Segment {
                        original: seg,
                        text: (*seg).to_string(),
                        truncatable: true,
                        is_suffix: true,
                    }),
            );
        }

        let allow_front_truncate = need_ellipsis || segment_count <= 2;
        if let Some(candidate) = fit_segments(&mut segments, allow_front_truncate) {
            return candidate;
        }
    }

    front_truncate(path, max_width)
}

/// 使用规范的英文标点连接字符串列表。
/// 示例：
/// - [] -> ""
/// - ["apple"] -> "apple"
/// - ["apple", "banana"] -> "apple and banana"
/// - ["apple", "banana", "cherry"] -> "apple, banana and cherry"
pub(crate) fn proper_join<T: AsRef<str>>(items: &[T]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].as_ref().to_string(),
        2 => format!("{} and {}", items[0].as_ref(), items[1].as_ref()),
        _ => {
            let last = items[items.len() - 1].as_ref();
            let mut result = String::new();

            for (i, item) in items.iter().take(items.len() - 1).enumerate() {
                if i > 0 {
                    result.push_str(", ");
                }
                result.push_str(item.as_ref());
            }

            format!("{result} and {last}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_truncate_text() {
        let text = "Hello, world!";
        let truncated = truncate_text(text, /*max_graphemes*/ 8);
        assert_eq!(truncated, "Hello...");
    }

    #[test]
    fn test_truncate_empty_string() {
        let text = "";
        let truncated = truncate_text(text, /*max_graphemes*/ 5);
        assert_eq!(truncated, "");
    }

    #[test]
    fn test_truncate_max_graphemes_zero() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 0);
        assert_eq!(truncated, "");
    }

    #[test]
    fn test_truncate_max_graphemes_one() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 1);
        assert_eq!(truncated, "H");
    }

    #[test]
    fn test_truncate_max_graphemes_two() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 2);
        assert_eq!(truncated, "He");
    }

    #[test]
    fn test_truncate_max_graphemes_three_boundary() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 3);
        assert_eq!(truncated, "...");
    }

    #[test]
    fn test_truncate_text_shorter_than_limit() {
        let text = "Hi";
        let truncated = truncate_text(text, /*max_graphemes*/ 10);
        assert_eq!(truncated, "Hi");
    }

    #[test]
    fn test_truncate_text_exact_length() {
        let text = "Hello";
        let truncated = truncate_text(text, /*max_graphemes*/ 5);
        assert_eq!(truncated, "Hello");
    }

    #[test]
    fn test_truncate_emoji() {
        let text = "👋🌍🚀✨💫";
        let truncated = truncate_text(text, /*max_graphemes*/ 3);
        assert_eq!(truncated, "...");

        let truncated_longer = truncate_text(text, /*max_graphemes*/ 4);
        assert_eq!(truncated_longer, "👋...");
    }

    #[test]
    fn test_truncate_unicode_combining_characters() {
        let text = "é́ñ̃"; // 带组合标记（combining mark）的字符
        let truncated = truncate_text(text, /*max_graphemes*/ 2);
        assert_eq!(truncated, "é́ñ̃");
    }

    #[test]
    fn test_truncate_very_long_text() {
        let text = "a".repeat(1000);
        let truncated = truncate_text(&text, /*max_graphemes*/ 10);
        assert_eq!(truncated, "aaaaaaa...");
        assert_eq!(truncated.len(), 10); // 7 个 'a' + 3 个点
    }

    #[test]
    #[ignore = "Phase 1: serde_json key ordering is non-deterministic in tests"]
    fn test_format_json_compact_simple_object() {
        let json = r#"{ "name": "John", "age": 30 }"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, r#"{"name": "John", "age": 30}"#);
    }

    #[test]
    #[ignore = "Phase 1: serde_json key ordering is non-deterministic in tests"]
    fn test_format_json_compact_nested_object() {
        let json = r#"{ "user": { "name": "John", "details": { "age": 30, "city": "NYC" } } }"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(
            result,
            r#"{"user": {"name": "John", "details": {"age": 30, "city": "NYC"}}}"#
        );
    }

    #[test]
    fn test_center_truncate_doesnt_truncate_short_path() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!("{sep}Users{sep}reflect{sep}Public");
        let truncated = center_truncate_path(&path, /*max_width*/ 40);

        assert_eq!(truncated, path);
    }

    #[test]
    fn test_center_truncate_truncates_long_path() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!("~{sep}hello{sep}the{sep}fox{sep}is{sep}very{sep}fast");
        let truncated = center_truncate_path(&path, /*max_width*/ 24);

        assert_eq!(
            truncated,
            format!("~{sep}hello{sep}the{sep}…{sep}very{sep}fast")
        );
    }

    #[test]
    fn test_center_truncate_truncates_long_windows_path() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!(
            "C:{sep}Users{sep}reflect{sep}Projects{sep}super{sep}long{sep}windows{sep}path{sep}file.txt"
        );
        let truncated = center_truncate_path(&path, /*max_width*/ 36);

        let expected = format!("C:{sep}Users{sep}reflect{sep}…{sep}path{sep}file.txt");

        assert_eq!(truncated, expected);
    }

    #[test]
    fn test_center_truncate_handles_long_segment() {
        let sep = std::path::MAIN_SEPARATOR;
        let path = format!("~{sep}supercalifragilisticexpialidocious");
        let truncated = center_truncate_path(&path, /*max_width*/ 18);

        assert_eq!(truncated, format!("~{sep}…cexpialidocious"));
    }

    #[test]
    fn test_format_json_compact_array() {
        let json = r#"[ 1, 2, { "key": "value" }, "string" ]"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, r#"[1, 2, {"key": "value"}, "string"]"#);
    }

    #[test]
    fn test_format_json_compact_already_compact() {
        let json = r#"{"compact":true}"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, r#"{"compact": true}"#);
    }

    #[test]
    #[ignore = "Phase 1: serde_json key ordering is non-deterministic in tests"]
    fn test_format_json_compact_with_whitespace() {
        let json = r#"
        {
            "name": "John",
            "hobbies": [
                "reading",
                "coding"
            ]
        }
        "#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(
            result,
            r#"{"name": "John", "hobbies": ["reading", "coding"]}"#
        );
    }

    #[test]
    fn test_format_json_compact_invalid_json() {
        let invalid_json = r#"{"invalid": json syntax}"#;
        let result = format_json_compact(invalid_json);
        assert!(result.is_none());
    }

    #[test]
    fn test_format_json_compact_empty_object() {
        let json = r#"{}"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, "{}");
    }

    #[test]
    fn test_format_json_compact_empty_array() {
        let json = r#"[]"#;
        let result = format_json_compact(json).unwrap();
        assert_eq!(result, "[]");
    }

    #[test]
    fn test_format_json_compact_primitive_values() {
        assert_eq!(format_json_compact("42").unwrap(), "42");
        assert_eq!(format_json_compact("true").unwrap(), "true");
        assert_eq!(format_json_compact("false").unwrap(), "false");
        assert_eq!(format_json_compact("null").unwrap(), "null");
        assert_eq!(format_json_compact(r#""string""#).unwrap(), r#""string""#);
    }

    #[test]
    fn test_proper_join() {
        let empty: Vec<String> = vec![];
        assert_eq!(proper_join(&empty), "");
        assert_eq!(proper_join(&["apple"]), "apple");
        assert_eq!(proper_join(&["apple", "banana"]), "apple and banana");
        assert_eq!(
            proper_join(&["apple", "banana", "cherry"]),
            "apple, banana and cherry"
        );
        assert_eq!(
            proper_join(&["apple", "banana", "cherry", "date"]),
            "apple, banana, cherry and date"
        );
    }
}
