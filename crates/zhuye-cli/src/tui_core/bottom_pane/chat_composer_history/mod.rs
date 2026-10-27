//! 聊天编辑器历史。

//! 聊天编辑器历史模块负责实现类 shell 的历史回忆与增量搜索遍历。
//!
//! 它将持久化的跨会话条目与当前会话内的本地条目合并到同一个偏移量空间中。普通导航通过
//! [`ChatComposerHistory::on_entry_response`] 逐个获取持久化条目。反向搜索在探测最新条目后，
//! 通过 [`ChatComposerHistory::on_batch_response`] 切换为有界的、与查询无关的批量加载。即使当前
//! 搜索已经移动，批量响应也会填充共享缓存，但只有被等待的游标才能恢复搜索；过期的日志 ID 会被
//! 忽略，批量读取失败则遵循有界的重试路径。本地条目已经带有完整的草稿元数据，可直接使用。
//!
//! Ctrl+R 搜索与普通的 Up/Down 导航分开建模，因为二者的保证不同：查询编辑时会从最新的匹配项
//! 重新开始，反复按 Older/Newer 键会在去重后的匹配文本中移动，待处理的持久化获取在响应到达后
//! 继续同一扫描，并且命中边界时不得推进隐藏的游标状态。搜索去重的范围仅限于单个活动搜索会话，
//! 并且基于精确的提示文本；它不会修改已存储的历史记录，也不会改变普通的历史浏览。
use std::collections::HashMap;
use std::collections::HashSet;

use crate::message_history::HistoryBatchCursor;
use crate::protocol_compat::ThreadId;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event_sender::AppEventSender;

#[path = "search_batch.rs"]
mod search_batch;
#[cfg(test)]
#[path = "search_batch_tests.rs"]
mod search_batch_tests;

const MAX_BATCH_READ_RETRIES: u8 = 2;

// ── 历史条目类型（外移子模块） ──
mod history_entry;
pub(crate) use history_entry::*;

/// 管理聊天编辑器内类 shell 历史导航（Up/Down）的状态机。此结构体有意与渲染控件解耦，
/// 以便逻辑保持独立、更易于测试。
pub(crate) struct ChatComposerHistory {
    /// 持有该元数据快照对应的持久化查询响应的线程。
    thread_id: Option<ThreadId>,
    /// 持久化历史日志的标识符，用于拒绝已过期的查询响应。
    persistent_log_id: Option<u64>,
    /// 会话开始时，持久化跨会话历史文件中已存在的条目数量。
    persistent_entry_count: usize,

    /// 用户在本 UI 会话期间提交的消息（最新的位于末尾）。
    /// 本地条目保留完整的草稿状态（文本元素、图片路径、待粘贴内容、远程图片 URL）。
    local_history: Vec<HistoryEntry>,
    /// 由恢复后的转录重放所填充的本地条目。
    replay_seeded_history: Vec<HistoryEntry>,

    /// 按需获取的持久化偏移量，格式错误的批量行对应 `None`。
    fetched_history: HashMap<usize, Option<HistoryEntry>>,

    /// 合并（持久化 + 本地）历史中的当前游标。`None` 表示用户当前*没有*在浏览历史。
    history_cursor: Option<isize>,
    pending_navigation_direction: Option<HistorySearchDirection>,

    /// 上次因历史导航而插入编辑器中的文本。与 [`Self::should_handle_navigation`] 中的
    /// "光标是否位于行边界"检查一起，用于决定后续的 Up/Down 按键应视为导航还是普通光标移动。
    last_history_text: Option<String>,

    /// 当前活动的增量历史搜索，如果 Ctrl+R 搜索模式已开启。
    search: Option<HistorySearchState>,
    /// 持久化历史恢复时是否应重新填充 `@` 工具提及（mention）。
    at_mention_restore_enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistorySearchDirection {
    /// 向更旧的历史偏移量方向遍历。
    Older,
    /// 向更新的历史偏移量方向遍历。
    Newer,
}

/// 单步增量历史搜索的结果。
///
/// `Pending` 表示已请求持久化条目的查询，调用方应保持可见的搜索会话开启，直到
/// [`ChatComposerHistory::on_entry_response`] 提供下一个结果。`AtBoundary` 表示当前选中的
/// 匹配仍然有效，但所请求的方向没有更多的唯一匹配；调用方不应将其视为查询未命中。`Unavailable`
/// 结束一次失败的查询，但不声称查询在历史中没有匹配项。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HistorySearchResult {
    Found(HistoryEntry),
    Pending,
    AtBoundary,
    NotFound,
    Unavailable,
}

