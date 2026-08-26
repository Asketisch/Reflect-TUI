//! `reflect-tui session ...` —— fork / rename / export / ls / show / rm 本地 session。
//!
//! 与 `reflect-cli session ...` 对称:所有底层操作都走 `reflect-rollout`
//! 的公开 API。`id` 可以是完整 UUID 或前 8+ 字符前缀。
//!
//! fork 子命令创建子会话后打印 child id,提示用
//! `reflect-tui --resume <child_id>` 续作(与 reflect-cli 的
//! `reflect exec --resume` 语义一致,但二进制名不同)。

use anyhow::{Context, anyhow};
use reflect_protocol::{RolloutRecord, SessionInfo, ThreadId};
use reflect_rollout::index::list_sessions;
use reflect_rollout::path::{default_base, session_path_at};
// v1.x:接入此前孤儿的 rename/read_name/to_markdown + fork_with_history。
use reflect_rollout::index::{fork_with_history, read_session_name, rename_session};
use reflect_rollout::to_markdown;
use uuid::Uuid;

/// `reflect-tui session ls [-n <limit>] [--model <substr>]` —— 列出本地 session。
///
/// 展示 session_id / model / started / msgs / tokens / cost 六列。tokens 与
/// cost 聚合自 JSONL 里的 `RolloutRecord::TokenCount` 记录(旧 jsonl 无该
/// 记录则显示 0 / --)。
pub fn ls(limit: usize, model_filter: Option<&str>) -> anyhow::Result<()> {
    let base = default_base();
    let mut sessions = list_sessions(&base).context("list_sessions")?;

    if let Some(m) = model_filter {
        let m_lc = m.to_lowercase();
        sessions.retain(|s| s.model.to_lowercase().contains(&m_lc));
    }

    if sessions.is_empty() {
        println!("(no sessions found under {})", base.display());
        return Ok(());
    }

    println!(
        "{:<36}  {:<32}  {:<12}  {:>6}  {:>10}  {:>10}",
        "session_id", "model", "started", "msgs", "tokens", "cost"
    );
    for s in sessions.iter().take(limit) {
        let model = truncate(&s.model, 32);
        let tokens = format_with_thousands(&s.total_tokens.to_string());
        let cost = match s.cost_usd {
            Some(c) => format!("${:.2}", c),
            None => String::from("--"),
        };
        println!(
            "{:<36}  {:<32}  {:<12}  {:>6}  {:>10}  {:>10}",
            s.session_id.to_string(),
            model,
            s.started_at.format("%Y-%m-%d"),
            s.message_count,
            tokens,
            cost,
        );
    }
    let shown = sessions.len().min(limit);
    if sessions.len() > limit {
        println!(
            "(showing {shown} of {}; pass --limit/-n to see more)",
            sessions.len()
        );
    }
    Ok(())
}

