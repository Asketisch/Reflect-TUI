//! Chat composer 历史 的测试集。
//!
//! 从 bottom_pane/chat_composer_history.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event::HistoryBatchEntryResponse;
use crate::tui_core::bottom_pane::MentionBinding;
use pretty_assertions::assert_eq;
use tokio::sync::mpsc::unbounded_channel;

fn test_thread_id() -> ThreadId {
    ThreadId::from_string("67e55044-10b1-426f-9247-bb680e5fe0c8").expect("thread id should parse")
}

fn batch_entry(offset: usize, entry: &str) -> HistoryBatchEntryResponse {
    HistoryBatchEntryResponse {
        offset,
        entry: Some(entry.to_string()),
    }
}

#[test]
fn duplicate_submissions_are_not_recorded() {
    let mut history = ChatComposerHistory::new();

    // 空提交会被忽略。
    history.record_local_submission(HistoryEntry::new(String::new()));
    assert_eq!(history.local_history.len(), 0);

    // 第一条条目被记录。
    history.record_local_submission(HistoryEntry::new("hello".to_string()));
    assert_eq!(history.local_history.len(), 1);
    assert_eq!(
        history.local_history.last().unwrap(),
        &HistoryEntry::new("hello".to_string())
    );

    // 相同的连续条目会被跳过。
    history.record_local_submission(HistoryEntry::new("hello".to_string()));
    assert_eq!(history.local_history.len(), 1);

    // 不同的条目会被记录。
    history.record_local_submission(HistoryEntry::new("world".to_string()));
    assert_eq!(history.local_history.len(), 2);
    assert_eq!(
        history.local_history.last().unwrap(),
        &HistoryEntry::new("world".to_string())
    );
}

#[test]
fn persistent_restore_gates_at_mentions() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);
    let mut history = ChatComposerHistory::new();
    history.set_metadata(test_thread_id(), /*log_id*/ 42, /*entry_count*/ 1);

    assert!(history.navigate_up(&tx).is_none());
    let disabled = history.on_entry_response(
        /*log_id*/ 42,
        /*offset*/ 0,
        Some("[@sample](plugin://sample@test) and [$figma](app://figma)".to_string()),
        &tx,
    );
    assert_eq!(
        disabled,
        HistoryEntryResponse::Found(HistoryEntry {
            text: "$sample and $figma".to_string(),
            text_elements: Vec::new(),
            local_image_paths: Vec::new(),
            remote_image_urls: Vec::new(),
            mention_bindings: vec![
                MentionBinding {
                    sigil: '$',
                    mention: "sample".to_string(),
                    path: "plugin://sample@test".to_string(),
                },
                MentionBinding {
                    sigil: '$',
                    mention: "figma".to_string(),
                    path: "app://figma".to_string(),
                },
            ],
            pending_pastes: Vec::new(),
        })
    );

    history.set_at_mention_restore_enabled(/*enabled*/ true);
    assert!(history.navigate_up(&tx).is_none());
    let enabled = history.on_entry_response(
        /*log_id*/ 42,
        /*offset*/ 0,
        Some("[@sample](plugin://sample@test) and [$figma](app://figma)".to_string()),
        &tx,
    );
    assert_eq!(
        enabled,
        HistoryEntryResponse::Found(HistoryEntry {
            text: "@sample and $figma".to_string(),
            text_elements: Vec::new(),
            local_image_paths: Vec::new(),
            remote_image_urls: Vec::new(),
            mention_bindings: vec![
                MentionBinding {
                    sigil: '@',
                    mention: "sample".to_string(),
                    path: "plugin://sample@test".to_string(),
                },
                MentionBinding {
                    sigil: '$',
                    mention: "figma".to_string(),
                    path: "app://figma".to_string(),
                },
            ],
            pending_pastes: Vec::new(),
        })
    );
}

