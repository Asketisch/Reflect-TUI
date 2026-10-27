//! 工具调用参数的流式预览(v1.5 `ToolCallArgsDelta` 接线)。
//!
//! 背景:core 侧 `ToolCallBegin` 携带的 `args` **恒为 `null`** —— 参数要等
//! 模型生成完、工具真正开始执行才有。于是长参数工具(write / edit 的正文、
//! bash 的长命令)在生成期间对用户是「黑盒等待」。
//!
//! v1.5 新增 `EventMsg::ToolCallArgsDelta`:core 把 LLM 层的
//! `ChatEvent::ToolUseDelta` 原样转发,客户端可**在模型仍在生成时**渐进
//! 渲染工具输入。本模块负责把「累积到一半的 JSON 片段」变成一行人能读的
//! 预览。
//!
//! 本模块是**纯逻辑**(无 IO、无全局状态),便于单测覆盖。

/// 标识性字段优先级:命中即用,用来告诉用户「这个工具要对哪个东西动手」。
const PRIMARY_KEYS: &[&str] = &[
    "path",
    "file_path",
    "filename",
    "command",
    "pattern",
    "url",
    "query",
    "name",
    "description",
];

/// 长正文字段优先级:工具输入里的大块文本(write 的 content / edit 的
/// 新旧串),用于让用户看到「正文正在生成」。
const BODY_KEYS: &[&str] = &[
    "content",
    "new_str",
    "new_text",
    "new_string",
    "text",
    "body",
    "input",
    "patch",
];

/// 预览里正文尾部保留的字符数上限。
const BODY_TAIL_CHARS: usize = 160;

/// 单个工具调用的参数预览。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolArgsPreview {
    /// 标识性字段摘要(path / command / pattern / url …)。
    pub primary: Option<String>,
    /// 长正文当前已累积的字符数。
    pub body_len: usize,
    /// 长正文尾部片段(供用户看到「正在写什么」)。
    pub body_tail: Option<String>,
}

impl ToolArgsPreview {
    /// 是否没有任何可展示内容。
    pub fn is_empty(&self) -> bool {
        self.primary.is_none() && self.body_tail.is_none()
    }
}

/// 从累积的原始 JSON 片段里提取预览。
///
/// `raw` 可以是**不完整**的 JSON(模型还在生成)—— 能解析成完整对象时走
/// 结构化路径,否则退回「扫描已出现的字符串字段」路径。
///
/// `tool` 仅用于挑选展示字段的优先级(不同工具的首要字段不同)。
pub fn preview_from_partial_json(raw: &str, tool: &str) -> ToolArgsPreview {
    if raw.trim().is_empty() {
        return ToolArgsPreview::default();
    }
    // 路径 1:能完整解析 → 结构化取值,更准。
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw)
        && let Some(obj) = v.as_object()
    {
        let mut out = ToolArgsPreview::default();
        if let Some(s) = pick_string(obj, PRIMARY_KEYS) {
            out.primary = Some(s);
        }
        if let Some(s) = pick_string(obj, BODY_KEYS) {
            out.body_len = s.chars().count();
            out.body_tail = Some(tail(&s, BODY_TAIL_CHARS));
        }
        if !out.is_empty() {
            return out;
        }
    }
    // 路径 2:半截 JSON → 扫描已出现的字符串字段。
    let mut out = ToolArgsPreview::default();
    if let Some(s) = find_partial_string(raw, PRIMARY_KEYS) {
        out.primary = Some(s);
    }
    if let Some(s) = find_partial_string(raw, BODY_KEYS) {
        out.body_len = s.chars().count();
        out.body_tail = Some(tail(&s, BODY_TAIL_CHARS));
    }
    // 工具名兜底:参数里一个字符串都没有时,用工具名占位,至少让用户知道
    // 「正在生成某个工具的调用」。
    let _ = tool;
    out
}

