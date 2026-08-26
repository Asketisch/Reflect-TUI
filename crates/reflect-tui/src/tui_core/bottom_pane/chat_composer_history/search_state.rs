//! 历史搜索状态类型与方法。从 chat_composer_history.rs 抽出。

use super::*;

#[derive(Clone, Debug)]
pub(super) struct HistorySearchState {
    pub(super) query: String,
    pub(super) query_lower: String,
    pub(super) selected_offset: Option<usize>,
    pub(super) unique_matches: Vec<UniqueHistoryMatch>,
    pub(super) selected_match_index: Option<usize>,
    pub(super) seen_texts: HashSet<String>,
    pub(super) awaiting: Option<PendingHistorySearch>,
    pub(super) next_older_cursor: Option<HistoryBatchCursor>,
    pub(super) exhausted_older: bool,
    pub(super) exhausted_newer: bool,
}

/// 缓存了足够草稿状态、可被再次选中的唯一搜索匹配。
///
/// 这些匹配的向量按偏移量从新到旧排列。在条目旁保存 `entry`，可避免用户在新/旧匹配之间移动时
/// 依赖后续的缓存查找。
#[derive(Clone, Debug)]
pub(super) struct UniqueHistoryMatch {
    pub(super) offset: usize,
    pub(super) entry: HistoryEntry,
}

/// 当前阻塞增量搜索扫描的持久化历史查询。
///
/// 待处理请求记录了发起获取时生效的边界行为，使响应要么返回唯一的匹配，要么像没有发生异步间隙
/// 那样继续扫描。单条目请求保留其方向；批量请求按构造仅用于更旧方向。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PendingHistorySearch {
    Entry {
        offset: usize,
        direction: HistorySearchDirection,
        boundary_if_exhausted: bool,
    },
    Batch {
        cursor: HistoryBatchCursor,
        boundary_if_exhausted: bool,
        read_failures: u8,
    },
}

impl HistorySearchState {
    pub(super) fn new(query: &str) -> Self {
        Self {
            query: query.to_string(),
            query_lower: query.to_lowercase(),
            selected_offset: None,
            unique_matches: Vec::new(),
            selected_match_index: None,
            seen_texts: HashSet::new(),
            awaiting: None,
            next_older_cursor: None,
            exhausted_older: false,
            exhausted_newer: false,
        }
    }

    pub(super) fn is_exhausted(&self, direction: HistorySearchDirection) -> bool {
        match direction {
            HistorySearchDirection::Older => self.exhausted_older,
            HistorySearchDirection::Newer => self.exhausted_newer,
        }
    }

    pub(super) fn mark_exhausted(&mut self, direction: HistorySearchDirection) {
        match direction {
            HistorySearchDirection::Older => self.exhausted_older = true,
            HistorySearchDirection::Newer => self.exhausted_newer = true,
        }
    }

    pub(super) fn record_match(&mut self, offset: usize, entry: &HistoryEntry) {
        if let Some(index) = self
            .unique_matches
            .iter()
            .position(|history_match| history_match.offset == offset)
        {
            self.select_match(index);
            return;
        }

        self.seen_texts.insert(entry.text.clone());
        let insert_index = self
            .unique_matches
            .partition_point(|history_match| history_match.offset > offset);
        self.unique_matches.insert(
            insert_index,
            UniqueHistoryMatch {
                offset,
                entry: entry.clone(),
            },
        );
        self.select_match(insert_index);
    }

    pub(super) fn select_match(&mut self, index: usize) {
        let Some(history_match) = self.unique_matches.get(index) else {
            return;
        };
        self.selected_offset = Some(history_match.offset);
        self.selected_match_index = Some(index);
        self.awaiting = None;
        self.exhausted_older = false;
        self.exhausted_newer = false;
    }
}
