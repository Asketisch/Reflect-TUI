//! Chat composer 历史搜索（history_search） 的测试集。
//!
//! 从 bottom_pane/chat_composer/history_search.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use tokio::sync::mpsc::unbounded_channel;

use super::super::super::chat_composer_history::HistoryEntry;
use super::super::super::chat_composer_history::HistorySearchResult;
use super::super::super::footer::FooterMode;
use super::super::ChatComposer;
use super::HistorySearchStatus;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::render::renderable::Renderable;

#[test]
fn history_search_opens_without_previewing_latest_entry() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer
        .history
        .record_local_submission(HistoryEntry::new("remembered command".to_string()));
    composer.set_text_content(String::new(), Vec::new(), Vec::new());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert!(composer.history_search_active());
    assert!(composer.draft.textarea.is_empty());
    assert_eq!(composer.footer_mode(), FooterMode::HistorySearch);
}

#[test]
fn unavailable_history_search_restores_idle_draft() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer.set_text_content("draft".to_string(), Vec::new(), Vec::new());
    composer.begin_history_search();
    composer.apply_history_search_result(HistorySearchResult::Pending);

    composer.apply_history_search_result(HistorySearchResult::Unavailable);

    assert_eq!(composer.draft.textarea.text(), "draft");
    assert!(
        composer
            .history_search
            .as_ref()
            .is_some_and(|search| matches!(search.status, HistorySearchStatus::Idle))
    );
}

#[test]
fn history_search_match_ranges_are_case_insensitive() {
    assert_eq!(
        ChatComposer::case_insensitive_match_ranges("git status git", "GIT"),
        vec![0..3, 11..14]
    );
    assert_eq!(
        ChatComposer::case_insensitive_match_ranges("aİ i", "i"),
        vec![1..3, 4..5]
    );
    assert!(ChatComposer::case_insensitive_match_ranges("git", "").is_empty());
}

#[test]
fn history_search_accepts_matching_entry() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer
        .history
        .record_local_submission(HistoryEntry::new("git status".to_string()));
    composer
        .history
        .record_local_submission(HistoryEntry::new("cargo test".to_string()));
    composer.set_text_content("draft".to_string(), Vec::new(), Vec::new());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert!(composer.history_search_active());
    assert_eq!(composer.draft.textarea.text(), "draft");

    for ch in ['g', 'i', 't'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(composer.draft.textarea.text(), "git status");
    assert_eq!(composer.footer_mode(), FooterMode::HistorySearch);

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!composer.history_search_active());
    assert_eq!(composer.draft.textarea.text(), "git status");
    assert_eq!(
        composer.draft.textarea.cursor(),
        composer.draft.textarea.text().len()
    );
}

#[test]
fn vim_normal_history_search_preview_places_cursor_on_last_char() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer
        .history
        .record_local_submission(HistoryEntry::new("git status".to_string()));
    composer.set_vim_enabled(/*enabled*/ true);

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    for ch in ['g', 'i', 't'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }

    assert_eq!(composer.draft.textarea.text(), "git status");
    assert_eq!(composer.draft.textarea.cursor(), "git status".len() - 1);
    assert_eq!(composer.footer_mode(), FooterMode::HistorySearch);
}

#[test]
fn history_search_stays_on_single_match_at_boundaries() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer.history.record_local_submission(HistoryEntry::new(
        "Find and fix a bug in @filename".to_string(),
    ));
    composer.set_text_content("draft".to_string(), Vec::new(), Vec::new());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    for ch in ['b', 'u', 'g'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(
        composer.draft.textarea.text(),
        "Find and fix a bug in @filename"
    );

    for _ in 0..3 {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    }
    assert_eq!(
        composer.draft.textarea.text(),
        "Find and fix a bug in @filename"
    );
    assert!(
        composer
            .history_search
            .as_ref()
            .is_some_and(|search| matches!(search.status, HistorySearchStatus::Match))
    );

    for _ in 0..3 {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    assert_eq!(
        composer.draft.textarea.text(),
        "Find and fix a bug in @filename"
    );
    assert!(
        composer
            .history_search
            .as_ref()
            .is_some_and(|search| matches!(search.status, HistorySearchStatus::Match))
    );
}

#[test]
fn history_search_footer_action_hints_are_emphasized() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ true,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer
        .history
        .record_local_submission(HistoryEntry::new("cargo test".to_string()));

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));

    let line = composer
        .history_search_footer_line()
        .expect("expected history search footer line");
    assert_eq!(
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>(),
        vec![
            "reverse-i-search: ",
            "c",
            "  ",
            "enter",
            " accept",
            " · ",
            "esc",
            " cancel"
        ]
    );

    let query_style = line.spans[1].style;
    assert_eq!(query_style.fg, Some(ratatui::style::Color::Cyan));

    let enter_style = line.spans[3].style;
    assert_eq!(enter_style.fg, Some(ratatui::style::Color::Cyan));
    assert!(enter_style.add_modifier.contains(Modifier::BOLD));
    assert!(enter_style.sub_modifier.contains(Modifier::DIM));

    let accept_style = line.spans[4].style;
    assert!(accept_style.add_modifier.contains(Modifier::DIM));

    let separator_style = line.spans[5].style;
    assert!(separator_style.add_modifier.contains(Modifier::DIM));

    let esc_style = line.spans[6].style;
    assert_eq!(esc_style.fg, Some(ratatui::style::Color::Cyan));
    assert!(esc_style.add_modifier.contains(Modifier::BOLD));
    assert!(esc_style.sub_modifier.contains(Modifier::DIM));

    let cancel_style = line.spans[7].style;
    assert!(cancel_style.add_modifier.contains(Modifier::DIM));
}

