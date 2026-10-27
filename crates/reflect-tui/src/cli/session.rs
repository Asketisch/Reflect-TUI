//! `reflect-tui session ...` —— fork / rename / export / ls / show / rm 本地 session。
//!
//! 与 `reflect-cli session ...` 对称:所有底层操作都走 `reflect-rollout`
//! 的公开 API。`id` 可以是完整 UUID 或前 8+ 字符前缀。
//!
//! fork 子命令创建子会话后打印 child id,提示用
//! `reflect-tui --resume <child_id>` 续作(与 reflect-cli 的
//! `reflect exec --resume` 语义一致,但二进制名不同)。

use anyhow::{Context, anyhow};
use zhuye_protocol::{ModelUsage, RolloutRecord, SessionInfo, SessionStats, ThreadId};
use zhuye_rollout::index::list_sessions;
use zhuye_rollout::path::{default_base, session_path_at};
// v1.x:接入此前孤儿的 rename/read_name/to_markdown + fork_with_history。
use zhuye_rollout::index::{fork_with_history, read_session_name, rename_session};
// v1.x:会话文件定位走 `reflect-rollout` 的共享 API —— 文件名是 thread id,
// 与首行 `SessionMeta.session_id`(内部 id)可能不同(生产中一致,历史文件
// / 外部 recorder 可能分歧),共享 API 同时按两种 id 匹配。
use zhuye_rollout::index::{
    find_file_by_session_id, find_files_by_name_prefix, first_line_session_id,
};
// v1.x:统一统计聚合(与 reflect-cli 同款)—— 不在 TUI 侧复刻 token 聚合
// 逻辑,改为直接消费 `reflect-rollout` 的读取侧 API。
use uuid::Uuid;
use zhuye_rollout::{all_model_usage, session_stats, to_markdown};

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
/// 整 session 累计 token / cost + per-model 用量表。`id` 可以是完整
/// UUID 或前缀。
///
/// v1.x:统计统一走 `zhuye_rollout::session_stats`(全量扫描活跃 +
/// 轮转文件),不再在 TUI 侧复刻聚合逻辑。
pub fn show(id: &str) -> anyhow::Result<()> {
    let base = default_base();
    let (tid, path) = resolve_session_path(&base, id)?;
    println!("Session: {tid}");
    if let Ok(Some(name)) = read_session_name(&base, tid) {
        println!("Name:    {name}");
    }
    println!("Path:    {}", path.display());

    let content = std::fs::read_to_string(&path).context("read session jsonl")?;
    // v1.x:统一统计(活跃 + 轮转文件全扫)。旧 session 无 TokenCount →
    // 全 0 / None,不破坏展示。
    let stats = session_stats(&base, tid);

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
                if let Some(s) = &stats {
                    print_stats_summary(s);
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
            // v1.x:记忆访问审计(读 = 检索注入;写 = compact 固化)。
            "memory_access" if msg_count < 5 => {
                msg_count += 1;
                let op = v.get("op").and_then(|o| o.as_str()).unwrap_or("?");
                let scope = v.get("scope").and_then(|s| s.as_str()).unwrap_or("?");
                let chars = v.get("chars").and_then(|c| c.as_u64()).unwrap_or(0);
                println!("[{msg_count:>2}] memory {op} [{scope}]: {chars} char(s)");
            }
            _ => {}
        }
    }
    if !meta_printed {
        if let Some(s) = &stats {
            print_stats_summary(s);
        }
    }
    // v1.x:per-model 用量表(每个模型的使用量)。
    if let Some(s) = &stats
        && !s.per_model.is_empty()
    {
        println!();
        print_model_usage_table(&s.per_model);
    }
    if msg_count == 0 {
        println!("(no message records in this session)");
    }
    Ok(())
}