/// 整合异步持久化历史响应的结果。
///
/// 一个响应可以满足普通的 Up/Down 导航、恢复待处理的 Ctrl+R 搜索扫描，或者在响应属于过期的日志或编辑器不再需要的偏移量时被忽略。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HistoryEntryResponse {
    Found(HistoryEntry),
    Search(HistorySearchResult),
    Ignored,
}

/// 单个活动的 Ctrl+R 搜索查询的状态。
///
/// 该状态维护两个游标：`selected_offset` 是用于继续扫描的原始合并历史偏移量，而
/// `selected_match_index` 指向 `unique_matches`，因此已发现的唯一结果可以无需重新扫描重复偏移量
/// 即可再次访问。`seen_texts` 特意以精确的提示文本作为键，因为 UI 预览和接受的是文本，而非每条
/// 历史记录的存储标识。`next_older_cursor` 保留与查询无关的批量边界，这样一次匹配不会迫使下一次
/// Older 搜索回到前缀扫描路径。
// ── 搜索状态类型（外移子模块） ──
mod search_state;
use search_state::*;

impl ChatComposerHistory {
    /// 创建一个不含持久化元数据的空历史状态机。
    ///
    /// 调用方必须先提供会话元数据才能获取跨会话历史，但当前会话内的本地条目仍然可以记录和遍历。
    /// 保持构造过程轻量且无需元数据，可以让编辑器在会话生命周期之间重置并复用此辅助结构。
    pub fn new() -> Self {
        Self {
            thread_id: None,
            persistent_log_id: None,
            persistent_entry_count: 0,
            local_history: Vec::new(),
            replay_seeded_history: Vec::new(),
            fetched_history: HashMap::new(),
            history_cursor: None,
            pending_navigation_direction: None,
            last_history_text: None,
            search: None,
            at_mention_restore_enabled: false,
        }
    }

    pub fn set_at_mention_restore_enabled(&mut self, enabled: bool) {
        if self.at_mention_restore_enabled == enabled {
            return;
        }
        self.at_mention_restore_enabled = enabled;
        self.fetched_history.clear();
        self.history_cursor = None;
        self.last_history_text = None;
        self.search = None;
    }

    /// 配置新会话时更新持久化历史元数据。
    ///
    /// 这会清除已获取的条目、本地条目、导航游标和活动搜索状态，因为偏移量只在同一个历史日志快照内才有意义。日志 ID 变化后复用旧偏移量，会让过期的异步响应填充到错误的提示中。
    pub fn set_metadata(&mut self, thread_id: ThreadId, log_id: u64, entry_count: usize) {
        self.thread_id = Some(thread_id);
        self.persistent_log_id = Some(log_id);
        self.persistent_entry_count = entry_count;
        self.fetched_history.clear();
        self.local_history.clear();
        self.replay_seeded_history.clear();
        self.history_cursor = None;
        self.pending_navigation_direction = None;
        self.last_history_text = None;
        self.search = None;
    }

    /// 记录当前会话的提交，以便日后能以完整的草稿元数据进行回忆。
    ///
    /// 空提交会被忽略，相邻的重复内容会被合并；由于新的最新条目会改变合并历史的偏移量空间，
    /// 活动的导航或搜索状态会被重置。
    pub fn record_local_submission(&mut self, entry: HistoryEntry) {
        self.record_local_submission_inner(entry);
    }

    pub fn record_replayed_submission(&mut self, entry: HistoryEntry) {
        if self.record_local_submission_inner(entry.clone()) {
            self.replay_seeded_history.push(entry);
        }
    }

    fn record_local_submission_inner(&mut self, entry: HistoryEntry) -> bool {
        if entry.text.is_empty()
            && entry.text_elements.is_empty()
            && entry.local_image_paths.is_empty()
            && entry.remote_image_urls.is_empty()
            && entry.mention_bindings.is_empty()
            && entry.pending_pastes.is_empty()
        {
            return false;
        }
        self.history_cursor = None;
        self.pending_navigation_direction = None;
        self.last_history_text = None;
        self.search = None;

        // 若与前一条目相同，则避免插入重复项。
        if self.local_history.last().is_some_and(|prev| prev == &entry) {
            return false;
        }

        self.local_history.push(entry);
        true
    }

