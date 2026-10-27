//! `reflect-tui traces ...` —— 列 / 看本地 LLM 调用记录(`~/.zhuye/traces/`)。
//!
//! 与 `session` 子命令正交:数据来自 `reflect-telemetry` 的 model-io JSONL
//! (每次 LLM 调用一条 `ModelIoRecord`)。`ls` 列 session 文件 + 调用次数 +
//! 最近时间;`show` 展开某 session 的逐条调用详情。
//!
//! 参照:`reflect-agent/crates/runtime/reflect-cli/src/traces.rs`。

use zhuye_rollout::index::list_sessions;
use zhuye_telemetry::{
    list_model_io_files, list_span_log_files, read_model_io_file, read_span_log_file,
    resolve_traces_dir,
};

/// `reflect-tui traces ls [-n <limit>]` —— 列出本地 LLM 调用 session(按最近
/// 修改 desc,limit 默认 20)。
pub fn ls(limit: usize) -> anyhow::Result<()> {
    let base = resolve_traces_dir(None);
    let files = list_model_io_files(&base);
    if files.is_empty() {
        println!("(no traces found under {})", base.display());
        return Ok(());
    }

    // 读 rollout session 列表,用于把 session_id 映射成可读 title。
    let sessions = list_sessions(&zhuye_rollout::path::default_base()).unwrap_or_default();

    println!(
        "{:<36}  {:<20}  {:<5}  {:<12}  model",
        "session_id", "title", "calls", "last"
    );
    for (id, path, _mtime) in files.iter().take(limit) {
        let recs = read_model_io_file(path);
        let calls = recs.len();
        let last = recs
            .last()
            .map(|r| r.completed_at.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "?".into());
        let model = recs
            .last()
            .map(|r| r.model.model_id.clone())
            .unwrap_or_default();
        let title = sessions
            .iter()
            .find(|s| s.session_id.to_string() == *id)
            .and_then(|s| s.title.clone())
            .unwrap_or_else(|| id.chars().take(8).collect());
        println!(
            "{:<36}  {:<20}  {:<5}  {:<12}  {}",
            id,
            truncate(&title, 20),
            calls,
            last,
            truncate(&model, 24),
        );
    }
    let shown = files.len().min(limit);
    if files.len() > limit {
        println!(
            "(showing {shown} of {}; pass --limit/-n to see more)",
            files.len()
        );
    }
    Ok(())
}

/// `reflect-tui traces show <id>` —— 显示某 session 的逐条 LLM 调用详情。
/// `id` 可以是完整 UUID 或前 8 字符前缀。
pub fn show(id: &str) -> anyhow::Result<()> {
    let base = resolve_traces_dir(None);
    let path = resolve_model_io_path(&base, id)?;
    let recs = read_model_io_file(&path);
    if recs.is_empty() {
        println!("(no model-io records in this session)");
        return Ok(());
    }

    println!("Session: {id}");
    println!("Path:    {}", path.display());
    println!("Calls:   {}", recs.len());
    println!();

    for (i, r) in recs.iter().enumerate() {
        println!(
            "── [{i:>3}] {} | {} | {}ms | attempt {} ──",
            r.completed_at.format("%Y-%m-%d %H:%M:%S"),
            r.query_source,
            r.duration_ms,
            r.attempt,
        );
        println!(
            "  model:    {} ({})",
            r.model.model_id,
            r.model.provider_id.as_deref().unwrap_or("?"),
        );
        println!(
            "  usage:    in={} out={} cached={} total={} cost={}",
            r.usage.input_tokens,
            r.usage.output_tokens,
            r.usage.cached_tokens,
            r.usage.total_tokens,
            r.usage
                .cost_usd
                .map(|c| format!("${c:.4}"))
                .unwrap_or_else(|| "-".into()),
        );
        let req_preview = preview_value(&r.request);
        let resp_preview = preview_value(&r.response);
        println!("  request:  {}", req_preview);
        println!("  response: {}", resp_preview);
        println!();
    }
    Ok(())
}

