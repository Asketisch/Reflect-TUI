/// 解析 `/name <rest>` 形式的首行斜杠命令。
/// 若该行以 `/` 开头且包含非空名称，则返回 `(name, rest_after_name, rest_offset)`；
/// 否则返回 `None`。
///
/// `rest_offset` 是去除前导空白后、`rest_after_name` 在原始行中的字节索引
/// （即 `line[rest_offset..] == rest_after_name`）。
pub fn parse_slash_name(line: &str) -> Option<(&str, &str, usize)> {
    let stripped = line.strip_prefix('/')?;
    let mut name_end_in_stripped = stripped.len();
    for (idx, ch) in stripped.char_indices() {
        if ch.is_whitespace() {
            name_end_in_stripped = idx;
            break;
        }
    }
    let name = &stripped[..name_end_in_stripped];
    if name.is_empty() {
        return None;
    }
    let rest_untrimmed = &stripped[name_end_in_stripped..];
    let rest = rest_untrimmed.trim_start();
    let rest_start_in_stripped = name_end_in_stripped + (rest_untrimmed.len() - rest.len());
    let rest_offset = rest_start_in_stripped + 1;
    Some((name, rest, rest_offset))
}
