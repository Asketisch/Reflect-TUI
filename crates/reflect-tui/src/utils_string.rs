//! `reflect_utils_string` 的 vendored 替代实现。
//!
//! 提供 `normalize_markdown_hash_location_suffix`,同时处理
//! 标题标签规范化(location hash normalization)与位置引用转换。

/// 将原始标题标签规范化成稳定的链接片段(link-fragment)字符串,
/// 或将位置引用后缀(`#L74C3`)转换为它的展示形式(`:74:3`)。
///
/// 处理两种输入形态:
///
/// 1. **标题片段**(例如 `## My Heading`、`my-long-heading`):
///    去掉开头的 `#`,把空白/连字符合并为单个空格,
///    去掉非字母数字字符。
///
/// 2. **位置引用**(例如 `#L74`、`#L74C3`、`#L12C3-L14C9`):
///    转换为 `:line[:col]` 形式,使用 `:` 作为分隔符,
///    以便文件链接渲染为 `path:74:3`。
///
/// 如果结果为空则返回 `None`。
pub fn normalize_markdown_hash_location_suffix(suffix: &str) -> Option<String> {
    let trimmed = suffix.trim_start_matches('#').trim();

    // 位置引用: L<line> 或 L<line>C<col>,可选地后跟
    // -L<line>C<col>。转换为带有 `:` 分隔符的 :<line>:<col> 形式。
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

    // 标题标签的回退规范化。
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

/// 将 `L74C3` 或 `L12C3-L14C9` 这样的位置引用转换为
/// `:74:3` 或 `:74:3-76:9`。如果输入不符合
/// 期望的位置引用形态,返回 `None`。
///
/// 注意:对于范围,只有第一部分会带前导 `:` 分隔符,
/// 因此输出形如 `:line:col-line:col`(而非 `:line:col-:line:col`)。
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

        // 仅第一部分会带前导 `:`。
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