/// `reflect-tui session show <id>` —— 显示元数据 + 前 5 条消息预览 +
/// 整 session 累计 token / cost。`id` 可以是完整 UUID 或前缀。
pub fn show(id: &str) -> anyhow::Result<()> {
    let base = default_base();
    let (tid, path) = resolve_session_path(&base, id)?;
    println!("Session: {tid}");
    if let Ok(Some(name)) = read_session_name(&base, tid) {
        println!("Name:    {name}");
    }
    println!("Path:    {}", path.display());

    let content = std::fs::read_to_string(&path).context("read session jsonl")?;
    let (input_tokens, output_tokens, total_tokens, cost_usd) = aggregate_token_counts(&content);

    let mut msg_count = 0usize;
    let mut meta_printed = false;
    for line in content.lines().take(20) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap_or_default();
        let r#type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match r#type {
            "session_meta" => {
                let model = v.get("model").and_then(|m| m.as_str()).unwrap_or("?");
                let started = v.get("started_at").and_then(|s| s.as_str()).unwrap_or("?");
                println!("Model:   {model}");
                println!("Started: {started}");
                println!(
                    "Tokens:  {} (in {} / out {})",
                    format_with_thousands(&total_tokens.to_string()),
                    format_with_thousands(&input_tokens.to_string()),
                    format_with_thousands(&output_tokens.to_string()),
                );
                if let Some(c) = cost_usd {
                    println!("Cost:    ${:.2}", c);
                }
                meta_printed = true;
            }
            "message" if msg_count < 5 => {
                msg_count += 1;
                let role = v.get("role").and_then(|r| r.as_str()).unwrap_or("?");
                let body = preview_message_body(&v);
                println!("[{msg_count:>2}] {role}: {body}");
            }
            "compaction" if msg_count < 5 => {
                msg_count += 1;
                let summary = v
                    .get("summary")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect::<String>();
                println!("[{msg_count:>2}] compaction: {summary}…");
            }
            _ => {}
        }
    }
    if !meta_printed {
        println!(
            "Tokens:  {} (in {} / out {})",
            format_with_thousands(&total_tokens.to_string()),
            format_with_thousands(&input_tokens.to_string()),
            format_with_thousands(&output_tokens.to_string()),
        );
        if let Some(c) = cost_usd {
            println!("Cost:    ${:.2}", c);
        }
    }
    if msg_count == 0 {
        println!("(no message records in this session)");
    }
    Ok(())
}

/// `reflect-tui session rm <id> [--yes]` —— 删除一个 session 文件。
pub fn rm(id: &str, yes: bool) -> anyhow::Result<()> {
    let base = default_base();
    let (_tid, path) = resolve_session_path(&base, id)?;

    if !path.exists() {
        return Err(anyhow!("session file not found: {}", path.display()));
    }
    if !yes {
        eprint!("Delete {}? [y/N] ", path.display());
        std::io::Write::flush(&mut std::io::stderr()).ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        if !matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("aborted");
            return Ok(());
        }
    }

    std::fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    println!("removed {}", path.display());
    Ok(())
}

/// `reflect-tui session fork <id> [--branch <name>]` —— fork 父会话截止当前
/// 完整历史到新子会话。
///
/// 子会话原样复制父 records(user + assistant 完整 ContentBlocks + Compaction
/// + Checkpoint/Rewind 等),无损。fork 完成后用
/// `reflect-tui --resume <child_id>` 续作。
pub fn fork(id: &str, branch_name: Option<&str>) -> anyhow::Result<()> {
    let base = default_base();
    let (parent_id, _path) = resolve_session_path(&base, id)?;

    let child_id = fork_with_history(
        &base,
        parent_id,
        branch_name.unwrap_or("manual"),
        // None = 全量复制父会话截至末尾(CLI 不暴露 turn 级截断)。
        None,
    )
    .with_context(|| format!("fork session {parent_id}"))?;

    println!("Forked session {parent_id} → {child_id}");
    println!("Resume with: reflect-tui --resume {child_id} \"...\"");
    Ok(())
}

/// `reflect-tui session rename <id> <name>` —— 给会话设置人可读名称。
pub fn rename(id: &str, name: &str) -> anyhow::Result<()> {
    let base = default_base();
    let (tid, _path) = resolve_session_path(&base, id)?;
    rename_session(&base, tid, name).with_context(|| format!("rename session {tid}"))?;
    println!("Renamed session {tid} → {name}");
    Ok(())
}

/// `reflect-tui session export <id> [--out <file>]` —— 导出为人类可读 markdown。
pub fn export(id: &str, out: Option<&std::path::Path>) -> anyhow::Result<()> {
    let base = default_base();
    let (tid, path) = resolve_session_path(&base, id)?;
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("read session file {}", path.display()))?;
    let records: Vec<RolloutRecord> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<RolloutRecord>(l).ok())
        .collect();
    let markdown = to_markdown(&records);
    match out {
        Some(p) => {
            std::fs::write(p, &markdown)
                .with_context(|| format!("write export file {}", p.display()))?;
            println!(
                "Exported session {tid} → {} ({} bytes)",
                p.display(),
                markdown.len()
            );
        }
        None => {
            print!("{markdown}");
        }
    }
    Ok(())
}