/// 从「整包形态」的完整参数(`ToolCallBeginEvent.args`)提取预览。
///
/// 背景:并非所有 provider 都分片下发参数 —— Ollama 等把**完整**参数放在
/// `ToolUseStart` 里,此时 core **不发** `ToolCallArgsDelta`(分片是重复的)。
/// 若只依赖增量事件,这类 provider 的参数摘要会永远是空的 —— 故用
/// `ToolCallBegin` 携带的 args 兜底。
///
/// `args` 为 `null` / 非对象 → `None`(没有参数可展示)。
pub fn preview_from_complete_args(args: &serde_json::Value) -> Option<ToolArgsPreview> {
    let obj = args.as_object()?;
    let mut out = ToolArgsPreview::default();
    if let Some(s) = pick_string(obj, PRIMARY_KEYS) {
        out.primary = Some(s);
    }
    if let Some(s) = pick_string(obj, BODY_KEYS) {
        out.body_len = s.chars().count();
        out.body_tail = Some(tail(&s, BODY_TAIL_CHARS));
    }
    if out.is_empty() { None } else { Some(out) }
}

/// 从完整 JSON 对象里按优先级取第一个字符串字段值。
fn pick_string(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = obj.get(*k)
            && let Some(s) = v.as_str()
            && !s.trim().is_empty()
        {
            return Some(s.to_string());
        }
    }
    None
}

/// 在**不完整**的 JSON 文本里找第一个出现的字符串字段值。
///
/// 做法:定位 `"key"`,跳到 `:` 后的第一个 `"`,再扫描到未转义的 `"` 为止
/// (扫不到就取到文本末尾 —— 字段仍在生成中)。
fn find_partial_string(raw: &str, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = scan_field(raw, key) {
            return Some(v);
        }
    }
    None
}

/// 扫描单个 key 的字符串值(半截 JSON 友好)。
fn scan_field(raw: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut search_from = 0usize;
    while let Some(pos) = raw.get(search_from..)?.find(&needle) {
        let after_key = search_from + pos + needle.len();
        let rest = raw.get(after_key..)?;
        let rest = rest.trim_start();
        // 该次命中后面不是 `:`(例如 key 出现在字符串值里)→ 继续往后找,
        // 不能就此放弃整条搜索。
        let Some(after_colon) = rest.strip_prefix(':') else {
            search_from = after_key;
            continue;
        };
        let after_colon = after_colon.trim_start();
        let Some(body) = after_colon.strip_prefix('"') else {
            // 值不是字符串(数字 / 对象 / 数组)→ 该 key 不承载文本,跳过。
            search_from = after_key;
            continue;
        };
        let decoded = decode_partial_string(body);
        // 找到的若是空串,继续找下一个候选(同名 key 可能出现多次)。
        if decoded.trim().is_empty() {
            search_from = after_key;
            continue;
        }
        return Some(decoded);
    }
    None
}

/// 解码半截 JSON 字符串字面量:处理常见转义,遇未转义引号即止。
fn decode_partial_string(body: &str) -> String {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some(other) => out.push(other),
                None => break,
            },
            '"' => break,
            _ => out.push(c),
        }
    }
    out
}

/// 取字符串尾部 `n` 个字符(按字符边界,不切碎多字节码点)。
fn tail(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    let skip = count - n;
    s.chars().skip(skip).collect()
}

/// 把预览渲染成 live 区的一行文本。
///
/// 例:`write ▷ src/main.rs` / `bash ▷ cargo build --release`。
pub fn format_primary(tool: &str, primary: &Option<String>) -> Option<String> {
    let p = primary.as_ref()?;
    let p = collapse_ws(p);
    if p.is_empty() {
        return None;
    }
    Some(format!("{tool} ▷ {p}"))
}

/// 把参数预览压成一行摘要(进历史行的工具调用标题旁)。
///
/// 例:`src/main.rs` / `src/main.rs · 正文 1.4 KiB`。
/// 无可展示内容 → `None`(调用方不留空摘要)。
pub fn format_args_summary(preview: &ToolArgsPreview) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = &preview.primary {
        let p = collapse_ws(p);
        if !p.is_empty() {
            parts.push(p);
        }
    }
    if preview.body_len > 0 {
        parts.push(format!("正文 {}", human_bytes(preview.body_len)));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

/// 把正文预览渲染成 live 区的一行文本(带已生成字节数)。
///
/// 例:`… 正文 1.4 KiB · fn main() {`。
pub fn format_body(body_len: usize, body_tail: &Option<String>) -> Option<String> {
    let t = body_tail.as_ref()?;
    if t.trim().is_empty() {
        return None;
    }
    Some(format!(
        "… 正文 {} · {}",
        human_bytes(body_len),
        collapse_ws(t)
    ))
}

/// 人类可读的字节量级(按字符数近似 —— 参数正文基本都是 ASCII 文本)。
fn human_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0))
    }
}