#[test]
fn history_search_highlights_matches_until_accepted() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ true,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer
        .history
        .record_local_submission(HistoryEntry::new("cargo test".to_string()));
    composer
        .history
        .record_local_submission(HistoryEntry::new("git status".to_string()));

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    for ch in ['g', 'i', 't'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }

    let area = Rect::new(0, 0, 60, 8);
    let [_, _, textarea_rect, _] = composer.layout_areas(area);
    let mut buf = Buffer::empty(area);
    composer.render(area, &mut buf);
    let x = textarea_rect.x;
    let y = textarea_rect.y;
    assert_eq!(buf[(x, y)].symbol(), "g");
    for offset in 0..3 {
        let modifier = buf[(x + offset, y)].style().add_modifier;
        assert!(modifier.contains(Modifier::REVERSED));
        assert!(modifier.contains(Modifier::BOLD));
    }
    assert!(
        !buf[(x + 3, y)]
            .style()
            .add_modifier
            .contains(Modifier::REVERSED)
    );

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let [_, _, accepted_textarea_rect, _] = composer.layout_areas(area);
    let mut accepted_buf = Buffer::empty(area);
    composer.render(area, &mut accepted_buf);
    for offset in 0..3 {
        let modifier = accepted_buf[(accepted_textarea_rect.x + offset, accepted_textarea_rect.y)]
            .style()
            .add_modifier;
        assert!(!modifier.contains(Modifier::REVERSED));
        assert!(!modifier.contains(Modifier::BOLD));
    }
}

#[test]
fn history_search_esc_restores_original_draft() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer
        .history
        .record_local_submission(HistoryEntry::new("remembered command".to_string()));
    composer.set_text_content("draft".to_string(), Vec::new(), Vec::new());
    composer.draft.textarea.set_cursor(/*pos*/ 2);

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    assert_eq!(composer.draft.textarea.text(), "draft");
    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    assert_eq!(composer.draft.textarea.text(), "remembered command");

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!composer.history_search_active());
    assert_eq!(composer.draft.textarea.text(), "draft");
    assert_eq!(composer.draft.textarea.cursor(), 2);
}

#[test]
fn history_search_ctrl_c_restores_original_draft() {
    fn composer_with_search_preview() -> ChatComposer {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let sender = AppEventSender::new(tx);
        let mut composer = ChatComposer::new(
            /*has_input_focus*/ true,
            sender,
            /*enhanced_keys_supported*/ false,
            "Ask Reflect to do anything".to_string(),
            /*disable_paste_burst*/ false,
        );
        composer
            .history
            .record_local_submission(HistoryEntry::new("remembered command".to_string()));
        composer.set_text_content("draft".to_string(), Vec::new(), Vec::new());
        composer.draft.textarea.set_cursor(/*pos*/ 2);

        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert_eq!(composer.draft.textarea.text(), "remembered command");
        composer
    }

    for cancel_key in [
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Char('\u{0003}'), KeyModifiers::NONE),
    ] {
        let mut composer = composer_with_search_preview();

        let _ = composer.handle_key_event(cancel_key);

        assert!(!composer.history_search_active());
        assert_eq!(composer.draft.textarea.text(), "draft");
        assert_eq!(composer.draft.textarea.cursor(), 2);
    }
}

#[test]
fn history_search_flushes_pending_first_char_before_snapshot() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert!(composer.is_in_paste_burst());
    assert_eq!(composer.draft.textarea.text(), "");

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert!(composer.history_search_active());
    assert!(!composer.is_in_paste_burst());
    assert_eq!(composer.draft.textarea.text(), "h");

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(!composer.history_search_active());
    assert_eq!(composer.draft.textarea.text(), "h");
}

#[test]
fn history_search_flushes_buffered_paste_before_snapshot() {
    use std::time::Duration;
    use std::time::Instant;

    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );

    let mut now = Instant::now();
    for ch in ['p', 'a', 's', 't', 'e'] {
        let _ = composer.handle_input_basic_with_time(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
            now,
        );
        now += Duration::from_millis(1);
    }
    assert!(composer.is_in_paste_burst());
    assert_eq!(composer.draft.textarea.text(), "");

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));

    assert!(composer.history_search_active());
    assert!(!composer.is_in_paste_burst());
    assert_eq!(composer.draft.textarea.text(), "paste");

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(!composer.history_search_active());
    assert_eq!(composer.draft.textarea.text(), "paste");
}

#[test]
fn history_search_esc_resets_normal_history_navigation() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer
        .history
        .record_local_submission(HistoryEntry::new("oldest matching entry".to_string()));
    composer
        .history
        .record_local_submission(HistoryEntry::new("newest entry".to_string()));
    composer.set_text_content(String::new(), Vec::new(), Vec::new());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    for ch in ['m', 'a', 't', 'c', 'h'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert_eq!(composer.draft.textarea.text(), "oldest matching entry");

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!composer.history_search_active());
    assert!(composer.draft.textarea.is_empty());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(composer.draft.textarea.text(), "newest entry");
}

#[test]
fn history_search_no_match_restores_preview_but_keeps_search_open() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let sender = AppEventSender::new(tx);
    let mut composer = ChatComposer::new(
        /*has_input_focus*/ true,
        sender,
        /*enhanced_keys_supported*/ false,
        "Ask Reflect to do anything".to_string(),
        /*disable_paste_burst*/ false,
    );
    composer
        .history
        .record_local_submission(HistoryEntry::new("git status".to_string()));
    composer.set_text_content("draft".to_string(), Vec::new(), Vec::new());

    let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
    for ch in ['z', 'z', 'z'] {
        let _ = composer.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }

    assert!(composer.history_search_active());
    assert_eq!(composer.draft.textarea.text(), "draft");
    assert_eq!(composer.footer_mode(), FooterMode::HistorySearch);
}
