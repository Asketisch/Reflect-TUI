//! `--resume` / `-c` / `-r N` 会话恢复:定位本地 rollout 记录并回放。
//!
//! 与 `reflect_core::resume::records_to_preload` 的分工:后者负责
//! **记录 → LLM 消息**的映射(单一事实源,在 submodule 内);本模块只负责
//! **定位与回放** —— 找到某会话的全部记录文件,按时间正序读出
//! [`RolloutRecord`]。submodule 只读消费,不复用其内部实现。
//!
//! ## 为什么不能直接用 `reflect_rollout::reader::replay`
//!
//! `reader::replay` 用 `Utc::now() - days(0..3)` 拼日期桶(即只扫今天 /
//! 昨天 / 前天),并且路径由 **文件名 thread id** 拼出(`session_path_at`)。
//! 于是:
//!
//! - **3 天以上前创建的会话 → 回放为空**,`--resume` 静默丢历史(用户
//!   看到的是「续作成功但没有任何上文」);
//! - **文件名 thread id ≠ 文件内首行 `SessionMeta` 的内部 id** 时(双 ID
//!   设计,UI 与索引展示的是内部 id),拼出来的路径根本不存在;
//! - **轮转副本**(`<id>.1.jsonl` / `.2.jsonl` …)不参与合并,长会话只能
//!   回放最后一段。
//!
//! 本模块改为**全盘扫描** `<base>/YYYY/MM/DD/**` 下的 `.jsonl`,按
//! 「首行内部 id == 目标」或「文件名 stem == 目标」匹配,再按
//! (日期桶升序 → 轮转号降序 → 活跃文件殿后)合并成一条时间正序的记录流。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use reflect_protocol::{RolloutRecord, ThreadId};

/// 收集某会话的全部记录文件(跨日期桶 + 轮转副本),按**时间正序**返回。
///
/// 命中条件(任一即可):
/// - 文件首行的 `SessionMeta.session_id` == `tid`(**内部 id**,UI 与索引
///   展示的就是它);
/// - 文件名 stem == `tid`(**thread id**)。
///
/// 排序:(日期桶目录升序 → 同桶内轮转号大者在前 → 活跃文件殿后)。
/// writer 语义:`.jsonl` 写满后轮转为 `.1.jsonl`、再满轮转为 `.2.jsonl` ——
/// 即**编号越大越旧**。
pub fn collect_session_files(base: &Path, tid: &ThreadId) -> Result<Vec<PathBuf>> {
    let target = tid.to_string();
    let mut files: Vec<(String, u64, PathBuf)> = Vec::new();

    // 递归扫描 base 下的所有子目录(布局为 YYYY/MM/DD)。
    fn scan(
        dir: &Path,
        rel: &str,
        target: &str,
        tid: &ThreadId,
        out: &mut Vec<(String, u64, PathBuf)>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let sub = if rel.is_empty() {
                    entry.file_name().to_string_lossy().to_string()
                } else {
                    format!("{rel}/{}", entry.file_name().to_string_lossy())
                };
                scan(&path, &sub, target, tid, out);
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // 只认 `.jsonl`;`.tmp` / `.bak` 等一律排除。
            let Some(stem) = name.strip_suffix(".jsonl") else {
                continue;
            };
            let by_name = stem == target;
            let by_meta = reflect_rollout::index::first_line_session_id(&path).as_ref() == Some(tid);
            if !by_name && !by_meta {
                continue;
            }
            // 轮转号:`<stem>.N.jsonl` 的 N 越大越旧;`<stem>.jsonl` 是活跃文件(键 0)。
            let rot = match stem.rsplit_once('.') {
                Some((_, num)) if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) => {
                    num.parse().unwrap_or(0)
                }
                _ => 0,
            };
            out.push((rel.to_string(), rot, path));
        }
    }
    scan(base, "", &target, tid, &mut files);

    if files.is_empty() {
        anyhow::bail!(
            "session {tid} 的记录文件不存在于 {}(无法 resume)",
            base.display()
        );
    }
    // 时间正序:日期桶升序 → 同桶内轮转号大者(旧)在前 → 活跃文件(键 0)殿后。
    // 末位以路径兜底,保证排序稳定可复现。
    files.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    Ok(files.into_iter().map(|(_, _, p)| p).collect())
}

/// 回放某会话的全部记录(跨日期桶 + 轮转副本,时间正序)。
pub async fn replay_all(base: &Path, tid: &ThreadId) -> Result<Vec<RolloutRecord>> {
    let paths = collect_session_files(base, tid)?;
    let mut out = Vec::new();
    for p in &paths {
        let mut chunk = reflect_rollout::reader::replay_path(p)
            .await
            .with_context(|| format!("回放 {} 失败", p.display()))?;
        out.append(&mut chunk);
    }
    Ok(out)
}

