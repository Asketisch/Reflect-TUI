use super::ChatComposerHistory;
use super::HistoryEntry;
use super::HistorySearchDirection;
use super::HistorySearchResult;
use super::MAX_BATCH_READ_RETRIES;
use super::PendingHistorySearch;
use crate::message_history::HistoryBatchCursor;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event::HistoryBatchEntryResponse;
use crate::tui_core::app_event_sender::AppEventSender;

impl ChatComposerHistory {
    /// 将与查询无关的批量结果应用到持久化缓存中，并在仍适用时恢复活动的反向搜索。
    pub(crate) fn on_batch_response(
        &mut self,
        log_id: u64,
        cursor: HistoryBatchCursor,
        entries: Vec<HistoryBatchEntryResponse>,
        next_older_cursor: Option<HistoryBatchCursor>,
        app_event_tx: &AppEventSender,
    ) -> Option<HistorySearchResult> {
        if self.persistent_log_id != Some(log_id) {
            return None;
        }

        let entries: Vec<_> = entries
            .into_iter()
            .map(|response| {
                let entry = response.entry.map(|text| {
                    HistoryEntry::new_with_at_mentions(text, self.at_mention_restore_enabled)
                });
                if entry.is_some() {
                    self.fetched_history.insert(response.offset, entry.clone());
                } else {
                    self.fetched_history.entry(response.offset).or_insert(None);
                }
                (response.offset, entry)
            })
            .collect();

        let (boundary_if_exhausted, _) = self.pending_batch(cursor)?;
        if let Some(search) = self.search.as_mut() {
            search.next_older_cursor = next_older_cursor;
        }

        for (offset, entry) in entries {
            if let Some(entry) = entry
                && self.search_matches(&entry)
                && self.search_result_is_unique(&entry)
            {
                return Some(self.search_match(offset, entry));
            }
        }

        let result = if let Some(next_cursor) = next_older_cursor {
            self.advance_older_search_with_batches_from(
                next_cursor,
                boundary_if_exhausted,
                app_event_tx,
            )
        } else {
            self.exhausted_search_result(HistorySearchDirection::Older, boundary_if_exhausted)
        };
        Some(result)
    }

    /// 在固定次数限制内重试失败的批量查找，而不将失败视为历史已耗尽。
    pub(crate) fn on_batch_error(
        &mut self,
        log_id: u64,
        cursor: HistoryBatchCursor,
        app_event_tx: &AppEventSender,
    ) -> Option<HistorySearchResult> {
        if self.persistent_log_id != Some(log_id) {
            return None;
        }

        let (boundary_if_exhausted, read_failures) = self.pending_batch(cursor)?;

        if read_failures < MAX_BATCH_READ_RETRIES
            && let Some(thread_id) = self.thread_id
        {
            if let Some(search) = self.search.as_mut() {
                search.awaiting = Some(PendingHistorySearch::Batch {
                    cursor: cursor.clone(),
                    boundary_if_exhausted,
                    read_failures: read_failures + 1,
                });
            }
            app_event_tx.send(AppEvent::LookupMessageHistoryBatch {
                thread_id,
                cursor,
                log_id,
            });
            return Some(HistorySearchResult::Pending);
        }

        Some(if boundary_if_exhausted {
            if let Some(search) = self.search.as_mut() {
                search.awaiting = None;
            }
            HistorySearchResult::AtBoundary
        } else {
            self.search = None;
            HistorySearchResult::Unavailable
        })
    }

    fn pending_batch(&self, cursor: HistoryBatchCursor) -> Option<(bool, u8)> {
        let Some(PendingHistorySearch::Batch {
            cursor: awaited_cursor,
            boundary_if_exhausted,
            read_failures,
        }) = self
            .search
            .as_ref()
            .and_then(|search| search.awaiting.as_ref())
        else {
            return None;
        };
        (awaited_cursor == &cursor).then_some((*boundary_if_exhausted, *read_failures))
    }

    /// 将更旧方向的搜索从“探测最新单条条目”切换到有界的批量查找。
    ///
    /// 将第一次探测保留在单条目路径上，可以在最新持久化条目即匹配时避免拉取整批数据；而此后的
    /// 每次未命中都分摊到一个有界的批量中。
    pub(super) fn advance_older_search_after_entry_miss(
        &mut self,
        offset: usize,
        boundary_if_exhausted: bool,
        app_event_tx: &AppEventSender,
    ) -> HistorySearchResult {
        let Some(next_offset) = offset.checked_sub(1) else {
            return self
                .exhausted_search_result(HistorySearchDirection::Older, boundary_if_exhausted);
        };
        self.advance_older_search_with_batches_from(
            HistoryBatchCursor::new(next_offset),
            boundary_if_exhausted,
            app_event_tx,
        )
    }

    /// 从批量游标处扫描已缓存的偏移量，并请求第一个未缓存的更旧范围。
    ///
    /// 字节锚点仅在扫描从其精确的 `end_offset` 开始时可用；越过已缓存条目后会产生仅含偏移量的
    /// 游标，因为那些中间位置不存在经过验证的字节边界。
    fn advance_older_search_with_batches_from(
        &mut self,
        mut cursor: HistoryBatchCursor,
        boundary_if_exhausted: bool,
        app_event_tx: &AppEventSender,
    ) -> HistorySearchResult {
        let mut offset = cursor.end_offset();
        loop {
            if let Some(entry) = self.entry_at_cached_offset(offset) {
                if self.search_matches(&entry) && self.search_result_is_unique(&entry) {
                    return self.search_match(offset, entry);
                }
            } else if !self.fetched_history.contains_key(&offset)
                && offset < self.persistent_entry_count
            {
                return self.request_older_search_batch(
                    cursor,
                    boundary_if_exhausted,
                    app_event_tx,
                );
            }

            let Some(next_offset) = offset.checked_sub(1) else {
                return self
                    .exhausted_search_result(HistorySearchDirection::Older, boundary_if_exhausted);
            };
            offset = next_offset;
            cursor = HistoryBatchCursor::new(offset);
        }
    }

    pub(super) fn request_older_search_batch(
        &mut self,
        cursor: HistoryBatchCursor,
        boundary_if_exhausted: bool,
        app_event_tx: &AppEventSender,
    ) -> HistorySearchResult {
        let (Some(thread_id), Some(log_id)) = (self.thread_id, self.persistent_log_id) else {
            return self
                .exhausted_search_result(HistorySearchDirection::Older, boundary_if_exhausted);
        };
        if let Some(search) = self.search.as_mut() {
            search.awaiting = Some(PendingHistorySearch::Batch {
                cursor: cursor.clone(),
                boundary_if_exhausted,
                read_failures: 0,
            });
        }
        app_event_tx.send(AppEvent::LookupMessageHistoryBatch {
            thread_id,
            cursor,
            log_id,
        });
        HistorySearchResult::Pending
    }
}