    /// 重置普通历史导航，以便下一次按 Up 键从最新条目开始。
    ///
    /// 这同时会清除任何活动的增量搜索，因为普通浏览与 Ctrl+R 搜索维护不同的游标语义。此处若不清除搜索，旧查询会影响之后的 Up/Down 回忆。
    pub fn reset_navigation(&mut self) {
        self.history_cursor = None;
        self.pending_navigation_direction = None;
        self.last_history_text = None;
        self.search = None;
    }

    /// 仅清除活动的增量搜索状态。
    ///
    /// 普通的 Up/Down 导航游标和已缓存的持久化条目保持不变。编辑器搜索模式在接受匹配或返回到空查询时调用此方法，以便下一次搜索从全新的去重结果缓存开始。
    pub fn reset_search(&mut self) {
        self.search = None;
    }

    /// 返回对于当前文本域状态，Up/Down 是否应导航历史。
    ///
    /// 空文本始终允许历史遍历。对于非空文本，需要同时满足以下两个条件：
    ///
    /// - 当前文本与最后一次回忆的历史条目完全一致，并且
    /// - 光标位于行边界（行首或行尾）。
    ///
    /// 这一边界限制在保留类 shell 历史回忆的同时，保证多行光标移动依然可用。如果调用方把光标移入
    /// 已回忆条目的中间仍强制导航，用户将无法在草稿中进行正常的纵向移动。
    pub fn should_handle_navigation(&self, text: &str, cursor: usize) -> bool {
        if self.persistent_entry_count == 0 && self.local_history.is_empty() {
            return false;
        }

        if text.is_empty() {
            return true;
        }

        // 文本域非空——仅当文本与最后一次回忆的历史条目一致且光标位于行边界时才导航。
        // 这可以保持类 shell 的 Up/Down 回忆，同时仍允许从中间位置进行普通的多行光标移动。
        if cursor != 0 && cursor != text.len() {
            return false;
        }

        matches!(&self.last_history_text, Some(prev) if prev == text)
    }

    /// 处理 Up 键，在合并历史空间中向更旧的条目移动。
    ///
    /// 本地条目可以立即返回，而缺失的持久化条目会发送 `LookupMessageHistoryEntry` 并在响应到达前返回 `None`。在 Ctrl+R 搜索激活期间调用此方法会有意退出搜索遍历。
    pub fn navigate_up(&mut self, app_event_tx: &AppEventSender) -> Option<HistoryEntry> {
        self.search = None;
        let total_entries = self.persistent_entry_count + self.local_history.len();
        if total_entries == 0 {
            return None;
        }

        let next_idx = match self.history_cursor {
            None => (total_entries as isize) - 1,
            Some(0) => return None, // 已在最旧位置
            Some(idx) => idx - 1,
        };

        self.history_cursor = Some(next_idx);
        self.populate_history_at_index(
            next_idx as usize,
            HistorySearchDirection::Older,
            app_event_tx,
        )
    }

    /// 处理 Down 键，向更新的条目移动，或在越过最新条目后清空编辑器。
    ///
    /// 返回空的 `HistoryEntry` 表示用户已越过最新的已知条目，调用方应清空编辑器草稿。与 Up 一样，在 Ctrl+R 搜索期间调用此方法会清除搜索状态并恢复普通类 shell 浏览。
    pub fn navigate_down(&mut self, app_event_tx: &AppEventSender) -> Option<HistoryEntry> {
        self.search = None;
        let total_entries = self.persistent_entry_count + self.local_history.len();
        if total_entries == 0 {
            return None;
        }

        let next_idx_opt = match self.history_cursor {
            None => return None, // 不在浏览状态
            Some(idx) if (idx as usize) + 1 >= total_entries => None,
            Some(idx) => Some(idx + 1),
        };

        match next_idx_opt {
            Some(idx) => {
                self.history_cursor = Some(idx);
                self.populate_history_at_index(
                    idx as usize,
                    HistorySearchDirection::Newer,
                    app_event_tx,
                )
            }
            None => {
                // 已越过最新条目——清空并退出浏览模式。
                self.history_cursor = None;
                self.pending_navigation_direction = None;
                self.last_history_text = None;
                Some(HistoryEntry::new(String::new()))
            }
        }
    }

