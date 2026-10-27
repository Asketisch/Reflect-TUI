//! `--resume` / `-c` / `-r N` 会话恢复:定位本地 rollout 记录并回放。
//!
//! 与 `reflect_core::resume::records_to_preload` 的分工:后者负责
//! **记录 → LLM 消息**的映射(单一事实源,在 submodule 内);本模块只负责
//! **定位与回放** —— 找到某会话的全部记录文件,按时间正序读出
//! [`RolloutRecord`]。
//!
//! ## 为什么回放直接走 submodule 共享 API
//!
//! submodule `reflect_rollout::reader::replay` 升级后(7912323 → e4560bc)
//! 已具备全盘定位能力:任意日期目录、轮转副本(`<id>.1.jsonl` 编号越大越
//! 旧,活跃文件殿后)合并、以及「文件名 thread id / 首行内部 session id」
//! 双入口。本仓库此前因共享 API 只扫近 3 天而维护的本地全盘扫描
//! (`collect_session_files`)随之删除,消费点回归共享实现。
//!
//! 本模块在共享 API 之上只保留一层「会话不存在 → 显式报错」语义:共享
//! `replay` 对定位不到的会话返回**空 Vec**,直接透传会让 `--resume` 静默
//! 以空历史续作(用户看到「续作成功」却没有任何上文)。

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use reflect_protocol::{RolloutRecord, ThreadId};

/// 回放某会话的全部记录(跨日期桶 + 轮转副本,时间正序)。
///
/// 命中条件由共享 API 保证(任一即可):
/// - 文件名 stem == `tid`(**thread id**);
/// - 文件首行的 `SessionMeta.session_id` == `tid`(**内部 id**,UI 与索引
///   展示的就是它,来自其它客户端写入的会话两者可能不同)。
pub async fn replay_all(base: &Path, tid: &ThreadId) -> Result<Vec<RolloutRecord>> {
    let records = reflect_rollout::reader::replay(base, *tid)
        .await
        .with_context(|| format!("回放 session {tid} 失败"))?;
    if records.is_empty() {
        anyhow::bail!(
            "session {tid} 的记录文件不存在于 {}(无法 resume)",
            base.display()
        );
    }
    Ok(records)
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
        let tid =
            reflect_rollout::index::first_line_session_id(&path).or_else(|| stem_thread_id(&path));
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

    #[tokio::test]
    async fn 不存在的会话回放报错() {
        // 共享 replay 对定位不到的会话返回空 Vec;包装层必须转成显式错误,
        // 否则 --resume 会以空历史静默续作。
        let tmp = std::env::temp_dir().join(format!("resume-miss-{}", uuid::Uuid::new_v4()));
        let base = tmp.join("sessions");
        fs::create_dir_all(&base).unwrap();
        let err = replay_all(&base, &ThreadId::new()).await.unwrap_err();
        assert!(
            err.to_string().contains("无法 resume"),
            "应报「无法 resume」: {err}"
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn 回放命中跨日期桶与轮转副本的记录() {
        // 行为级守护:共享 API 的定位面(任意日期 + 轮转副本)必须原样到达
        // 本仓库消费点 —— 造一个「旧日期桶里放轮转副本、新日期桶放活跃文件」
        // 的会话,全部记录都应被读出且按时间正序拼接。
        let tmp = std::env::temp_dir().join(format!("replay-all-{}", uuid::Uuid::new_v4()));
        let base = tmp.join("sessions");
        let tid = ThreadId::new();
        let meta = serde_json::json!({
            "type": "session_meta",
            "session_id": tid.to_string(),
            "model": "test-model",
            "started_at": "2026-01-01T00:00:00Z",
        });
        // 旧日期桶:轮转副本(编号越大越旧);新日期桶:活跃文件。
        let old_dir = base.join("2026/01/01");
        let new_dir = base.join("2026/02/02");
        fs::create_dir_all(&old_dir).unwrap();
        fs::create_dir_all(&new_dir).unwrap();
        fs::write(old_dir.join(format!("{tid}.2.jsonl")), format!("{meta}\n")).unwrap();
        fs::write(old_dir.join(format!("{tid}.1.jsonl")), format!("{meta}\n")).unwrap();
        fs::write(new_dir.join(format!("{tid}.jsonl")), format!("{meta}\n")).unwrap();

        let records = replay_all(&base, &tid).await.unwrap();
        // 每个文件首行都是一条 SessionMeta 记录 → 3 个文件 3 条记录。
        assert_eq!(records.len(), 3, "跨日期桶 + 轮转副本应全部回放");
        let _ = fs::remove_dir_all(&tmp);
    }
}