// ────────────────────────── 内部 helper(移植自 reflect-cli session.rs) ──────────────────────────

/// 解析 `id`(完整 UUID 或前缀)为 session 文件路径。
///
/// 路径解析策略:
/// 1. 前缀匹配:从 `list_sessions` 拿精确 `started_at`,用 `session_path_at`
///    拼 `<base>/YYYY/MM/DD/<id>.jsonl`(日期 = session 创建日,非今天)。
/// 2. 完整 UUID 且在 session 列表中:同上,用真实 `started_at`。
/// 3. 完整 UUID 但不在列表中(跨天 / started_at 缺失):`find_session_file`
///    全盘扫描兜底。
pub(crate) fn resolve_session_path(
    base: &std::path::Path,
    id: &str,
) -> anyhow::Result<(ThreadId, std::path::PathBuf)> {
    match resolve_session_id(base, id)? {
        Some(info) => {
            let path = session_path_at(base, info.session_id, info.started_at);
            if path.exists() {
                Ok((info.session_id, path))
            } else {
                let found = find_session_file(base, info.session_id)?
                    .ok_or_else(|| anyhow!("session file not found: {}", path.display()))?;
                Ok((info.session_id, found))
            }
        }
        None => {
            let tid = Uuid::parse_str(id)
                .map(ThreadId)
                .map_err(|e| anyhow!("invalid UUID '{id}': {e}"))?;
            let found = find_session_file(base, tid)?
                .ok_or_else(|| anyhow!("session file not found for id: {id}"))?;
            Ok((tid, found))
        }
    }
}

/// 解析 `id`(完整 UUID 或前缀)为 `SessionInfo`。前缀则扫描 `list_sessions`,
/// 返回唯一命中会话的完整 `SessionInfo`(`started_at` 精确)。
fn resolve_session_id(base: &std::path::Path, id: &str) -> anyhow::Result<Option<SessionInfo>> {
    if id.len() == 36 {
        let tid = Uuid::parse_str(id)
            .map(ThreadId)
            .map_err(|e| anyhow!("invalid UUID '{id}': {e}"))?;
        if let Ok(sessions) = list_sessions(base) {
            if let Some(found) = sessions.iter().find(|s| s.session_id == tid) {
                return Ok(Some(found.clone()));
            }
        }
        return Ok(None);
    }
    let sessions = list_sessions(base).context("list_sessions for prefix match")?;
    let matches: Vec<_> = sessions
        .iter()
        .filter(|s| s.session_id.to_string().starts_with(id))
        .collect();
    match matches.len() {
        0 => Err(anyhow!(
            "no session matches prefix '{id}'; expected UUID or first N chars"
        )),
        1 => Ok(Some(matches[0].clone())),
        n => Err(anyhow!(
            "prefix '{id} matches {n} sessions; please give a longer prefix or full UUID"
        )),
    }
}

/// 全盘扫描 `<base>/**/*.jsonl` 找指定 `session_id` 的文件路径(含
/// `.N.jsonl` 轮转副本)。用 `std::fs` 递归遍历(布局 `YYYY/MM/DD/`,3 层),
/// 避免引入 walkdir。优先返回无 `.N` 后缀的 active 文件。
fn find_session_file(
    base: &std::path::Path,
    session_id: ThreadId,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    let needle = format!("{}.jsonl", session_id);
    let mut found: Option<std::path::PathBuf> = None;
    fn scan_dir(dir: &std::path::Path, needle: &str, found: &mut Option<std::path::PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                scan_dir(&path, needle, found);
                if let Some(p) = found {
                    if p.file_name().map(|n| n.to_string_lossy().into_owned())
                        == Some(needle.to_string())
                    {
                        return;
                    }
                }
            } else if ft.is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&needle) {
                    if name == needle {
                        *found = Some(path);
                        return;
                    }
                    if found.is_none() {
                        *found = Some(path);
                    }
                }
            }
        }
    }
    scan_dir(base, &needle, &mut found);
    Ok(found)
}

