//! 跨会话 prompt 历史(Up / Ctrl+P / Ctrl+R 召回)。
//!
//! ## 为什么是自包含实现
//!
//! `tui_core/bottom_pane/chat_composer_history/` 有一套完整的 vendored 历史机制
//! (持久化条目 + 本地条目合并、Ctrl+R 增量搜索、批量加载游标),但它跑在一套
//! `AppEvent` 往返上 —— `LookupMessageHistoryEntry` → `ThreadHistoryEntryResponse`、
//! `AppendMessageHistoryEntry` 持久化 —— 这套往返的对面是 vendored `App` /
//! `AppServerClient`,而外层主循环把除 `StartFileSearch` 之外的 AppEvent 全部
//! 当空操作丢掉(`composer_input.rs` 的 `handle_app_events`)。所以那套机制在
//! 当前架构下是死代码,Up / Ctrl+P / Ctrl+R 全是死键。
//!
//! 这里按外层 TUI 的既定模式(`keymap.rs` 同款)做一个自包含薄层:JSONL 落盘 +
//! 内存游标。不追求 vendored 那套的批量游标/增量搜索,先把「按 Up 能召回上一条
//! 提示词」这个所有一线 harness 都有的基本能力补上。
//!
//! ## 落盘原子性
//!
//! 追加写走 temp + rename:直接 `fs::write` 截断原文件时若进程崩溃,用户会丢掉
//! **全部**历史而不是最后一条(与 submodule 8564f66 修 permissions.toml 同类)。

use std::io::Write;
use std::path::{Path, PathBuf};

/// 保留的最大条目数。超出后从最旧的开始丢。
pub const MAX_ENTRIES: usize = 500;

/// 单条提示词的最大字节数。超长的(粘贴整份文件)不进历史,
/// 否则一次误触 Up 就把整个 composer 撑爆。
pub const MAX_ENTRY_BYTES: usize = 8 * 1024;

/// 跨会话 prompt 历史。
///
/// `entries`  oldest → newest;`cursor` 为 `None` 表示当前不在浏览历史
/// (此时 Up/Down 归 composer 自己做光标移动)。
#[derive(Debug, Clone, Default)]
pub struct PromptHistory {
    entries: Vec<String>,
    /// 浏览游标:`Some(i)` = 当前显示 `entries[i]`。
    cursor: Option<usize>,
    /// 进入历史浏览前保存的草稿,向下越过最新一条时还原。
    draft: Option<String>,
}

impl PromptHistory {
    /// 从磁盘加载。文件不存在 / 解析失败都退化为空历史 —— 历史是增强,
    /// 不是启动前提,绝不能因为一个坏文件让 TUI 起不来。
    pub fn load(path: &Path) -> Self {
        let entries = match std::fs::read_to_string(path) {
            Ok(text) => parse_entries(&text),
            Err(_) => Vec::new(),
        };
        Self {
            entries,
            cursor: None,
            draft: None,
        }
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 是否正在浏览历史(此时 Up/Down 由历史消费,不归 composer)。
    pub fn is_browsing(&self) -> bool {
        self.cursor.is_some()
    }

    /// 结束浏览并复位游标(提交 / 清空 composer 时调用)。
    pub fn reset_cursor(&mut self) {
        self.cursor = None;
        self.draft = None;
    }

    /// Up:向更旧的一条移动。
    ///
    /// `current_text` 是 composer 当前内容 —— 首次进入浏览时存为草稿,
    /// 这样用户向下越过最新一条后能拿回自己正在写的东西。
    /// 已在最旧一条时返回 `None`(调用方应给个「已到最早」的反馈或不动作)。
    pub fn navigate_up(&mut self, current_text: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        match self.cursor {
            None => {
                self.draft = Some(current_text.to_string());
                let last = self.entries.len() - 1;
                self.cursor = Some(last);
                Some(self.entries[last].clone())
            }
            Some(0) => None, // 已到最旧
            Some(i) => {
                let next = i - 1;
                self.cursor = Some(next);
                Some(self.entries[next].clone())
            }
        }
    }

    /// Down:向更新的一条移动;越过最新一条时还原草稿并退出浏览。
    pub fn navigate_down(&mut self) -> Option<String> {
        let i = self.cursor?;
        if i + 1 < self.entries.len() {
            let next = i + 1;
            self.cursor = Some(next);
            Some(self.entries[next].clone())
        } else {
            // 越过最新一条 → 还原草稿,退出浏览。
            let draft = self.draft.take().unwrap_or_default();
            self.cursor = None;
            Some(draft)
        }
    }

    /// 记录一条已提交的提示词并落盘。
    ///
    /// 跳过:空串、纯空白、超长、与最新一条完全相同(连续重复提交同一条只记一次)。
    /// 落盘失败只 warn —— 丢历史不该让回合失败。
    pub fn record(&mut self, path: &Path, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_ENTRY_BYTES {
            return;
        }
        if self.entries.last().map(String::as_str) == Some(trimmed) {
            return;
        }
        self.entries.push(trimmed.to_string());
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..excess);
        }
        self.reset_cursor();
        if let Err(e) = persist(path, &self.entries) {
            tracing::warn!(error = %e, path = %path.display(), "prompt 历史落盘失败");
        }
    }
}