    /// 将持久化历史条目响应整合到导航或活动搜索中。
    ///
    /// 带有过期日志 ID 的响应会被忽略，匹配的响应会更新持久化缓存，待处理的 Ctrl+R 搜索会从返回的偏移量处恢复扫描。调用方应将 `HistoryEntryResponse::Search` 路由回编辑器搜索会话，而不是普通的历史回忆；否则异步搜索命中可能会在未更新底部状态或匹配高亮的情况下被接受。
    pub fn on_entry_response(
        &mut self,
        log_id: u64,
        offset: usize,
        entry: Option<String>,
        app_event_tx: &AppEventSender,
    ) -> HistoryEntryResponse {
        if self.persistent_log_id != Some(log_id) {
            return HistoryEntryResponse::Ignored;
        }

        let entry = entry.map(|entry| {
            HistoryEntry::new_with_at_mentions(entry, self.at_mention_restore_enabled)
        });
        if let Some(entry) = entry.clone() {
            self.fetched_history.insert(offset, Some(entry));
        }

        if self
            .search
            .as_ref()
            .and_then(|search| search.awaiting.clone())
            .is_some_and(|pending| {
                matches!(pending, PendingHistorySearch::Entry { offset: awaited, .. } if awaited == offset)
            })
        {
            let pending = self
                .search
                .as_ref()
                .and_then(|search| search.awaiting.clone())
                .unwrap_or(PendingHistorySearch::Entry {
                    offset,
                    direction: HistorySearchDirection::Older,
                    boundary_if_exhausted: false,
                });
            let PendingHistorySearch::Entry {
                direction,
                boundary_if_exhausted,
                ..
            } = pending
            else {
                return HistoryEntryResponse::Ignored;
            };
            if let Some(entry) = entry
                && self.search_matches(&entry)
                && self.search_result_is_unique(&entry)
            {
                return HistoryEntryResponse::Search(self.search_match(offset, entry));
            }
            let result = match direction {
                HistorySearchDirection::Older => self.advance_older_search_after_entry_miss(
                    offset,
                    boundary_if_exhausted,
                    app_event_tx,
                ),
                HistorySearchDirection::Newer => self.advance_search_after(
                    offset,
                    direction,
                    boundary_if_exhausted,
                    app_event_tx,
                ),
            };
            return HistoryEntryResponse::Search(result);
        }

        if self.history_cursor == Some(offset as isize) {
            let direction = self.pending_navigation_direction.take();
            let Some(entry) = entry else {
                return HistoryEntryResponse::Ignored;
            };
            if self.persistent_entry_duplicates_local(&entry)
                && let Some(direction) = direction
            {
                let Some(offset) = self.next_history_offset(offset, direction) else {
                    return HistoryEntryResponse::Ignored;
                };
                self.history_cursor = Some(offset as isize);
                return self
                    .populate_history_at_index(offset, direction, app_event_tx)
                    .map(HistoryEntryResponse::Found)
                    .unwrap_or(HistoryEntryResponse::Ignored);
            }
            self.last_history_text = Some(entry.text.clone());
            return HistoryEntryResponse::Found(entry);
        }

        HistoryEntryResponse::Ignored
    }

    /// 推进活动的 Ctrl+R 搜索并返回下一个可见的搜索状态。
    ///
    /// 调用方在打开搜索或编辑查询后传入 `restart`；这会清除去重匹配缓存，并从合并历史的末尾开始。使用相同查询且 `restart == false` 的重复调用会相对于当前唯一匹配移动，并在边界处保留所选条目。当上一次持久化查询仍待处理时调用此方法会持续返回 `Pending`；否则过期的响应可能与更新的用户操作竞争，并用意外的条目替换编辑器内容。
    pub fn search(
        &mut self,
        query: &str,
        direction: HistorySearchDirection,
        restart: bool,
        app_event_tx: &AppEventSender,
    ) -> HistorySearchResult {
        let total_entries = self.total_entries();
        if total_entries == 0 {
            self.search = Some(HistorySearchState::new(query));
            return HistorySearchResult::NotFound;
        }

        let query_changed = self
            .search
            .as_ref()
            .is_none_or(|search| search.query != query);
        if !query_changed
            && !restart
            && self
                .search
                .as_ref()
                .and_then(|search| search.awaiting.clone())
                .is_some()
        {
            return HistorySearchResult::Pending;
        }

        if query_changed || restart || self.search.is_none() {
            self.search = Some(HistorySearchState::new(query));
        } else if let Some(search) = self.search.as_mut() {
            search.awaiting = None;
        }

        let boundary_if_exhausted = !restart
            && self
                .search
                .as_ref()
                .and_then(|search| search.selected_offset)
                .is_some();
        if !restart
            && !query_changed
            && let Some(result) = self.select_cached_unique_match(direction)
        {
            return result;
        }
        if boundary_if_exhausted
            && self
                .search
                .as_ref()
                .is_some_and(|search| search.is_exhausted(direction))
        {
            return HistorySearchResult::AtBoundary;
        }

        let start_offset =
            self.search_start_offset(total_entries, direction, query_changed || restart);
        let Some(start_offset) = start_offset else {
            return self.exhausted_search_result(direction, boundary_if_exhausted);
        };

        let result =
            self.advance_search_from(start_offset, direction, boundary_if_exhausted, app_event_tx);
        if matches!(result, HistorySearchResult::NotFound) {
            self.exhausted_search_result(direction, boundary_if_exhausted)
        } else {
            result
        }
    }