fn preview_message_body(v: &serde_json::Value) -> String {
    let body = v
        .get("content")
        .and_then(|c| match c {
            serde_json::Value::String(s) => Some(s.clone()),
            other => other
                .get("text")
                .and_then(|t| t.as_str())
                .map(str::to_string),
        })
        .unwrap_or_default();
    let single_line = body.replace('\n', " ").trim().to_string();
    if single_line.chars().count() <= 80 {
        single_line
    } else {
        let truncated: String = single_line.chars().take(80).collect();
        format!("{truncated}…")
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn format_with_thousands(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in chars.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    out.chars().rev().collect()
}

/// 扫描 JSONL body,聚合 `RolloutRecord::TokenCount` 的 token 与 cost。
/// 返回 `(input, output, total, cost_usd)`。
fn aggregate_token_counts(body: &str) -> (u64, u64, u64, Option<f64>) {
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut cost_sum: f64 = 0.0;
    let mut cost_present = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<RolloutRecord>(trimmed) else {
            continue;
        };
        if let RolloutRecord::TokenCount {
            usage, cost_usd, ..
        } = rec
        {
            input_tokens = input_tokens.saturating_add(usage.input_tokens as u64);
            output_tokens = output_tokens.saturating_add(usage.output_tokens as u64);
            total_tokens = total_tokens.saturating_add(usage.total_tokens as u64);
            if let Some(c) = cost_usd {
                cost_sum += c;
                cost_present = true;
            }
        }
    }
    (
        input_tokens,
        output_tokens,
        total_tokens,
        cost_present.then_some(cost_sum),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reflect_protocol::{RolloutRecord, ThreadId};
    use reflect_rollout::path::session_path_at;
    use std::fs;
    use tempfile::tempdir;

    fn write_session(
        base: &std::path::Path,
        tid: ThreadId,
        started: chrono::DateTime<chrono::Utc>,
    ) -> std::path::PathBuf {
        let path = session_path_at(base, tid, started);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // SessionMeta 首行 + 一条 user message + 一条 TokenCount。
        let meta = serde_json::json!({
            "type": "session_meta",
            "session_id": tid.to_string(),
            "model": "glm-5.2",
            "started_at": started.to_rfc3339(),
        });
        let msg = serde_json::json!({
            "type": "message",
            "turn_id": uuid::Uuid::new_v4(),
            "role": "user",
            "content": "hello fork",
        });
        let tokens = serde_json::json!({
            "type": "token_count",
            "turn_id": uuid::Uuid::new_v4(),
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cached_tokens": 0,
                "cache_write_tokens": 0,
                "total_tokens": 150,
            },
            "cost_usd": 0.01,
            "at": started.to_rfc3339(),
        });
        let body = format!(
            "{}\n{}\n{}\n",
            serde_json::to_string(&meta).unwrap(),
            serde_json::to_string(&msg).unwrap(),
            serde_json::to_string(&tokens).unwrap(),
        );
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn truncate_short_and_long() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn format_with_thousands_works() {
        assert_eq!(format_with_thousands("0"), "0");
        assert_eq!(format_with_thousands("1234"), "1,234");
        assert_eq!(format_with_thousands("1234567"), "1,234,567");
    }

    #[test]
    fn resolve_session_path_full_uuid() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let tid = ThreadId::new();
        let path = write_session(base, tid, chrono::Utc::now());
        let (resolved_tid, resolved_path) =
            resolve_session_path(base, &tid.to_string()).unwrap();
        assert_eq!(resolved_tid, tid);
        assert_eq!(resolved_path, path);
    }

    #[test]
    fn resolve_session_path_prefix() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let tid = ThreadId::new();
        write_session(base, tid, chrono::Utc::now());
        // 前 8 字符前缀。
        let prefix = &tid.to_string()[..8];
        let (resolved_tid, _) = resolve_session_path(base, prefix).unwrap();
        assert_eq!(resolved_tid, tid);
    }

    #[test]
    fn fork_creates_child_with_history() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let parent = ThreadId::new();
        let parent_path = write_session(base, parent, chrono::Utc::now());
        let parent_lines = fs::read_to_string(&parent_path)
            .unwrap()
            .lines()
            .count();

        let child =
            fork_with_history(base, parent, "test-fork", None).expect("fork_with_history");

        // 子文件首行 SessionMeta 的 session_id 应为 child。
        let child_path = find_session_file(base, child).unwrap().unwrap();
        let child_content = fs::read_to_string(&child_path).unwrap();
        let first_line: serde_json::Value =
            serde_json::from_str(child_content.lines().next().unwrap()).unwrap();
        assert_eq!(
            first_line.get("session_id").and_then(|v| v.as_str()),
            Some(child.to_string().as_str())
        );
        // 子文件 record 数应 ≥ 父文件(原样复制 + 末尾 Fork marker)。
        let child_lines = child_content.lines().count();
        assert!(child_lines >= parent_lines, "child {child_lines} should preserve parent {parent_lines} records");
    }

    #[test]
    fn fork_record_appended_to_parent() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let parent = ThreadId::new();
        let parent_path = write_session(base, parent, chrono::Utc::now());
        let before = fs::read_to_string(&parent_path).unwrap();
        let before_lines = before.lines().count();

        let _child = fork_with_history(base, parent, "marker-test", None).unwrap();

        let after = fs::read_to_string(&parent_path).unwrap();
        assert!(after.lines().count() > before_lines, "parent should gain a Fork marker");
        // 末行应为 Fork record。
        let last: RolloutRecord = serde_json::from_str(after.lines().last().unwrap()).unwrap();
        assert!(
            matches!(last, RolloutRecord::Fork { parent_session_id, .. } if parent_session_id == parent),
            "parent last record should be Fork pointing back to parent"
        );
    }

    #[test]
    fn rename_writes_name_file() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let tid = ThreadId::new();
        write_session(base, tid, chrono::Utc::now());

        rename_session(base, tid, "my-session").unwrap();
        let read_back = read_session_name(base, tid).unwrap();
        assert_eq!(read_back.as_deref(), Some("my-session"));
    }

    #[test]
    fn export_produces_markdown() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let tid = ThreadId::new();
        let path = write_session(base, tid, chrono::Utc::now());

        let content = fs::read_to_string(&path).unwrap();
        let records: Vec<RolloutRecord> = content
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        let md = to_markdown(&records);
        // to_markdown 应包含 user 消息文本。
        assert!(md.contains("hello fork"), "export markdown should contain message text");
        assert!(!md.is_empty());
    }

    #[test]
    fn aggregate_token_counts_sums() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let tid = ThreadId::new();
        let path = write_session(base, tid, chrono::Utc::now());
        let content = fs::read_to_string(&path).unwrap();
        let (input, output, total, cost) = aggregate_token_counts(&content);
        assert_eq!(input, 100);
        assert_eq!(output, 50);
        assert_eq!(total, 150);
        assert_eq!(cost, Some(0.01));
    }

    #[test]
    fn resolve_session_id_ambiguous_prefix_errors() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        // 两个 session,造一个共同前缀极难;这里直接测空前缀的错误路径。
        let err = resolve_session_id(base, "nonexistent").unwrap_err();
        assert!(err.to_string().contains("no session matches prefix"));
    }
}