/// 历史文件路径:`<reflect home>/prompt_history.jsonl`。
pub fn default_path() -> Option<PathBuf> {
    crate::utils_home_dir::find_reflect_home().map(|h| h.join("prompt_history.jsonl"))
}

/// 解析 JSONL:每行一个 JSON 字符串。坏行跳过(不因一行损坏丢全部)。
fn parse_entries(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<String>(line) {
            Ok(s) if !s.trim().is_empty() && s.len() <= MAX_ENTRY_BYTES => out.push(s),
            _ => continue,
        }
    }
    if out.len() > MAX_ENTRIES {
        out.drain(..out.len() - MAX_ENTRIES);
    }
    out
}

/// 原子落盘:同目录 temp + rename。直接写原文件会在崩溃时截断成空,
/// 让用户丢掉**全部**历史而不是最后一条。
fn persist(path: &Path, entries: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut body = String::with_capacity(entries.len() * 64);
    for e in entries {
        // serde_json::to_string 对 String 产出带引号的 JSON 字符串字面量,
        // 换行/引号都会被转义,所以一行一条是安全的。
        body.push_str(&serde_json::to_string(e).unwrap_or_else(|_| "\"\"".to_string()));
        body.push('\n');
    }
    // temp 名带 pid,避免同机多实例并发写互相覆盖。
    let tmp = path.with_extension(format!("jsonl.tmp.{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 反向历史搜索(Ctrl+R)的纯逻辑状态机。
///
/// 与 `PromptHistory` 的浏览游标分开:搜索有自己的 query 与匹配游标,
/// 且 Enter 接受 / Esc 取消的语义不同。
#[derive(Debug, Clone)]
pub struct HistorySearch {
    /// 当前查询(大小写不敏感子串匹配)。
    pub query: String,
    /// 当前匹配在 `entries` 中的下标;`None` = 查询无匹配。
    pub match_idx: Option<usize>,
    /// 进入搜索时的编辑器草稿,Esc 时还原。
    pub original_draft: String,
}

/// 搜索动作的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchStep {
    /// 查询/游标变了,调用方应把 `text` 灌进编辑器。
    Matched(String),
    /// 该方向没有更多匹配,查询与游标不变。
    NoMatch,
}

impl HistorySearch {
    pub fn new(original_draft: String) -> Self {
        Self {
            query: String::new(),
            match_idx: None,
            original_draft,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// 追加一个字符到查询并重新匹配(从最新一条往下找)。
    pub fn push_char(&mut self, c: char, entries: &[String]) -> SearchStep {
        self.query.push(c);
        self.rematch_from_newest(entries)
    }

    /// 退格一个字符并重新匹配。
    pub fn pop_char(&mut self, entries: &[String]) -> SearchStep {
        self.query.pop();
        self.rematch_from_newest(entries)
    }

    /// 重复 Ctrl+R / Up:向更旧的匹配移动。
    pub fn older(&mut self, entries: &[String]) -> SearchStep {
        // 从当前匹配的下一条开始往下找;无当前匹配时从最新开始。
        let start = match self.match_idx {
            Some(i) if i > 0 => i - 1,
            Some(_) => return SearchStep::NoMatch, // 已在最旧匹配
            None => entries.len().saturating_sub(1),
        };
        match find_match(entries, &self.query, start) {
            Some(i) => {
                self.match_idx = Some(i);
                SearchStep::Matched(entries[i].clone())
            }
            None => SearchStep::NoMatch,
        }
    }

    /// Ctrl+S / Down:向更新的匹配移动。
    pub fn newer(&mut self, entries: &[String]) -> SearchStep {
        let Some(i) = self.match_idx else {
            return SearchStep::NoMatch;
        };
        if i + 1 >= entries.len() {
            return SearchStep::NoMatch;
        }
        match find_match_forward(entries, &self.query, i + 1) {
            Some(j) => {
                self.match_idx = Some(j);
                SearchStep::Matched(entries[j].clone())
            }
            None => SearchStep::NoMatch,
        }
    }

    fn rematch_from_newest(&mut self, entries: &[String]) -> SearchStep {
        match find_match(entries, &self.query, entries.len().saturating_sub(1)) {
            Some(i) => {
                self.match_idx = Some(i);
                SearchStep::Matched(entries[i].clone())
            }
            None => {
                self.match_idx = None;
                SearchStep::NoMatch
            }
        }
    }
}

/// 从 `from` 往下(下标递减)找第一条包含 query 的条目。空 query 匹配一切。
fn find_match(entries: &[String], query: &str, from: usize) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    let q = query.to_lowercase();
    (0..=from.min(entries.len() - 1))
        .rev()
        .find(|&i| entries[i].to_lowercase().contains(&q))
}

/// 从 `from` 往上(下标递增)找第一条包含 query 的条目。
fn find_match_forward(entries: &[String], query: &str, from: usize) -> Option<usize> {
    let q = query.to_lowercase();
    (from..entries.len()).find(|&i| entries[i].to_lowercase().contains(&q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reflect-prompt-history-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("prompt_history.jsonl")
    }

    #[test]
    fn navigate_up_then_down_round_trips_draft() {
        let mut h = PromptHistory::default();
        let p = tmpdir("roundtrip");
        h.record(&p, "first");
        h.record(&p, "second");

        // 首次 Up:存草稿,跳到最新一条。
        assert_eq!(h.navigate_up("my draft").as_deref(), Some("second"));
        assert!(h.is_browsing());
        // 继续 Up:更旧。
        assert_eq!(h.navigate_up("ignored").as_deref(), Some("first"));
        // 已到最旧。
        assert_eq!(h.navigate_up("ignored"), None);
        // Down 回到 second。
        assert_eq!(h.navigate_down().as_deref(), Some("second"));
        // 越过最新一条 → 还原草稿并退出浏览。
        assert_eq!(h.navigate_down().as_deref(), Some("my draft"));
        assert!(!h.is_browsing());
        // 退出浏览后 Down 无操作。
        assert_eq!(h.navigate_down(), None);
    }

    #[test]
    fn empty_history_navigate_up_is_none() {
        let mut h = PromptHistory::default();
        assert_eq!(h.navigate_up("draft"), None);
        assert!(!h.is_browsing(), "空历史不得进入浏览态");
    }

    #[test]
    fn record_skips_empty_blank_duplicate_and_oversized() {
        let mut h = PromptHistory::default();
        let p = tmpdir("skip");
        h.record(&p, "");
        h.record(&p, "   \n\t ");
        assert!(h.is_empty(), "空/纯空白不入历史");

        h.record(&p, "hello");
        h.record(&p, "hello");
        assert_eq!(h.entries().len(), 1, "连续重复只记一次");

        let huge = "x".repeat(MAX_ENTRY_BYTES + 1);
        h.record(&p, &huge);
        assert_eq!(h.entries().len(), 1, "超长不入历史");
    }

    #[test]
    fn record_caps_at_max_entries_dropping_oldest() {
        let mut h = PromptHistory::default();
        let p = tmpdir("cap");
        for i in 0..MAX_ENTRIES + 20 {
            h.record(&p, &format!("p{i}"));
        }
        assert_eq!(h.entries().len(), MAX_ENTRIES);
        assert_eq!(h.entries()[0], "p20", "最旧的应被丢掉");
        assert_eq!(
            h.entries()[MAX_ENTRIES - 1],
            format!("p{}", MAX_ENTRIES + 19)
        );
    }

    #[test]
    fn persist_and_load_round_trip_is_atomic_and_jsonl() {
        let mut h = PromptHistory::default();
        let p = tmpdir("persist");
        h.record(&p, "line one");
        h.record(&p, "has \"quotes\" and\nnewline");
        h.record(&p, "中文提示词 🎉");

        let loaded = PromptHistory::load(&p);
        assert_eq!(loaded.entries(), h.entries());
        assert_eq!(loaded.entries().len(), 3);

        // 落盘后不得留下 temp 文件。
        let leftovers: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp 文件应已被 rename 消费: {leftovers:?}"
        );
    }

    #[test]
    fn load_tolerates_missing_and_corrupt_file() {
        // 历史是增强而非启动前提:坏文件必须退化为空,不能让 TUI 起不来。
        let missing = tmpdir("missing").join("nope.jsonl");
        assert!(PromptHistory::load(&missing).is_empty());

        let p = tmpdir("corrupt");
        std::fs::write(&p, "not json\n\n\"ok\"\n{\"broken\":\n").unwrap();
        let loaded = PromptHistory::load(&p);
        assert_eq!(loaded.entries(), &["ok".to_string()], "坏行跳过,好行保留");
    }

    #[test]
    fn reset_cursor_exits_browsing() {
        let mut h = PromptHistory::default();
        let p = tmpdir("reset");
        h.record(&p, "a");
        h.navigate_up("draft");
        assert!(h.is_browsing());
        h.reset_cursor();
        assert!(!h.is_browsing());
        // 复位后再次 Up 重新从最新一条开始,并重新保存草稿。
        assert_eq!(h.navigate_up("new draft").as_deref(), Some("a"));
        assert_eq!(h.navigate_down().as_deref(), Some("new draft"));
    }

    fn sample() -> Vec<String> {
        vec![
            "fix the parser".into(),
            "add tests for parser".into(),
            "refactor the lexer".into(),
            "write docs".into(),
        ]
    }

    #[test]
    fn search_matches_newest_first_and_walks_older() {
        let e = sample();
        let mut s = HistorySearch::new("draft".into());
        // 输入 "par" → 最新匹配是 "add tests for parser"(下标 1)。
        assert_eq!(
            s.push_char('p', &e),
            SearchStep::Matched("add tests for parser".into())
        );
        assert_eq!(
            s.push_char('a', &e),
            SearchStep::Matched("add tests for parser".into())
        );
        assert_eq!(
            s.push_char('r', &e),
            SearchStep::Matched("add tests for parser".into())
        );
        assert_eq!(s.match_idx, Some(1));
        // 更旧 → "fix the parser"(下标 0)。
        assert_eq!(s.older(&e), SearchStep::Matched("fix the parser".into()));
        assert_eq!(s.match_idx, Some(0));
        // 已到最旧匹配。
        assert_eq!(s.older(&e), SearchStep::NoMatch);
        // 更新 → 回到下标 1。
        assert_eq!(
            s.newer(&e),
            SearchStep::Matched("add tests for parser".into())
        );
    }

    #[test]
    fn search_is_case_insensitive_substring() {
        let e = vec!["Fix The PARSER".to_string()];
        let mut s = HistorySearch::new(String::new());
        assert_eq!(
            s.push_char('p', &e),
            SearchStep::Matched("Fix The PARSER".to_string())
        );
    }

    #[test]
    fn search_no_match_keeps_state_and_reports_nomatch() {
        let e = sample();
        let mut s = HistorySearch::new("draft".into());
        assert_eq!(s.push_char('z', &e), SearchStep::NoMatch);
        assert_eq!(s.match_idx, None, "无匹配不得推进游标");
        assert_eq!(s.query(), "z");
        // 退格回到空 query → 匹配最新一条。
        assert_eq!(s.pop_char(&e), SearchStep::Matched("write docs".into()));
    }

    #[test]
    fn search_on_empty_history_is_nomatch() {
        let mut s = HistorySearch::new(String::new());
        assert_eq!(s.push_char('a', &[]), SearchStep::NoMatch);
        assert_eq!(s.older(&[]), SearchStep::NoMatch);
        assert_eq!(s.newer(&[]), SearchStep::NoMatch);
    }

    #[test]
    fn search_preserves_original_draft_for_cancel() {
        let e = sample();
        let mut s = HistorySearch::new("my wip draft".into());
        s.push_char('d', &e);
        assert_eq!(s.original_draft, "my wip draft", "Esc 取消时必须能还原草稿");
    }
}