/// `reflect-tui traces spans [id] [--days N]` —— 展示 span 日志
/// (`~/.zhuye/traces/span-log/reflect-YYYY-MM-DD.jsonl`)。
///
/// - 不带 `id` → 展示最近 `--days` 天(默认 7)的全部 span 事件;
/// - 带 `id`(完整 UUID 或前缀)→ 只看该 session 的事件。
///
/// v1.x:submodule 补齐了失败调用的 span(`model.request.failed`)——
/// 此前 4 个失败路径全部直接返回,失败调用在 telemetry 里完全不可见。
/// 本命令是其读取侧。
pub fn spans(id: Option<&str>, days: usize) -> anyhow::Result<()> {
    let base = resolve_traces_dir(None);
    render_spans(&base, id, days);
    Ok(())
}

/// span 日志渲染核心(与 `resolve_traces_dir` 解耦,便于测试)。
fn render_spans(base: &std::path::Path, id: Option<&str>, days: usize) {
    let files = list_span_log_files(base, days);
    if files.is_empty() {
        println!("(no span logs found under {})", base.display());
        return;
    }

    let mut shown = 0usize;
    for (_day, path) in &files {
        for e in read_span_log_file(path) {
            // 带 id 时按 session 前缀过滤(完整 UUID 或前缀均可)。
            if let Some(want) = id
                && !e
                    .session_id
                    .as_deref()
                    .is_some_and(|sid| sid.starts_with(want))
            {
                continue;
            }
            shown += 1;
            println!(
                "{}  [{:?}] {}  status={}  dur={}ms  turn={}  {}",
                e.timestamp.format("%Y-%m-%d %H:%M:%S"),
                e.level,
                e.event,
                e.status
                    .map(|s| format!("{s:?}"))
                    .unwrap_or_else(|| "-".into()),
                e.duration_ms.unwrap_or(0),
                e.turn_id.as_deref().unwrap_or("-"),
                e.context.as_ref().map(preview_value).unwrap_or_default(),
            );
        }
    }
    if shown == 0 {
        match id {
            Some(want) => println!("(no span events for session '{want}')"),
            None => println!("(no span events recorded)"),
        }
    }
}

/// 把 `id`(完整 UUID 或前缀)解析为 model-io 文件路径。前缀匹配扫描
/// `list_model_io_files`,要求唯一命中。
fn resolve_model_io_path(base: &std::path::Path, id: &str) -> anyhow::Result<std::path::PathBuf> {
    let files = list_model_io_files(base);
    if let Some((_, path, _)) = files.iter().find(|(sid, _, _)| sid == id) {
        return Ok(path.clone());
    }
    let matches: Vec<_> = files
        .iter()
        .filter(|(sid, _, _)| sid.starts_with(id))
        .collect();
    match matches.len() {
        0 => anyhow::bail!("no trace session matches '{id}'"),
        1 => Ok(matches[0].1.clone()),
        n => anyhow::bail!("prefix '{id}' matches {n} sessions; give a longer prefix or full UUID"),
    }
}

/// 把 serde_json::Value 压成单行预览(截 120 字符),便于终端展示。
fn preview_value(v: &serde_json::Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_else(|_| "<unserializable>".into());
    let single = s.replace('\n', " ");
    if single.chars().count() <= 120 {
        single
    } else {
        let truncated: String = single.chars().take(120).collect();
        format!("{truncated}…")
    }
}