/// `reflect-tui session stats [--workspace W]` —— 跨会话按模型汇总 token
/// 用量(v1.x 模型级统计的全局视图)。
///
/// - 不带 `--workspace`:统计 base 下全部会话(`all_model_usage`);
/// - 带 `--workspace`:只统计 `SessionMeta.workspace` 匹配的会话
///   (逐会话 `session_stats` 后按模型合并)。
pub fn stats(workspace: Option<&str>) -> anyhow::Result<()> {
    let base = default_base();
    let usage: Vec<ModelUsage> = match workspace {
        Some(ws) => {
            let sessions = list_sessions(&base).context("list_sessions")?;
            let scoped: Vec<_> = sessions
                .into_iter()
                .filter(|s| s.workspace.as_deref() == Some(ws))
                .collect();
            if scoped.is_empty() {
                println!(
                    "(no sessions found for workspace {ws} under {})",
                    base.display()
                );
                return Ok(());
            }
            // 逐会话统计后按模型合并(合并已聚合的数据,不重复扫文件)。
            let mut merged: Vec<ModelUsage> = Vec::new();
            for s in &scoped {
                if let Some(st) = session_stats(&base, s.session_id) {
                    merge_model_usage(&mut merged, st.per_model);
                }
            }
            merged.sort_by(|a, b| {
                b.total_tokens
                    .cmp(&a.total_tokens)
                    .then_with(|| a.model.cmp(&b.model))
            });
            println!(
                "Model usage across {} session(s) in workspace {ws}:",
                scoped.len()
            );
            merged
        }
        None => {
            let all = all_model_usage(&base);
            if all.is_empty() {
                println!("(no token usage recorded under {})", base.display());
                return Ok(());
            }
            let n = list_sessions(&base)?.len();
            println!("Model usage across {n} session(s):");
            all
        }
    };
    print_model_usage_table(&usage);
    // 总计行。
    let total_calls: u64 = usage.iter().map(|m| m.calls).sum();
    let total_in: u64 = usage.iter().map(|m| m.input_tokens).sum();
    let total_out: u64 = usage.iter().map(|m| m.output_tokens).sum();
    let total_cached: u64 = usage.iter().map(|m| m.cached_tokens).sum();
    let total_tokens: u64 = usage.iter().map(|m| m.total_tokens).sum();
    let total_cost: Option<f64> = {
        let mut sum = 0.0;
        let mut present = false;
        for m in &usage {
            if let Some(c) = m.cost_usd {
                sum += c;
                present = true;
            }
        }
        present.then_some(sum)
    };
    println!(
        "{:<40}  {:>6}  {:>12}  {:>12}  {:>12}  {:>14}  {:>10}",
        "TOTAL",
        total_calls,
        format_with_thousands(&total_in.to_string()),
        format_with_thousands(&total_out.to_string()),
        format_with_thousands(&total_cached.to_string()),
        format_with_thousands(&total_tokens.to_string()),
        match total_cost {
            Some(c) => format!("${:.2}", c),
            None => String::from("--"),
        },
    );
    Ok(())
}

/// 打印会话级统计摘要(Tokens / Cost / 工具调用 / 压缩 / 记忆 / 缓存命中)。
fn print_stats_summary(s: &SessionStats) {
    println!(
        "Tokens:  {} (in {} / out {})",
        format_with_thousands(&s.total_tokens.to_string()),
        format_with_thousands(&s.input_tokens.to_string()),
        format_with_thousands(&s.output_tokens.to_string()),
    );
    if let Some(c) = s.cost_usd {
        println!("Cost:    ${:.2}", c);
    }
    if s.tool_calls > 0 || s.compactions > 0 {
        println!(
            "Tools:   {} call(s), {} compaction(s)",
            s.tool_calls, s.compactions
        );
    }
    // v1.x:压缩节省的 token(before 之和 - after 之和;旧记录无字段 → 0)。
    let saved = s.compaction_tokens_saved();
    if saved > 0 {
        println!(
            "Compact: saved {} token(s) ({} → {})",
            format_with_thousands(&saved.to_string()),
            format_with_thousands(&s.compaction_before_tokens.to_string()),
            format_with_thousands(&s.compaction_after_tokens.to_string()),
        );
    }
    // v1.x:记忆读写审计计数(无记录时不显示)。
    if s.memory_reads > 0 || s.memory_writes > 0 {
        println!(
            "Memory:  {} read(s), {} write(s)",
            s.memory_reads, s.memory_writes
        );
    }
    if let Some(rate) = s.cache_hit_rate()
        && rate > 0.0
    {
        println!(
            "Cache:   {:.1}% hit ({} cached)",
            rate * 100.0,
            s.cached_tokens
        );
    }
}