    // ---------------------------------------------------------------------
    // 内部辅助函数
    // ---------------------------------------------------------------------

    fn total_entries(&self) -> usize {
        self.persistent_entry_count + self.local_history.len()
    }

    fn search_start_offset(
        &self,
        total_entries: usize,
        direction: HistorySearchDirection,
        restart: bool,
    ) -> Option<usize> {
        let selected = self
            .search
            .as_ref()
            .and_then(|search| search.selected_offset);
        match direction {
            HistorySearchDirection::Older => {
                if restart {
                    total_entries.checked_sub(1)
                } else {
                    selected.and_then(|offset| offset.checked_sub(1))
                }
            }
            HistorySearchDirection::Newer => {
                if restart {
                    Some(0)
                } else {
                    selected
                        .and_then(|offset| offset.checked_add(1))
                        .filter(|offset| *offset < total_entries)
                }
            }
        }
    }

    fn advance_search_after(
        &mut self,
        offset: usize,
        direction: HistorySearchDirection,
        boundary_if_exhausted: bool,
        app_event_tx: &AppEventSender,
    ) -> HistorySearchResult {
        let next_offset = match direction {
            HistorySearchDirection::Older => offset.checked_sub(1),
            HistorySearchDirection::Newer => offset
                .checked_add(1)
                .filter(|next| *next < self.total_entries()),
        };
        let Some(next_offset) = next_offset else {
            return self.exhausted_search_result(direction, boundary_if_exhausted);
        };
        let result =
            self.advance_search_from(next_offset, direction, boundary_if_exhausted, app_event_tx);
        if matches!(result, HistorySearchResult::NotFound) {
            self.exhausted_search_result(direction, boundary_if_exhausted)
        } else {
            result
        }
    }

    fn advance_search_from(
        &mut self,
        mut offset: usize,
        direction: HistorySearchDirection,
        boundary_if_exhausted: bool,
        app_event_tx: &AppEventSender,
    ) -> HistorySearchResult {
        let total_entries = self.total_entries();
        while offset < total_entries {
            if let Some(entry) = self.entry_at_cached_offset(offset) {
                if self.search_matches(&entry) && self.search_result_is_unique(&entry) {
                    return self.search_match(offset, entry);
                }
            } else if !self.fetched_history.contains_key(&offset)
                && offset < self.persistent_entry_count
            {
                if direction == HistorySearchDirection::Older
                    && let Some(cursor) = self
                        .search
                        .as_ref()
                        .and_then(|search| search.next_older_cursor)
                    && cursor.end_offset() == offset
                {
                    return self.request_older_search_batch(
                        cursor,
                        boundary_if_exhausted,
                        app_event_tx,
                    );
                }
                if let (Some(thread_id), Some(log_id)) = (self.thread_id, self.persistent_log_id) {
                    if let Some(search) = self.search.as_mut() {
                        search.awaiting = Some(PendingHistorySearch::Entry {
                            offset,
                            direction,
                            boundary_if_exhausted,
                        });
                    }
                    app_event_tx.send(AppEvent::LookupMessageHistoryEntry {
                        thread_id,
                        offset,
                        log_id,
                    });
                    return HistorySearchResult::Pending;
                }
            }

            let next_offset = match direction {
                HistorySearchDirection::Older => offset.checked_sub(1),
                HistorySearchDirection::Newer => {
                    offset.checked_add(1).filter(|next| *next < total_entries)
                }
            };
            let Some(next_offset) = next_offset else {
                return HistorySearchResult::NotFound;
            };
            offset = next_offset;
        }

        HistorySearchResult::NotFound
    }