#[test]
fn navigation_with_async_fetch() {
    let (tx, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);

    let mut history = ChatComposerHistory::new();
    // 假设存在 3 条持久化条目。
    let thread_id = test_thread_id();
    history.set_metadata(thread_id, /*log_id*/ 1, /*entry_count*/ 3);
    history.record_local_submission(HistoryEntry::new("latest".to_string()));

    // 第一次按 Up 应当回忆当前会话内的本地历史。
    assert!(history.should_handle_navigation("", /*cursor*/ 0));
    assert_eq!(
        Some(HistoryEntry::new("latest".to_string())),
        history.navigate_up(&tx)
    );

    // 下一次按 Up 应请求 offset 2 并等待异步数据。
    assert!(history.navigate_up(&tx).is_none()); // 暂不替换文本

    // 验证已发送历史查找请求。
    let event = rx.try_recv().expect("expected AppEvent to be sent");
    let AppEvent::LookupMessageHistoryEntry {
        thread_id: response_thread_id,
        offset,
        log_id,
    } = event
    else {
        panic!("unexpected event variant");
    };
    assert_eq!(response_thread_id, thread_id);
    assert_eq!(offset, 2);
    assert_eq!(log_id, 1);

    // 注入异步响应。
    assert_eq!(
        HistoryEntryResponse::Found(HistoryEntry::new("latest".to_string())),
        history.on_entry_response(
            /*log_id*/ 1,
            /*offset*/ 2,
            Some("latest".into()),
            &tx
        )
    );

    // 下一次按 Up 应移动到 offset 1。
    assert!(history.navigate_up(&tx).is_none()); // 暂不替换文本

    // 验证针对 offset 1 的第二次查找请求。
    let event2 = rx.try_recv().expect("expected second event");
    let AppEvent::LookupMessageHistoryEntry {
        thread_id: response_thread_id,
        offset,
        log_id,
    } = event2
    else {
        panic!("unexpected event variant");
    };
    assert_eq!(response_thread_id, thread_id);
    assert_eq!(offset, 1);
    assert_eq!(log_id, 1);

    assert_eq!(
        HistoryEntryResponse::Found(HistoryEntry::new("older".to_string())),
        history.on_entry_response(
            /*log_id*/ 1,
            /*offset*/ 1,
            Some("older".into()),
            &tx
        )
    );
}

