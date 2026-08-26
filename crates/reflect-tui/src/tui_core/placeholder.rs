//! `crate::utils_string::normalize_markdown_hash_location_suffix` 的内置替代实现。
//!
//! Reflect 用可链接的 `#fragment` 后缀渲染 markdown 标题。上游
//! `reflect_utils_string` crate 会把原始标题标签归一化为稳定的
//! 空格连接片段。我们在此复制最小化行为，使
//! 工作区无需依赖该 crate。

/// 将原始标题标签归一化为稳定的链接片段字符串，或者
/// 将位置引用后缀（`#L74C3`）转换为其显示形式
/// （`:74:3`）。
///
/// 处理两种输入形态：
///
/// 1. **标题片段**（例如 `## My Heading`、`my-long-heading`）：
///    去除前导 `#`，将空白/连字符折叠为单个空格，
///    去除非字母数字字符。
///
/// 2. **位置引用**（例如 `#L74`、`#L74C3`、`#L12C3-L14C9`）：
///    由 `HASH_LOCATION_SUFFIX_RE` 匹配；转换为 `:line[:col]`
///    形式，使用 `:` 分隔符，使文件链接渲染为 `path:74:3`。
///
/// 若结果为空则返回 `None`。
pub fn normalize_markdown_hash_location_suffix(suffix: &str) -> Option<String> {
    let trimmed = suffix.trim_start_matches('#').trim();

    // 位置引用：L<line> 或 L<line>C<col>，可选后跟
    // -L<line>C<col>。转换为使用 `:` 分隔符的 :<line>:<col> 形式。
    if trimmed
        .chars()
        .next()
        .map(|c| c == 'L' || c == 'l')
        .unwrap_or(false)
    {
        let result = convert_location_reference(trimmed);
        if result.is_some() {
            return result;
        }
    }

    // 标题标签归一化（回退）。
    if trimmed.is_empty() {
        return None;
    }

    let mut result = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_alphanumeric() {
            result.push(ch);
        } else if ch == ' ' || ch == '-' {
            result.push(' ');
        }
    }

    let collapsed = result.split_whitespace().collect::<Vec<_>>().join(" ");

    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

/// 将 `L74C3` 或 `L12C3-L14C9` 之类的位置引用转换为
/// `:74:3` 或 `:74:3-76:9`。若输入不符合
/// 预期的位置引用形态，返回 `None`。
///
/// 注意：对于区间，只有第一部分获得前导 `:` 分隔符，
/// 因此输出为 `:line:col-line:col`（而不是 `:line:col-:line:col`）。
fn convert_location_reference(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split('-').collect();
    let mut converted = String::new();

    for (idx, part) in parts.iter().enumerate() {
        if idx > 0 {
            converted.push('-');
        }

        let part = part.trim();
        if !part.starts_with('L') && !part.starts_with('l') {
            return None;
        }

        let rest = &part[1..];
        let (line_str, col_str) = if let Some(pos) = rest.find(|c: char| c == 'C' || c == 'c') {
            let (l, c) = rest.split_at(pos);
            (l, &c[1..])
        } else {
            (rest, "")
        };

        if line_str.is_empty() || !line_str.chars().all(|c: char| c.is_ascii_digit()) {
            return None;
        }

        // 只有第一部分获得前导 ':'。
        if idx == 0 {
            converted.push_str(":");
        }
        converted.push_str(line_str);

        if !col_str.is_empty() {
            if !col_str.chars().all(|c: char| c.is_ascii_digit()) {
                return None;
            }
            converted.push_str(":");
            converted.push_str(col_str);
        }
    }

    Some(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_hash_and_whitespace() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("## My Heading"),
            Some("My Heading".to_string())
        );
    }

    #[test]
    fn collapses_hyphens_to_spaces() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("my-long-heading"),
            Some("my long heading".to_string())
        );
    }

    #[test]
    fn preserves_existing_spaces() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("hello world"),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn strips_special_chars() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("hello! @world #foo"),
            Some("hello world foo".to_string())
        );
    }

    #[test]
    fn returns_none_for_empty() {
        assert_eq!(normalize_markdown_hash_location_suffix("###"), None);
        assert_eq!(normalize_markdown_hash_location_suffix("   "), None);
    }

    #[test]
    fn converts_line_only_location_reference() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("#L74"),
            Some(":74".to_string())
        );
    }

    #[test]
    fn converts_line_column_location_reference() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("#L74C3"),
            Some(":74:3".to_string())
        );
    }

    #[test]
    fn converts_location_range() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("#L12C3-L14C9"),
            Some(":12:3-14:9".to_string())
        );
    }

    #[test]
    fn converts_lowercase_l() {
        assert_eq!(
            normalize_markdown_hash_location_suffix("#l50c10"),
            Some(":50:10".to_string())
        );
    }

    #[test]
    fn lowercased_l_without_location_chars_falls_back_to_heading() {
        // "lorem" 以 'l' 开头，但不匹配 L<number> 模式，
        // 因此落入标题规范化处理。
        assert_eq!(
            normalize_markdown_hash_location_suffix("#lorem ipsum"),
            Some("lorem ipsum".to_string())
        );
    }
}