    fn entry_at_cached_offset(&self, offset: usize) -> Option<HistoryEntry> {
        if offset >= self.persistent_entry_count {
            self.local_history
                .get(offset - self.persistent_entry_count)
                .cloned()
        } else {
            self.fetched_history.get(&offset).cloned().flatten()
        }
    }

    fn search_matches(&self, entry: &HistoryEntry) -> bool {
        let Some(search) = self.search.as_ref() else {
            return false;
        };
        search.query.is_empty() || entry.text.to_lowercase().contains(&search.query_lower)
    }

    fn search_result_is_unique(&self, entry: &HistoryEntry) -> bool {
        self.search
            .as_ref()
            .is_none_or(|search| !search.seen_texts.contains(entry.text.as_str()))
    }

    fn search_match(&mut self, offset: usize, entry: HistoryEntry) -> HistorySearchResult {
        self.history_cursor = Some(offset as isize);
        self.last_history_text = Some(entry.text.clone());
        if let Some(search) = self.search.as_mut() {
            search.selected_offset = Some(offset);
            search.record_match(offset, &entry);
            search.awaiting = None;
            search.exhausted_older = false;
            search.exhausted_newer = false;
        }
        HistorySearchResult::Found(entry)
    }

    fn select_cached_unique_match(
        &mut self,
        direction: HistorySearchDirection,
    ) -> Option<HistorySearchResult> {
        let next_index = {
            let search = self.search.as_ref()?;
            let selected_index = search.selected_match_index?;
            match direction {
                HistorySearchDirection::Older => {
                    let next_index = selected_index + 1;
                    (next_index < search.unique_matches.len()).then_some(next_index)?
                }
                HistorySearchDirection::Newer => selected_index.checked_sub(1)?,
            }
        };

        let history_match = self.search.as_ref()?.unique_matches[next_index].clone();
        self.history_cursor = Some(history_match.offset as isize);
        self.last_history_text = Some(history_match.entry.text.clone());
        if let Some(search) = self.search.as_mut() {
            search.select_match(next_index);
        }
        Some(HistorySearchResult::Found(history_match.entry))
    }

    fn exhausted_search_result(
        &mut self,
        direction: HistorySearchDirection,
        boundary_if_exhausted: bool,
    ) -> HistorySearchResult {
        if let Some(search) = self.search.as_mut() {
            search.awaiting = None;
            if boundary_if_exhausted {
                search.mark_exhausted(direction);
            }
        }

        if boundary_if_exhausted {
            HistorySearchResult::AtBoundary
        } else {
            HistorySearchResult::NotFound
        }
    }

    fn populate_history_at_index(
        &mut self,
        global_idx: usize,
        direction: HistorySearchDirection,
        app_event_tx: &AppEventSender,
    ) -> Option<HistoryEntry> {
        let mut global_idx = global_idx;
        loop {
            if let Some(entry) = self.entry_at_cached_offset(global_idx) {
                if global_idx < self.persistent_entry_count
                    && self.persistent_entry_duplicates_local(&entry)
                {
                    let Some(next_idx) = self.next_history_offset(global_idx, direction) else {
                        self.pending_navigation_direction = None;
                        return None;
                    };
                    self.history_cursor = Some(next_idx as isize);
                    global_idx = next_idx;
                    continue;
                }
                self.pending_navigation_direction = None;
                self.last_history_text = Some(entry.text.clone());
                return Some(entry);
            }

            if global_idx >= self.persistent_entry_count {
                return None;
            }

            if let (Some(thread_id), Some(log_id)) = (self.thread_id, self.persistent_log_id) {
                self.pending_navigation_direction = Some(direction);
                app_event_tx.send(AppEvent::LookupMessageHistoryEntry {
                    thread_id,
                    offset: global_idx,
                    log_id,
                });
            }
            return None;
        }
    }

    fn next_history_offset(
        &self,
        offset: usize,
        direction: HistorySearchDirection,
    ) -> Option<usize> {
        match direction {
            HistorySearchDirection::Older => offset.checked_sub(1),
            HistorySearchDirection::Newer => offset
                .checked_add(1)
                .filter(|next| *next < self.total_entries()),
        }
    }

    fn persistent_entry_duplicates_local(&self, entry: &HistoryEntry) -> bool {
        self.replay_seeded_history.iter().any(|local_entry| {
            local_entry.text == entry.text && local_entry.mention_bindings == entry.mention_bindings
        })
    }
}

#[cfg(test)]
mod tests;