#[test]
fn search_matches_local_history_and_stops_at_boundaries() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);

    let mut history = ChatComposerHistory::new();
    history.record_local_submission(HistoryEntry::new("git status".to_string()));
    history.record_local_submission(HistoryEntry::new("cargo test -p reflect-tui".to_string()));
    history.record_local_submission(HistoryEntry::new("git diff".to_string()));

    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("git diff".to_string())),
        history.search(
            "git",
            HistorySearchDirection::Older,
            /*restart*/ true,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("git status".to_string())),
        history.search(
            "git",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::AtBoundary,
        history.search(
            "git",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::AtBoundary,
        history.search(
            "git",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("git diff".to_string())),
        history.search(
            "git",
            HistorySearchDirection::Newer,
            /*restart*/ false,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::AtBoundary,
        history.search(
            "git",
            HistorySearchDirection::Newer,
            /*restart*/ false,
            &tx
        )
    );
}

#[test]
fn search_skips_duplicate_local_matches() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);

    let mut history = ChatComposerHistory::new();
    history.record_local_submission(HistoryEntry::new("git status".to_string()));
    history.record_local_submission(HistoryEntry::new("cargo test -p reflect-tui".to_string()));
    history.record_local_submission(HistoryEntry::new("git status".to_string()));
    history.record_local_submission(HistoryEntry::new("git diff".to_string()));

    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("git diff".to_string())),
        history.search(
            "git",
            HistorySearchDirection::Older,
            /*restart*/ true,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("git status".to_string())),
        history.search(
            "git",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::AtBoundary,
        history.search(
            "git",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("git diff".to_string())),
        history.search(
            "git",
            HistorySearchDirection::Newer,
            /*restart*/ false,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("git status".to_string())),
        history.search(
            "git",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
}

#[test]
fn repeated_boundary_search_does_not_refetch_persistent_history() {
    let (tx, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);

    let mut history = ChatComposerHistory::new();
    history.set_metadata(test_thread_id(), /*log_id*/ 1, /*entry_count*/ 3);

    assert_eq!(
        HistorySearchResult::Pending,
        history.search(
            "needle",
            HistorySearchDirection::Older,
            /*restart*/ true,
            &tx
        )
    );
    let _ = rx.try_recv().expect("expected latest lookup");
    assert_eq!(
        HistoryEntryResponse::Search(HistorySearchResult::Found(HistoryEntry::new(
            "needle latest".to_string()
        ))),
        history.on_entry_response(
            /*log_id*/ 1,
            /*offset*/ 2,
            Some("needle latest".into()),
            &tx,
        )
    );

    assert_eq!(
        HistorySearchResult::Pending,
        history.search(
            "needle",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    let _ = rx.try_recv().expect("expected next older lookup");
    assert_eq!(
        HistoryEntryResponse::Search(HistorySearchResult::Pending),
        history.on_entry_response(
            /*log_id*/ 1,
            /*offset*/ 1,
            Some("not a match".into()),
            &tx,
        )
    );
    let AppEvent::LookupMessageHistoryBatch { cursor, .. } =
        rx.try_recv().expect("expected oldest batch")
    else {
        panic!("unexpected event variant");
    };
    assert_eq!(cursor.end_offset(), 0);
    assert_eq!(
        Some(HistorySearchResult::AtBoundary),
        history.on_batch_response(
            /*log_id*/ 1,
            cursor,
            vec![batch_entry(/*offset*/ 0, "also not a match")],
            /*next_older_cursor*/ None,
            &tx,
        )
    );
    assert!(rx.try_recv().is_err());

    assert_eq!(
        HistorySearchResult::AtBoundary,
        history.search(
            "needle",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    assert!(rx.try_recv().is_err());
}

#[test]
fn search_fetches_persistent_history_until_match() {
    let (tx, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);

    let mut history = ChatComposerHistory::new();
    let thread_id = test_thread_id();
    history.set_metadata(thread_id, /*log_id*/ 1, /*entry_count*/ 3);

    assert_eq!(
        HistorySearchResult::Pending,
        history.search(
            "older",
            HistorySearchDirection::Older,
            /*restart*/ true,
            &tx
        )
    );
    let AppEvent::LookupMessageHistoryEntry {
        thread_id: response_thread_id,
        offset,
        log_id,
    } = rx.try_recv().expect("expected latest lookup")
    else {
        panic!("unexpected event variant");
    };
    assert_eq!(response_thread_id, thread_id);
    assert_eq!(offset, 2);
    assert_eq!(log_id, 1);

    assert_eq!(
        HistoryEntryResponse::Search(HistorySearchResult::Pending),
        history.on_entry_response(
            /*log_id*/ 1,
            /*offset*/ 2,
            Some("latest".into()),
            &tx
        )
    );
    let AppEvent::LookupMessageHistoryBatch {
        thread_id: response_thread_id,
        cursor,
        log_id,
    } = rx.try_recv().expect("expected next lookup")
    else {
        panic!("unexpected event variant");
    };
    assert_eq!(response_thread_id, thread_id);
    assert_eq!(cursor.end_offset(), 1);
    assert_eq!(log_id, 1);

    assert_eq!(
        Some(HistorySearchResult::Found(HistoryEntry::new(
            "OLDER command".to_string()
        ))),
        history.on_batch_response(
            /*log_id*/ 1,
            cursor,
            vec![batch_entry(/*offset*/ 1, "OLDER command")],
            Some(HistoryBatchCursor::new(/*end_offset*/ 0)),
            &tx
        )
    );
}

#[test]
fn search_skips_duplicate_persistent_matches() {
    let (tx, mut rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);

    let mut history = ChatComposerHistory::new();
    history.set_metadata(test_thread_id(), /*log_id*/ 1, /*entry_count*/ 4);

    assert_eq!(
        HistorySearchResult::Pending,
        history.search(
            "needle",
            HistorySearchDirection::Older,
            /*restart*/ true,
            &tx
        )
    );
    let _ = rx.try_recv().expect("expected latest lookup");
    assert_eq!(
        HistoryEntryResponse::Search(HistorySearchResult::Found(HistoryEntry::new(
            "needle same".to_string()
        ))),
        history.on_entry_response(
            /*log_id*/ 1,
            /*offset*/ 3,
            Some("needle same".into()),
            &tx,
        )
    );

    assert_eq!(
        HistorySearchResult::Pending,
        history.search(
            "needle",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    let _ = rx.try_recv().expect("expected duplicate lookup");
    assert_eq!(
        HistoryEntryResponse::Search(HistorySearchResult::Pending),
        history.on_entry_response(
            /*log_id*/ 1,
            /*offset*/ 2,
            Some("needle same".into()),
            &tx,
        )
    );
    let AppEvent::LookupMessageHistoryBatch { cursor, .. } =
        rx.try_recv().expect("expected next batch after duplicate")
    else {
        panic!("unexpected event variant");
    };
    assert_eq!(cursor.end_offset(), 1);
    assert_eq!(
        Some(HistorySearchResult::Found(HistoryEntry::new(
            "needle older".to_string()
        ))),
        history.on_batch_response(
            /*log_id*/ 1,
            cursor,
            vec![
                batch_entry(/*offset*/ 1, "not a match"),
                batch_entry(/*offset*/ 0, "needle older"),
            ],
            /*next_older_cursor*/ None,
            &tx,
        )
    );
    assert_eq!(
        HistorySearchResult::AtBoundary,
        history.search(
            "needle",
            HistorySearchDirection::Older,
            /*restart*/ false,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("needle same".to_string())),
        history.search(
            "needle",
            HistorySearchDirection::Newer,
            /*restart*/ false,
            &tx
        )
    );
}

#[test]
fn search_is_case_insensitive_and_empty_query_finds_latest() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);

    let mut history = ChatComposerHistory::new();
    history.record_local_submission(HistoryEntry::new("Build Release".to_string()));

    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("Build Release".to_string())),
        history.search(
            "release",
            HistorySearchDirection::Older,
            /*restart*/ true,
            &tx
        )
    );
    assert_eq!(
        HistorySearchResult::Found(HistoryEntry::new("Build Release".to_string())),
        history.search(
            "",
            HistorySearchDirection::Older,
            /*restart*/ true,
            &tx
        )
    );
}

#[test]
fn reset_navigation_resets_cursor() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx);

    let mut history = ChatComposerHistory::new();
    history.set_metadata(test_thread_id(), /*log_id*/ 1, /*entry_count*/ 3);
    history
        .fetched_history
        .insert(1, Some(HistoryEntry::new("command2".to_string())));
    history
        .fetched_history
        .insert(2, Some(HistoryEntry::new("command3".to_string())));

    assert_eq!(
        Some(HistoryEntry::new("command3".to_string())),
        history.navigate_up(&tx)
    );
    assert_eq!(
        Some(HistoryEntry::new("command2".to_string())),
        history.navigate_up(&tx)
    );

    history.reset_navigation();
    assert!(history.history_cursor.is_none());
    assert!(history.last_history_text.is_none());

    assert_eq!(
        Some(HistoryEntry::new("command3".to_string())),
        history.navigate_up(&tx)
    );
}

#[test]
fn should_handle_navigation_when_cursor_is_at_line_boundaries() {
    let mut history = ChatComposerHistory::new();
    history.record_local_submission(HistoryEntry::new("hello".to_string()));
    history.last_history_text = Some("hello".to_string());

    assert!(history.should_handle_navigation("hello", /*cursor*/ 0));
    assert!(history.should_handle_navigation("hello", "hello".len()));
    assert!(!history.should_handle_navigation("hello", /*cursor*/ 1));
    assert!(!history.should_handle_navigation("other", /*cursor*/ 0));
}