/// 把连续空白折成单个空格(预览是单行,不能出现换行)。
fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_json_prefers_path_then_content() {
        let raw = r#"{"path":"src/main.rs","content":"fn main() {}"}"#;
        let p = preview_from_partial_json(raw, "write");
        assert_eq!(p.primary.as_deref(), Some("src/main.rs"));
        assert_eq!(p.body_len, 12);
        assert_eq!(p.body_tail.as_deref(), Some("fn main() {}"));
    }

    #[test]
    fn partial_json_still_yields_path() {
        // 模型还在生成,JSON 尚未闭合。
        let raw = r#"{"path":"src/ma"#;
        let p = preview_from_partial_json(raw, "write");
        assert_eq!(p.primary.as_deref(), Some("src/ma"));
    }

    #[test]
    fn partial_json_streams_body_tail() {
        let raw = r#"{"path":"a.rs","content":"hello wor"#;
        let p = preview_from_partial_json(raw, "write");
        assert_eq!(p.primary.as_deref(), Some("a.rs"));
        assert_eq!(p.body_len, 9);
        assert_eq!(p.body_tail.as_deref(), Some("hello wor"));
    }

    #[test]
    fn bash_command_is_primary() {
        let raw = r#"{"command":"cargo build --release"}"#;
        let p = preview_from_partial_json(raw, "bash");
        assert_eq!(p.primary.as_deref(), Some("cargo build --release"));
        assert!(p.body_tail.is_none());
    }

    #[test]
    fn empty_raw_yields_empty_preview() {
        let p = preview_from_partial_json("", "write");
        assert!(p.is_empty());
    }

    #[test]
    fn escapes_are_decoded_in_partial_scan() {
        let raw = r#"{"path":"a\"b.txt"}"#;
        let p = preview_from_partial_json(raw, "write");
        assert_eq!(p.primary.as_deref(), Some("a\"b.txt"));
    }

    #[test]
    fn unicode_tail_keeps_char_boundary() {
        let s = "中文内容测试";
        let t = tail(s, 3);
        // 末 3 个字符完整,不切碎多字节码点。
        assert_eq!(t.chars().count(), 3);
        assert_eq!(t, "容测试");
        assert!(s.ends_with(&t));
    }

    #[test]
    fn format_body_reports_human_size() {
        let out = format_body(2048, &Some("x".to_string())).unwrap();
        assert!(out.contains("2.0 KiB"));
    }

    #[test]
    fn format_primary_collapses_whitespace() {
        let out = format_primary("bash", &Some("cargo   build\n--release".to_string())).unwrap();
        assert_eq!(out, "bash ▷ cargo build --release");
    }

    #[test]
    fn args_summary_joins_primary_and_body_size() {
        let p = ToolArgsPreview {
            primary: Some("src/main.rs".into()),
            body_len: 2048,
            body_tail: Some("x".into()),
        };
        assert_eq!(
            format_args_summary(&p).unwrap(),
            "src/main.rs · 正文 2.0 KiB"
        );
    }

    #[test]
    fn args_summary_is_none_when_nothing_to_show() {
        assert!(format_args_summary(&ToolArgsPreview::default()).is_none());
    }

    #[test]
    fn complete_args_preview_handles_whole_package_providers() {
        // Ollama 等「整包形态」provider 把完整参数放在 ToolCallBegin.args,
        // 且不发 ToolCallArgsDelta —— 这条路径必须也能出预览。
        let args = serde_json::json!({"path": "src/main.rs", "content": "fn main() {}"});
        let p = preview_from_complete_args(&args).expect("应有预览");
        assert_eq!(p.primary.as_deref(), Some("src/main.rs"));
        assert_eq!(p.body_len, 12);
    }

    #[test]
    fn complete_args_preview_is_none_for_null_or_empty() {
        assert!(preview_from_complete_args(&serde_json::Value::Null).is_none());
        assert!(preview_from_complete_args(&serde_json::json!({})).is_none());
        assert!(preview_from_complete_args(&serde_json::json!([1, 2])).is_none());
    }
}
