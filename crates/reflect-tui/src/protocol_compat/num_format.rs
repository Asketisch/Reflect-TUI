//! 感知区域设置（locale）的整数格式化。上游使用 `icu_decimal`；stub
//! 退化为使用简单的 en-US 逗号分隔符，这样 TUI 就不必引入
//! 完整的 ICU 依赖栈。

/// 使用感知区域设置的数字分隔符格式化 `i64`（例如 en-US 下
/// `12345` -> `12,345`）。
pub fn format_with_separators(n: i64) -> String {
    let mut s = n.abs().to_string();
    let bytes: Vec<char> = s.chars().collect();
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 3);
    let len = bytes.len();
    for (i, c) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    s = out.into_iter().collect();
    if n < 0 { format!("-{}", s) } else { s }
}