/// UTF-8 字符级截断(超长追加 `…`)。
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn truncate_short_and_long() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn preview_value_truncates_long() {
        let long = serde_json::json!({"text": "x".repeat(300)});
        let p = preview_value(&long);
        assert!(p.ends_with('…'));
        assert!(p.chars().count() <= 121);
    }

    #[test]
    fn preview_value_preserves_short() {
        let v = serde_json::json!({"finish_reason": "stop"});
        assert_eq!(preview_value(&v), r#"{"finish_reason":"stop"}"#);
    }

    #[test]
    fn list_model_io_files_empty_dir() {
        let tmp = tempdir().unwrap();
        let files = list_model_io_files(tmp.path());
        assert!(files.is_empty());
    }

    #[test]
    fn list_and_read_roundtrip() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let mio_dir = base.join("model-io");
        fs::create_dir_all(&mio_dir).unwrap();
        let sid = "11111111-2222-3333-4444-555555555555";
        let path = mio_dir.join(format!("model-io-sess_{sid}.jsonl"));
        let rec = format!(
            r#"{{"started_at":"2026-07-31T00:00:00Z","completed_at":"2026-07-31T00:00:01Z","duration_ms":1000,"attempt":1,"request_id":"r1","trace_id":"t1","session_id":"{sid}","query_source":"main_turn","model":{{"model_id":"glm-5.2","provider_id":"builtin","role":"main","source":"main_turn"}},"request":{{}},"response":{{"finish_reason":"stop"}},"usage":{{"input_tokens":10,"output_tokens":5,"cached_tokens":0,"cache_write_tokens":0,"total_tokens":15}}}}"#
        );
        fs::write(&path, rec).unwrap();

        let files = list_model_io_files(base);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, sid);

        let recs = read_model_io_file(&path);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].usage.total_tokens, 15);
        assert_eq!(recs[0].model.model_id, "glm-5.2");

        let resolved = resolve_model_io_path(base, "11111111").unwrap();
        assert_eq!(resolved, path);
    }

    #[test]
    fn span_logs_are_listed_and_read() {
        // v1.x:span 日志读取侧(`list_span_log_files` / `read_span_log_file`)。
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let log_dir = base.join(zhuye_telemetry::model::SPAN_LOG_SUBDIR);
        fs::create_dir_all(&log_dir).unwrap();
        let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let path = log_dir.join(format!("zhuye-{day}.jsonl"));
        // `Level` / `SpanStatus` 的 serde 形态为小写。
        let ev = r#"{"timestamp":"2026-07-31T00:00:00Z","level":"error","event":"model.request.failed","module":"zhuye_core::graph::nodes::model_call","trace_id":"t1","span_id":"s1","session_id":"sess-1","turn_id":"turn-1","status":"failed","duration_ms":1200,"context":{"error":"ALL_CREDENTIALS_EXHAUSTED"}}"#;
        fs::write(&path, format!("{ev}\n")).unwrap();

        let files = list_span_log_files(base, 7);
        assert_eq!(files.len(), 1);
        let events = read_span_log_file(&files[0].1);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "model.request.failed");
        assert_eq!(events[0].session_id.as_deref(), Some("sess-1"));
        assert_eq!(
            events[0].status,
            Some(zhuye_telemetry::SpanStatus::Failed),
            "失败调用的 span 应带 failed 状态"
        );
    }

    #[test]
    fn render_spans_filters_by_session_prefix() {
        // 带 id 时按 session 前缀过滤;命中不到的显示占位提示。
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let log_dir = base.join(zhuye_telemetry::model::SPAN_LOG_SUBDIR);
        fs::create_dir_all(&log_dir).unwrap();
        let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let path = log_dir.join(format!("zhuye-{day}.jsonl"));
        let mk = |sid: &str, event: &str| {
            format!(
                r#"{{"timestamp":"2026-07-31T00:00:00Z","level":"info","event":"{event}","module":"m","trace_id":"t1","span_id":"s1","session_id":"{sid}"}}"#
            )
        };
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                mk("sess-a", "turn.started"),
                mk("sess-b", "turn.ended")
            ),
        )
        .unwrap();

        // 前缀命中 sess-a。
        render_spans(base, Some("sess-a"), 7);
        // 无命中(前缀不存在)→ 走空结果分支,不 panic。
        render_spans(base, Some("sess-zzz"), 7);
    }
}
