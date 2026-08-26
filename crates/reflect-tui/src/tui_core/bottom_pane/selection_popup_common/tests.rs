//! 选择弹窗公共逻辑 的测试集。
//!
//! 从 bottom_pane/selection_popup_common.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use pretty_assertions::assert_eq;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

#[test]
fn one_cell_width_falls_back_without_panic_for_wrapped_two_column_rows() {
    let row = GenericDisplayRow {
        name: "1. Very long option label".to_string(),
        description: Some("Very long description".to_string()),
        wrap_indent: Some(4),
        ..Default::default()
    };

    let two_col = wrap_two_column_row(&row, /*desc_col*/ 0, /*width*/ 1);
    assert_eq!(two_col.len(), 0);
}

#[test]
fn selected_rows_use_the_shared_accent_style() {
    let rows = vec![GenericDisplayRow {
        name: "selected".to_string(),
        ..Default::default()
    }];
    let state = ScrollState {
        selected_idx: Some(0),
        ..Default::default()
    };
    let area = Rect::new(0, 0, 16, 1);
    let mut buf = Buffer::empty(area);

    render_rows(
        area, &mut buf, &rows, &state, /*max_results*/ 1, "no rows",
    );

    let style = buf[(0, 0)].style();
    let expected = accent_style();
    assert_eq!(style.fg, expected.fg);
    assert!(style.add_modifier.contains(Modifier::BOLD));
}
