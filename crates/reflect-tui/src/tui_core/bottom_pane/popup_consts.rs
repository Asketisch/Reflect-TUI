//! 底部面板各 widget 共享的弹窗相关常量。

use ratatui::text::Line;

use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::keymap::ListKeymap;
use crate::tui_core::keymap::primary_binding;
use crossterm::event::KeyCode;

/// 任何弹窗最多应尝试显示的行数。
/// 各弹窗之间保持一致，以获得统一的手感。
pub(crate) const MAX_POPUP_ROWS: usize = 8;

/// 弹窗使用的标准页脚提示文本。
pub(crate) fn standard_popup_hint_line() -> Line<'static> {
    Line::from(vec![
        "Press ".into(),
        key_hint::plain(KeyCode::Enter).into(),
        " to confirm or ".into(),
        key_hint::plain(KeyCode::Esc).into(),
        " to go back".into(),
    ])
}

pub(crate) fn standard_popup_hint_line_for_keymap(list_keymap: &ListKeymap) -> Line<'static> {
    accept_cancel_hint_line(
        primary_binding(&list_keymap.accept),
        "to confirm",
        primary_binding(&list_keymap.cancel),
        "to go back",
    )
}

pub(crate) fn accept_cancel_hint_line(
    accept: Option<KeyBinding>,
    accept_label: &'static str,
    cancel: Option<KeyBinding>,
    cancel_label: &'static str,
) -> Line<'static> {
    match (accept, cancel) {
        (Some(accept), Some(cancel)) => Line::from(vec![
            "Press ".into(),
            accept.into(),
            format!(" {accept_label} or ").into(),
            cancel.into(),
            format!(" {cancel_label}").into(),
        ]),
        (Some(accept), None) => Line::from(vec![
            "Press ".into(),
            accept.into(),
            format!(" {accept_label}").into(),
        ]),
        (None, Some(cancel)) => Line::from(vec![
            "Press ".into(),
            cancel.into(),
            format!(" {cancel_label}").into(),
        ]),
        (None, None) => Line::from(""),
    }
}