/// 打印 per-model 用量表(列宽与 `stats` 的总计行对齐)。
fn print_model_usage_table(usage: &[ModelUsage]) {
    println!("Per-model usage:");
    println!(
        "  {:<38}  {:>6}  {:>12}  {:>12}  {:>12}  {:>14}  {:>10}",
        "model", "calls", "input", "output", "cached", "total", "cost"
    );
    for m in usage {
        let model = truncate(&m.model, 38);
        let cost = match m.cost_usd {
            Some(c) => format!("${:.2}", c),
            None => String::from("--"),
        };
        println!(
            "  {:<38}  {:>6}  {:>12}  {:>12}  {:>12}  {:>14}  {:>10}",
            model,
            m.calls,
            format_with_thousands(&m.input_tokens.to_string()),
            format_with_thousands(&m.output_tokens.to_string()),
            format_with_thousands(&m.cached_tokens.to_string()),
            format_with_thousands(&m.total_tokens.to_string()),
            cost,
        );
    }
}

/// 把 `src` 的 per-model 用量合并进 `dst`(按 model 键累加)。
/// `stats --workspace` 的逐会话合并用;同模型跨会话累加。
fn merge_model_usage(dst: &mut Vec<ModelUsage>, src: Vec<ModelUsage>) {
    for m in src {
        match dst.iter_mut().find(|d| d.model == m.model) {
            Some(d) => {
                d.calls += m.calls;
                d.input_tokens += m.input_tokens;
                d.output_tokens += m.output_tokens;
                d.cached_tokens += m.cached_tokens;
                d.cache_write_tokens += m.cache_write_tokens;
                d.total_tokens += m.total_tokens;
                // cost 存在性:任一带价即带价,求和。
                match (d.cost_usd, m.cost_usd) {
                    (Some(a), Some(b)) => d.cost_usd = Some(a + b),
                    (None, Some(b)) => d.cost_usd = Some(b),
                    (a, None) => d.cost_usd = a,
                }
            }
            None => dst.push(m),
        }
    }
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
            // 原子落盘:直接写目标文件时若进程崩溃/磁盘满,用户指定的导出路径
            // 会被截断成一个不完整的 markdown,而且看不出是坏的。同目录
            // temp + rename 保证要么完整要么原文件不动。
            let tmp = p.with_extension(format!("md.tmp.{}", std::process::id()));
            std::fs::write(&tmp, &markdown)
                .with_context(|| format!("write export temp {}", tmp.display()))?;
            std::fs::rename(&tmp, p).with_context(|| format!("finalize export {}", p.display()))?;
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

/// `reflect-tui session search <query>` —— 跨会话全文搜索(v1.5 新功能)。
///
/// 在全部本地 session 的消息内容里找包含 `query`(大小写不敏感子串)的
/// 记录,按「最近活跃优先」输出命中摘要。输出字段:session id 前缀
/// (可直接喂给 `reflect-tui --resume` / `session show`)、角色、摘要。
/// `--json` 以 JSON 行输出,便于脚本消费。
///
/// 命中摘要在返回前已由 `reflect-rollout` 过密钥脱敏 —— 不会把密钥带进
/// 搜索输出。
pub fn search(query: &str, limit: usize, json: bool) -> anyhow::Result<()> {
    let base = default_base();
    let hits = zhuye_rollout::search_sessions(
        &base,
        query,
        &zhuye_rollout::SearchOptions {
            max_hits: limit.max(1),
            ..Default::default()
        },
    )
    .context("search sessions")?;

    if hits.is_empty() {
        println!("(no matches for {query:?} under {})", base.display());
        return Ok(());
    }

    if json {
        for h in &hits {
            let obj = serde_json::json!({
                "session_id": h.session_id.map(|s| s.to_string()),
                "file": h.file.display().to_string(),
                "line_no": h.line_no,
                "role": h.role,
                "snippet": h.snippet,
            });
            println!("{obj}");
        }
        return Ok(());
    }

    println!("{:<10}  {:<10}  snippet", "session", "role");
    for h in &hits {
        // session id 取前 8 位前缀(resume / show 均支持前缀定位)。
        let id_prefix: String = h
            .session_id
            .map(|s| s.to_string()[..8].to_string())
            .unwrap_or_else(|| "-".into());
        let role = h.role.as_deref().unwrap_or("-");
        println!("{:<10}  {:<10}  {}", id_prefix, role, h.snippet);
    }
    println!(
        "\n{} hit(s);用 `reflect-tui session show <id前缀>` 看详情",
        hits.len()
    );
    Ok(())
}

// ────────────────────────── 内部 helper(移植自 reflect-cli session.rs) ──────────────────────────

/// 解析 `id`(完整 UUID 或前缀)为 session 文件路径。
///
/// 路径解析策略(与 `reflect-cli` 同款,统一走 `reflect-rollout` 共享 API):
/// 1. 前缀匹配:从 `list_sessions` 拿精确 `started_at`,用 `session_path_at`
///    拼 `<base>/YYYY/MM/DD/<id>.jsonl`(日期 = session 创建日,非今天)。
/// 2. 完整 UUID 且在 session 列表中:同上,用真实 `started_at`。
/// 3. 拼出的路径不存在:按文件名找,再按首行内部 id 全盘找
///    (`find_file_by_session_id`)。**文件名是 thread id,与内部
///    `SessionMeta.session_id` 可能不同** —— 只按文件名找会漏。
/// 4. 前缀在内部 id 里 0 命中:回退按文件名(thread id)前缀查找。
pub(crate) fn resolve_session_path(
    base: &std::path::Path,
    id: &str,
) -> anyhow::Result<(ThreadId, std::path::PathBuf)> {
    let resolved = match resolve_session_id(base, id) {
        Ok(r) => r,
        // 内部 session_id 前缀 0 命中:回退按文件名(thread id)前缀查找。
        Err(e) if id.len() != 36 => {
            let files = find_files_by_name_prefix(base, id)?;
            let deduped = dedupe_prefix_files(&files);
            match deduped.len() {
                0 => return Err(e),
                1 => return Ok(deduped.into_iter().next().expect("len == 1")),
                n => {
                    return Err(anyhow!(
                        "prefix '{id}' matches {n} sessions; please give a longer prefix or full UUID"
                    ));
                }
            }
        }
        Err(e) => return Err(e),
    };
    match resolved {
        Some(info) => {
            let path = session_path_at(base, info.session_id, info.started_at);
            if path.exists() {
                Ok((info.session_id, path))
            } else {
                // 文件名是 thread id,与内部 session_id 可能不同:先按文件名
                // 找,再按首行内部 id 全盘找。
                let found = find_session_file(base, info.session_id)?.or_else(|| {
                    find_file_by_session_id(base, &info.session_id)
                        .ok()
                        .flatten()
                });
                found
                    .ok_or_else(|| anyhow!("session file not found: {}", path.display()))
                    .map(|p| (info.session_id, p))
            }
        }
        // 完整 UUID 但不在列表:文件名查找未命中时按首行内部 id 再找
        // (完整 UUID 既可能是 thread id,也可能是内部 session_id)。
        None => {
            let tid = Uuid::parse_str(id)
                .map(ThreadId)
                .map_err(|e| anyhow!("invalid UUID '{id}': {e}"))?;
            let found = find_session_file(base, tid)?
                .or_else(|| find_file_by_session_id(base, &tid).ok().flatten())
                .ok_or_else(|| anyhow!("session file not found for id: {id}"))?;
            Ok((tid, found))
        }
    }
}

/// 按文件**内部** `session_id` 对前缀命中的文件去重,返回
/// `(内部 session_id, 路径)` 列表。同一 session 的活跃文件与轮转 `.N`
/// 副本只保留活跃文件(找不到活跃文件时保留任一副本)。
///
/// 首行无 `SessionMeta`(writer 崩溃等)时退化为文件名 stem;stem 也非
/// UUID 的候选跳过,避免把无关 jsonl 误当作 session。
fn dedupe_prefix_files(files: &[std::path::PathBuf]) -> Vec<(ThreadId, std::path::PathBuf)> {
    let mut out: Vec<(ThreadId, std::path::PathBuf)> = Vec::new();
    for f in files {
        let sid = first_line_session_id(f).or_else(|| {
            f.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".jsonl"))
                .and_then(|s| Uuid::parse_str(s).ok())
                .map(ThreadId)
        });
        let Some(sid) = sid else {
            continue;
        };
        let active = f
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".jsonl"))
            .map(|stem| !stem.contains('.'))
            .unwrap_or(false);
        match out.iter_mut().find(|(s, _)| *s == sid) {
            Some((_, prev)) if active => *prev = f.clone(),
            None => out.push((sid, f.clone())),
            Some(_) => {}
        }
    }
    out
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
                if name.starts_with(needle) {
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

/// 取出一条 message record 的正文预览。
///
/// `content` 有三种历史形态,都要认:
/// - **字符串**:最早的纯文本记录;
/// - **对象** `{ "text": "..." }`:单块结构;
/// - **数组** `[{ "type": "text", "text": "..." }, ...]`:**当前形态**
///   (v1.2 起引擎把 ContentBlock 数组整个写进 `content`)。
///
/// 此前只认前两种,遇到第三种(也就是**所有真实会话**)会取到空串 ——
/// `session show` 的消息预览永远是空的。本函数按数组顺序拼接其中的
/// 文本块,非文本块(图片 / 工具载荷)跳过。
fn preview_message_body(v: &serde_json::Value) -> String {
    let body = v
        .get("content")
        .and_then(|c| match c {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(items) => {
                let mut parts: Vec<String> = Vec::new();
                for item in items {
                    if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                        parts.push(t.to_string());
                    }
                }
                Some(parts.join(" "))
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use zhuye_protocol::{RolloutRecord, ThreadId};
    use zhuye_rollout::path::session_path_at;

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
            // v1.x:模型归属(实际命中的模型 spec)。
            "model": "glm-5.2",
            "provider": "builtin",
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

    // ── 消息预览:三种 content 形态都要认 ────────────────────────────

    #[test]
    fn preview_message_body_accepts_plain_string() {
        let v = serde_json::json!({"content": "hello"});
        assert_eq!(preview_message_body(&v), "hello");
    }

    #[test]
    fn preview_message_body_accepts_text_object() {
        let v = serde_json::json!({"content": {"text": "hello"}});
        assert_eq!(preview_message_body(&v), "hello");
    }

    #[test]
    fn preview_message_body_accepts_content_block_array() {
        // v1.2 起引擎把 ContentBlock 数组写进 `content` —— 这是**真实会话**
        // 的形态。此前只认字符串 / 单块对象,导致 `session show` 的消息
        // 预览对所有真实会话都是空的。
        let v = serde_json::json!({
            "content": [
                {"type": "text", "text": "第一块"},
                {"type": "image", "image_url": "data:..."},
                {"type": "text", "text": "第二块"},
            ]
        });
        assert_eq!(preview_message_body(&v), "第一块 第二块");
    }

    #[test]
    fn preview_message_body_truncates_and_flattens_newlines() {
        let v = serde_json::json!({
            "content": [{"type": "text", "text": "line1\nline2"}]
        });
        assert_eq!(preview_message_body(&v), "line1 line2");

        let long = serde_json::json!({
            "content": [{"type": "text", "text": "x".repeat(200)}]
        });
        let out = preview_message_body(&long);
        assert!(out.ends_with('…'), "超长应截断并加省略号: {out}");
        assert_eq!(out.chars().count(), 81);
    }

    #[test]
    fn preview_message_body_empty_for_unrelated_shape() {
        let v = serde_json::json!({"content": 42});
        assert_eq!(preview_message_body(&v), "");
    }

    #[test]
    fn resolve_session_path_full_uuid() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let tid = ThreadId::new();
        let path = write_session(base, tid, chrono::Utc::now());
        let (resolved_tid, resolved_path) = resolve_session_path(base, &tid.to_string()).unwrap();
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
    fn resolve_session_path_finds_file_when_name_differs_from_internal_id() {
        // 文件名是 thread id,首行 `SessionMeta.session_id` 是内部 id ——
        // 两者可能不同(历史文件 / 外部 recorder)。按内部 id 查询也必须
        // 能定位到实际文件。
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let file_id = ThreadId::new();
        let internal_id = ThreadId::new();
        let started = chrono::Utc::now();
        let path = session_path_at(base, file_id, started);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let meta = serde_json::json!({
            "type": "session_meta",
            "session_id": internal_id.to_string(),
            "model": "glm-5.2",
            "started_at": started.to_rfc3339(),
        });
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&meta).unwrap()),
        )
        .unwrap();

        // 完整内部 id → 定位到该文件。
        let (resolved_tid, resolved_path) =
            resolve_session_path(base, &internal_id.to_string()).unwrap();
        assert_eq!(resolved_tid, internal_id);
        assert_eq!(resolved_path, path);

        // 文件名(thread id)前缀 → 同样能定位。
        let prefix = &file_id.to_string()[..8];
        let (_, resolved_path2) = resolve_session_path(base, prefix).unwrap();
        assert_eq!(resolved_path2, path);
    }

    #[test]
    fn dedupe_prefix_files_prefers_active_file() {
        // 同一 session 的活跃文件与轮转 `.N` 副本只保留活跃文件。
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let sid = ThreadId::new();
        let started = chrono::Utc::now();
        let active = session_path_at(base, sid, started);
        fs::create_dir_all(active.parent().unwrap()).unwrap();
        let meta = serde_json::json!({
            "type": "session_meta",
            "session_id": sid.to_string(),
            "model": "glm-5.2",
            "started_at": started.to_rfc3339(),
        });
        fs::write(
            &active,
            format!("{}\n", serde_json::to_string(&meta).unwrap()),
        )
        .unwrap();
        // 轮转副本:文件名带 `.1` 后缀。
        let rotated = active.with_file_name(format!("{}.1.jsonl", sid));
        fs::write(
            &rotated,
            format!("{}\n", serde_json::to_string(&meta).unwrap()),
        )
        .unwrap();

        let deduped = dedupe_prefix_files(&[rotated.clone(), active.clone()]);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].0, sid);
        assert_eq!(deduped[0].1, active, "应优先保留活跃文件");
    }

    #[test]
    fn fork_creates_child_with_history() {
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let parent = ThreadId::new();
        let parent_path = write_session(base, parent, chrono::Utc::now());
        let parent_lines = fs::read_to_string(&parent_path).unwrap().lines().count();

        let child = fork_with_history(base, parent, "test-fork", None).expect("fork_with_history");

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
        assert!(
            child_lines >= parent_lines,
            "child {child_lines} should preserve parent {parent_lines} records"
        );
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
        assert!(
            after.lines().count() > before_lines,
            "parent should gain a Fork marker"
        );
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
        assert!(
            md.contains("hello fork"),
            "export markdown should contain message text"
        );
        assert!(!md.is_empty());
    }

    #[test]
    fn session_stats_sums_tokens_and_cost() {
        // v1.x:统计统一走 `zhuye_rollout::session_stats`(不再在 TUI 侧
        // 复刻聚合逻辑)。
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let tid = ThreadId::new();
        write_session(base, tid, chrono::Utc::now());
        let stats = session_stats(base, tid).expect("session_stats");
        assert_eq!(stats.input_tokens, 100);
        assert_eq!(stats.output_tokens, 50);
        assert_eq!(stats.total_tokens, 150);
        assert_eq!(stats.cost_usd, Some(0.01));
    }

    #[test]
    fn session_stats_attributes_model() {
        // v1.x:TokenCount 带 model 字段 → per-model 用量按该模型归集。
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let tid = ThreadId::new();
        write_session(base, tid, chrono::Utc::now());
        let stats = session_stats(base, tid).expect("session_stats");
        assert_eq!(stats.per_model.len(), 1);
        let usage = &stats.per_model[0];
        assert_eq!(usage.model, "glm-5.2");
        assert_eq!(usage.calls, 1);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.total_tokens, 150);
        assert_eq!(usage.provider.as_deref(), Some("builtin"));
    }

    #[test]
    fn all_model_usage_aggregates_across_sessions() {
        // v1.x:跨会话按模型汇总(`all_model_usage`)。
        let tmp = tempdir().unwrap();
        let base = tmp.path();
        let a = ThreadId::new();
        let b = ThreadId::new();
        write_session(base, a, chrono::Utc::now());
        write_session(base, b, chrono::Utc::now());
        let usage = all_model_usage(base);
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].model, "glm-5.2");
        assert_eq!(usage[0].calls, 2);
        assert_eq!(usage[0].total_tokens, 300);
    }

    #[test]
    fn merge_model_usage_accumulates_same_model() {
        // v1.x:同模型跨会话合并(调用次数 / token / cost 累加)。
        let mut merged: Vec<ModelUsage> = Vec::new();
        merge_model_usage(
            &mut merged,
            vec![ModelUsage {
                model: "glm-5.2".into(),
                provider: Some("builtin".into()),
                calls: 1,
                input_tokens: 100,
                output_tokens: 50,
                cached_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: 150,
                cost_usd: Some(0.01),
            }],
        );
        merge_model_usage(
            &mut merged,
            vec![ModelUsage {
                model: "glm-5.2".into(),
                provider: Some("builtin".into()),
                calls: 2,
                input_tokens: 200,
                output_tokens: 100,
                cached_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: 300,
                cost_usd: Some(0.02),
            }],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].calls, 3);
        assert_eq!(merged[0].total_tokens, 450);
        // cost 求和(浮点用近似比较)。
        let cost = merged[0].cost_usd.expect("cost");
        assert!((cost - 0.03).abs() < 1e-9, "merged cost: {cost}");
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