/// 解析 `--resume` 的 id:完整 UUID,或**唯一**前缀(≥4 位)。
///
/// 前缀语义与 headless `session show` 对齐:短于 4 位直接拒绝,命中多个
/// 会话时报错并给出候选数,避免误续作。
pub fn resolve_thread_id(id_or_prefix: &str) -> Result<ThreadId> {
    if let Ok(tid) = ThreadId::parse_str(id_or_prefix) {
        return Ok(tid);
    }
    if !id_or_prefix
        .chars()
        .all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        anyhow::bail!("无效的 session id '{id_or_prefix}':应为 UUID 或其前缀");
    }
    if id_or_prefix.len() < 4 {
        anyhow::bail!("session id 前缀过短(至少 4 个字符)");
    }
    let base = reflect_rollout::path::default_base();
    let mut hits: Vec<ThreadId> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for path in reflect_rollout::index::find_files_by_name_prefix(&base, id_or_prefix)
        .map_err(|e| anyhow::anyhow!("扫描 session 目录失败: {e}"))?
    {
        // 内部 id 优先(UI 展示的就是它);文件首行无 SessionMeta 时退回文件名 stem。
        let tid = reflect_rollout::index::first_line_session_id(&path).or_else(|| stem_thread_id(&path));
        if let Some(t) = tid
            && seen.insert(t.to_string())
        {
            hits.push(t);
        }
    }
    match hits.len() {
        1 => Ok(hits[0]),
        0 => anyhow::bail!("session '{id_or_prefix}' 不存在"),
        _ => anyhow::bail!(
            "session id 前缀 '{id_or_prefix}' 命中 {} 个会话,请使用更长的前缀",
            hits.len()
        ),
    }
}

/// 从文件名反推 thread id:`<stem>.jsonl` / `<stem>.N.jsonl` → `stem`。
///
/// 轮转副本的 stem 去掉 `.N` 后缀;解析不成 UUID 则返回 `None`。
fn stem_thread_id(path: &Path) -> Option<ThreadId> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".jsonl")?;
    let stem = match stem.rsplit_once('.') {
        Some((base, num)) if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) => base,
        _ => stem,
    };
    ThreadId::parse_str(stem).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 造一个 session 目录:<base>/YYYY/MM/DD/<tid>.jsonl(首行 SessionMeta)。
    fn write_session(base: &Path, day: &str, tid: &ThreadId, body: &str) -> PathBuf {
        let dir = base.join(day);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{tid}.jsonl"));
        let meta = serde_json::json!({
            "type": "session_meta",
            "session_id": tid.to_string(),
            "model": "test-model",
            "started_at": "2026-01-01T00:00:00Z",
        });
        fs::write(&path, format!("{meta}\n{body}")).unwrap();
        path
    }

    #[test]
    fn 收集跨日期桶的全部文件() {
        let tmp = std::env::temp_dir().join(format!("resume-collect-{}", uuid::Uuid::new_v4()));
        let base = tmp.join("sessions");
        let tid = ThreadId::new();
        write_session(&base, "2026/01/01", &tid, "");
        write_session(&base, "2026/02/02", &tid, "");
        // 别的会话不应被收进来。
        write_session(&base, "2026/01/01", &ThreadId::new(), "");

        let files = collect_session_files(&base, &tid).unwrap();
        assert_eq!(files.len(), 2, "应跨日期桶收齐 2 个文件");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn 轮转副本按编号降序排在活跃文件之前() {
        let tmp = std::env::temp_dir().join(format!("resume-rot-{}", uuid::Uuid::new_v4()));
        let base = tmp.join("sessions");
        let tid = ThreadId::new();
        let dir = base.join("2026/01/01");
        fs::create_dir_all(&dir).unwrap();
        let meta = serde_json::json!({
            "type": "session_meta",
            "session_id": tid.to_string(),
            "model": "test-model",
            "started_at": "2026-01-01T00:00:00Z",
        });
        // 活跃文件 + 两份轮转副本(编号越大越旧)。
        for name in [
            format!("{tid}.jsonl"),
            format!("{tid}.1.jsonl"),
            format!("{tid}.2.jsonl"),
        ] {
            fs::write(dir.join(name), format!("{meta}\n")).unwrap();
        }

        let files = collect_session_files(&base, &tid).unwrap();
        assert_eq!(files.len(), 3, "活跃 + 2 份轮转副本");
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        // 时间正序:轮转号大者(旧)在前 → 活跃文件殿后。
        assert_eq!(
            names,
            vec![
                format!("{tid}.2.jsonl"),
                format!("{tid}.1.jsonl"),
                format!("{tid}.jsonl"),
            ],
            "轮转副本应按编号降序在前、活跃文件殿后"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn 不存在的会话报错() {
        let tmp = std::env::temp_dir().join(format!("resume-miss-{}", uuid::Uuid::new_v4()));
        let base = tmp.join("sessions");
        fs::create_dir_all(&base).unwrap();
        let err = collect_session_files(&base, &ThreadId::new()).unwrap_err();
        assert!(
            err.to_string().contains("无法 resume"),
            "应报「无法 resume」: {err}"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn 非_jsonl_干扰文件被排除() {
        let tmp = std::env::temp_dir().join(format!("resume-ext-{}", uuid::Uuid::new_v4()));
        let base = tmp.join("sessions");
        let tid = ThreadId::new();
        write_session(&base, "2026/01/01", &tid, "");
        // 同名的 .tmp / .bak 干扰文件不应被收进来。
        let dir = base.join("2026/01/01");
        fs::write(dir.join(format!("{tid}.jsonl.tmp")), "").unwrap();
        fs::write(dir.join(format!("{tid}.jsonl.bak")), "").unwrap();

        let files = collect_session_files(&base, &tid).unwrap();
        assert_eq!(files.len(), 1, "只应收到 .jsonl 正式文件");
        let _ = fs::remove_dir_all(&tmp);
    }
}
