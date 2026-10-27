//! 多选选择器 的测试集。
//!
//! 从 bottom_pane/multi_select_picker.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::tui_core::app_event::AppEvent;
use pretty_assertions::assert_eq;
use tokio::sync::mpsc::unbounded_channel;

fn test_picker(items: Vec<MultiSelectItem>) -> MultiSelectPicker {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    MultiSelectPicker::builder(
        "Test".to_string(),
        /*subtitle*/ None,
        AppEventSender::new(tx),
    )
    .items(items)
    .enable_ordering()
    .build()
}

fn item(id: &str, orderable: bool, section_break_after: bool) -> MultiSelectItem {
    MultiSelectItem {
        id: id.to_string(),
        name: id.to_string(),
        orderable,
        section_break_after,
        ..Default::default()
    }
}

#[test]
fn non_orderable_items_cannot_move_or_be_crossed() {
    let mut picker = test_picker(vec![
        item(
            "theme-colors",
            /*orderable*/ false,
            /*section_break_after*/ true,
        ),
        item(
            "model", /*orderable*/ true, /*section_break_after*/ false,
        ),
        item(
            "branch", /*orderable*/ true, /*section_break_after*/ false,
        ),
    ]);

    picker.move_selected_item(Direction::Down);
    assert_eq!(
        picker
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["theme-colors", "model", "branch"]
    );

    picker.move_down();
    picker.move_selected_item(Direction::Up);
    assert_eq!(
        picker
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["theme-colors", "model", "branch"]
    );
}

#[test]
fn horizontal_list_keys_reorder_orderable_items() {
    let mut picker = test_picker(vec![
        item(
            "model", /*orderable*/ true, /*section_break_after*/ false,
        ),
        item(
            "branch", /*orderable*/ true, /*section_break_after*/ false,
        ),
    ]);

    picker.handle_key_event(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    assert_eq!(
        picker
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["branch", "model"]
    );

    picker.handle_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert_eq!(
        picker
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["model", "branch"]
    );
}

#[test]
fn section_break_after_item_renders_separator_row() {
    let picker = test_picker(vec![
        item(
            "theme-colors",
            /*orderable*/ false,
            /*section_break_after*/ true,
        ),
        item(
            "model", /*orderable*/ true, /*section_break_after*/ false,
        ),
    ]);

    let rows = picker.build_rows();

    assert_eq!(
        rows.rows
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        vec!["› [ ] theme-colors", SECTION_BREAK_ROW, "  [ ] model"]
    );
    assert_eq!(rows.state.selected_idx, Some(0));
}

#[test]
fn searchable_plain_j_updates_query_instead_of_navigating() {
    let mut picker = test_picker(vec![
        item(
            "alpha", /*orderable*/ true, /*section_break_after*/ false,
        ),
        item(
            "jupiter", /*orderable*/ true, /*section_break_after*/ false,
        ),
    ]);

    picker.handle_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));

    assert_eq!(picker.search_query, "j");
    assert_eq!(picker.filtered_indices, vec![1]);
    assert_eq!(picker.state.selected_idx, Some(0));
}

#[test]
fn page_and_jump_navigation_use_list_keymap() {
    let (tx, _rx) = unbounded_channel::<AppEvent>();
    let mut keymap = RuntimeKeymap::defaults().list;
    keymap.page_down = vec![key_hint::ctrl(KeyCode::Char('d'))];
    keymap.page_up = vec![key_hint::ctrl(KeyCode::Char('u'))];
    keymap.jump_bottom = vec![key_hint::ctrl(KeyCode::Char('e'))];
    keymap.jump_top = vec![key_hint::ctrl(KeyCode::Char('a'))];
    let mut picker = MultiSelectPicker::builder(
        "Test".to_string(),
        /*subtitle*/ None,
        AppEventSender::new(tx),
    )
    .items(
        (0..12)
            .map(|idx| {
                item(
                    &format!("item-{idx}"),
                    /*orderable*/ true,
                    /*section_break_after*/ false,
                )
            })
            .collect(),
    )
    .list_keymap(keymap)
    .build();

    picker.handle_key_event(KeyEvent::from(KeyCode::PageDown));
    assert_eq!(picker.state.selected_idx, Some(0));

    picker.handle_key_event(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    assert_eq!(picker.state.selected_idx, Some(8));

    picker.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(picker.state.selected_idx, Some(0));

    picker.handle_key_event(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert_eq!(picker.state.selected_idx, Some(11));

    picker.handle_key_event(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_eq!(picker.state.selected_idx, Some(0));
}
