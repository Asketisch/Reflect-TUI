//! TUI 的按键绑定原语与输入匹配。
//!
//! 本模块提供 `KeyBinding` —— 单个按键绑定（按键码 + 修饰键集合）
//! 的运行时表示 —— 以及处理跨终端不一致性的匹配逻辑：
//! 移位字母与原始 C0 控制字符的上报方式各有不同。
//!
//! 列表和选择器代码应通过这些辅助函数匹配导航，而不是直接
//! 比较 `KeyEvent` 值。匹配器负责兼容将控制组合键
//! 上报为 C0 字符的终端，而 `is_plain_text_key_event` 为可搜索
//! 选择器提供了文本输入与导航命令之间的共享边界。
//!
//! 它还提供渲染辅助函数，将绑定转换为带样式的
//! `ratatui::text::Span` 值用于 UI 提示显示。

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Span;

#[cfg(test)]
const ALT_PREFIX: &str = "⌥ + ";
#[cfg(all(not(test), target_os = "macos"))]
const ALT_PREFIX: &str = "⌥ + ";
#[cfg(all(not(test), not(target_os = "macos")))]
const ALT_PREFIX: &str = "alt + ";
const CTRL_PREFIX: &str = "ctrl + ";
const SHIFT_PREFIX: &str = "shift + ";

/// 一个可触发 TUI 动作的具体按键事件。
///
/// 通过 `is_press` 匹配处理精确相等，并为以下终端提供兼容回退：
/// 无 SHIFT 上报大写字母的终端，以及将 Ctrl 键
/// 上报为原始 C0 控制字符的终端。这意味着定义为 `shift-a` 的绑定
/// 可匹配 `Shift+a` 或普通 `A`，而 `ctrl-j` 可匹配原始 LF。
///
/// 它不建模多键组合键或部分匹配；需要序列的调用方
/// 必须将该状态保存在此类型之外。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct KeyBinding {
    key: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyBinding {
    pub(crate) const fn new(key: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { key, modifiers }
    }

    pub(crate) fn from_event(event: KeyEvent) -> Self {
        let (key, modifiers) = normalize_key_parts(event.code, event.modifiers);
        Self { key, modifiers }
    }

    pub fn is_press(&self, event: KeyEvent) -> bool {
        normalize_key_parts(self.key, self.modifiers)
            == normalize_key_parts(event.code, event.modifiers)
            && (event.kind == KeyEventKind::Press || event.kind == KeyEventKind::Repeat)
    }

    pub(crate) const fn parts(&self) -> (KeyCode, KeyModifiers) {
        (self.key, self.modifiers)
    }

    pub(crate) fn display_label(&self) -> String {
        let modifiers = modifiers_to_string(self.modifiers);
        let key = match self.key {
            KeyCode::Enter => "enter".to_string(),
            KeyCode::Char(' ') => "space".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::PageUp => "pgup".to_string(),
            KeyCode::PageDown => "pgdn".to_string(),
            _ => self.key.to_string().to_ascii_lowercase(),
        };
        format!("{modifiers}{key}")
    }
}

pub(crate) fn normalize_key_parts(
    key: KeyCode,
    mut modifiers: KeyModifiers,
) -> (KeyCode, KeyModifiers) {
    let KeyCode::Char(ch) = key else {
        return (key, modifiers);
    };
    if modifiers.is_empty()
        && let Some(ctrl_char) = c0_control_char_to_ctrl_char(ch)
    {
        return (KeyCode::Char(ctrl_char), KeyModifiers::CONTROL | modifiers);
    }
    if ch.is_ascii_uppercase() {
        modifiers.insert(KeyModifiers::SHIFT);
        return (KeyCode::Char(ch.to_ascii_lowercase()), modifiers);
    }
    (key, modifiers)
}

fn c0_control_char_to_ctrl_char(ch: char) -> Option<char> {
    let code = u32::from(ch);
    match code {
        0x00 => Some(' '),
        0x01..=0x1a => char::from_u32(code - 0x01 + u32::from('a')),
        0x1c..=0x1f => char::from_u32(code - 0x1c + u32::from('4')),
        _ => None,
    }
}

/// 单个动作按键绑定集合的匹配辅助函数。
///
/// 实现应把该切片视为一个动作的替代方案。它们不应将顺序
/// 解释为分发的优先级；顺序保留给通过 `primary_binding` 的
/// UI 提示选择。
pub(crate) trait KeyBindingListExt {
    /// 当此集合中的任一绑定匹配 `event` 时返回 true。
    fn is_pressed(&self, event: KeyEvent) -> bool;
}

impl KeyBindingListExt for [KeyBinding] {
    fn is_pressed(&self, event: KeyEvent) -> bool {
        self.iter().any(|binding| binding.is_press(event))
    }
}

/// 返回一个事件是否应被当作字面文本输入。
///
/// 可搜索选择器用它来避免在同一个字符可能是有效查询时
/// 为导航而吞掉纯可打印字符。例如，
/// 列表可以绑定 `j` 和 `k` 用于移动，但可搜索列表必须让
/// 普通 `j` 更新查询，同时仍允许 `Ctrl+J` 移动。在归一化按键绑定之后
/// 调用此函数会模糊这一区分，导致可打印的
/// 搜索输入消失。
pub(crate) fn is_plain_text_key_event(event: KeyEvent) -> bool {
    matches!(
        event,
        KeyEvent {
            code: KeyCode::Char(ch),
            modifiers,
            ..
        } if !ch.is_ascii_control()
            && !modifiers.contains(KeyModifiers::CONTROL)
            && !modifiers.contains(KeyModifiers::ALT)
    )
}

pub(crate) const fn plain(key: KeyCode) -> KeyBinding {
    KeyBinding::new(key, KeyModifiers::NONE)
}

pub(crate) const fn alt(key: KeyCode) -> KeyBinding {
    KeyBinding::new(key, KeyModifiers::ALT)
}

pub(crate) const fn shift(key: KeyCode) -> KeyBinding {
    KeyBinding::new(key, KeyModifiers::SHIFT)
}

pub(crate) const fn ctrl(key: KeyCode) -> KeyBinding {
    KeyBinding::new(key, KeyModifiers::CONTROL)
}

pub(crate) const fn ctrl_alt(key: KeyCode) -> KeyBinding {
    KeyBinding::new(key, KeyModifiers::CONTROL.union(KeyModifiers::ALT))
}

fn modifiers_to_string(modifiers: KeyModifiers) -> String {
    let mut result = String::new();
    if modifiers.contains(KeyModifiers::CONTROL) {
        result.push_str(CTRL_PREFIX);
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        result.push_str(SHIFT_PREFIX);
    }
    if modifiers.contains(KeyModifiers::ALT) {
        result.push_str(ALT_PREFIX);
    }
    result
}

impl From<KeyBinding> for Span<'static> {
    fn from(binding: KeyBinding) -> Self {
        (&binding).into()
    }
}
impl From<&KeyBinding> for Span<'static> {
    fn from(binding: &KeyBinding) -> Self {
        Span::styled(binding.display_label(), key_hint_style())
    }
}

fn key_hint_style() -> Style {
    Style::default().dim()
}

pub(crate) fn has_ctrl_or_alt(mods: KeyModifiers) -> bool {
    (mods.contains(KeyModifiers::CONTROL) || mods.contains(KeyModifiers::ALT)) && !is_altgr(mods)
}

#[cfg(windows)]
#[inline]
pub(crate) fn is_altgr(mods: KeyModifiers) -> bool {
    mods.contains(KeyModifiers::ALT) && mods.contains(KeyModifiers::CONTROL)
}

#[cfg(not(windows))]
#[inline]
pub(crate) fn is_altgr(_mods: KeyModifiers) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_press_accepts_press_and_repeat_but_rejects_release() {
        let binding = ctrl(KeyCode::Char('k'));
        let press = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
        let repeat = KeyEvent {
            kind: KeyEventKind::Repeat,
            ..press
        };
        let release = KeyEvent {
            kind: KeyEventKind::Release,
            ..press
        };
        let wrong_modifiers = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);

        assert!(binding.is_press(press));
        assert!(binding.is_press(repeat));
        assert!(!binding.is_press(release));
        assert!(!binding.is_press(wrong_modifiers));
    }

    #[test]
    fn keybinding_list_ext_matches_any_binding() {
        let bindings = [plain(KeyCode::Char('a')), ctrl(KeyCode::Char('b'))];

        assert!(bindings.is_pressed(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)));
        assert!(bindings.is_pressed(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)));
        assert!(!bindings.is_pressed(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)));
    }

    #[test]
    fn shifted_letter_binding_matches_uppercase_char_events() {
        let binding = shift(KeyCode::Char('a'));

        assert!(binding.is_press(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT)));
        assert!(binding.is_press(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE)));
        assert!(binding.is_press(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT)));
    }

    #[test]
    fn shift_letter_binding_preserves_other_modifiers_with_uppercase_compat() {
        let binding = KeyBinding::new(
            KeyCode::Char('i'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );

        assert!(binding.is_press(KeyEvent::new(KeyCode::Char('I'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn shift_letter_binding_does_not_match_plain_lowercase_or_other_uppercase() {
        let binding = shift(KeyCode::Char('o'));

        assert!(!binding.is_press(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE)));
        assert!(!binding.is_press(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE)));
    }

    #[test]
    fn ctrl_letter_binding_matches_c0_control_char_events() {
        let binding = ctrl(KeyCode::Char('p'));

        assert!(binding.is_press(KeyEvent::new(KeyCode::Char('\u{0010}'), KeyModifiers::NONE)));
        assert!(!binding.is_press(KeyEvent::new(KeyCode::Char('\u{0010}'), KeyModifiers::ALT)));
    }

    #[test]
    fn ctrl_bindings_match_all_supported_c0_control_char_events() {
        let cases = [
            (' ', '\u{0000}'),
            ('a', '\u{0001}'),
            ('b', '\u{0002}'),
            ('c', '\u{0003}'),
            ('d', '\u{0004}'),
            ('e', '\u{0005}'),
            ('f', '\u{0006}'),
            ('g', '\u{0007}'),
            ('h', '\u{0008}'),
            ('i', '\u{0009}'),
            ('j', '\u{000a}'),
            ('k', '\u{000b}'),
            ('l', '\u{000c}'),
            ('m', '\u{000d}'),
            ('n', '\u{000e}'),
            ('o', '\u{000f}'),
            ('p', '\u{0010}'),
            ('q', '\u{0011}'),
            ('r', '\u{0012}'),
            ('s', '\u{0013}'),
            ('t', '\u{0014}'),
            ('u', '\u{0015}'),
            ('v', '\u{0016}'),
            ('w', '\u{0017}'),
            ('x', '\u{0018}'),
            ('y', '\u{0019}'),
            ('z', '\u{001a}'),
            ('4', '\u{001c}'),
            ('5', '\u{001d}'),
            ('6', '\u{001e}'),
            ('7', '\u{001f}'),
        ];

        for (ctrl_char, c0_char) in cases {
            assert!(
                ctrl(KeyCode::Char(ctrl_char))
                    .is_press(KeyEvent::new(KeyCode::Char(c0_char), KeyModifiers::NONE)),
                "expected raw C0 {c0_char:?} to match ctrl-{ctrl_char}"
            );
            assert!(
                !ctrl(KeyCode::Char(ctrl_char))
                    .is_press(KeyEvent::new(KeyCode::Char(c0_char), KeyModifiers::ALT)),
                "expected modified raw C0 {c0_char:?} not to match ctrl-{ctrl_char}"
            );
        }
    }

    #[test]
    fn ctrl_binding_does_not_match_ambiguous_c0_escape_or_delete() {
        assert!(
            !ctrl(KeyCode::Char('['))
                .is_press(KeyEvent::new(KeyCode::Char('\u{001b}'), KeyModifiers::NONE,))
        );
        assert!(
            !ctrl(KeyCode::Char('?'))
                .is_press(KeyEvent::new(KeyCode::Char('\u{007f}'), KeyModifiers::NONE,))
        );
    }

    #[test]
    fn history_search_ctrl_bindings_match_c0_control_char_events() {
        assert!(
            ctrl(KeyCode::Char('r'))
                .is_press(KeyEvent::new(KeyCode::Char('\u{0012}'), KeyModifiers::NONE))
        );
        assert!(
            ctrl(KeyCode::Char('s'))
                .is_press(KeyEvent::new(KeyCode::Char('\u{0013}'), KeyModifiers::NONE))
        );
    }

    #[test]
    fn ctrl_alt_sets_both_modifiers() {
        assert_eq!(
            ctrl_alt(KeyCode::Char('v')).parts(),
            (
                KeyCode::Char('v'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )
        );
    }

    #[test]
    fn has_ctrl_or_alt_checks_supported_modifier_combinations() {
        assert!(!has_ctrl_or_alt(KeyModifiers::NONE));
        assert!(has_ctrl_or_alt(KeyModifiers::CONTROL));
        assert!(has_ctrl_or_alt(KeyModifiers::ALT));

        #[cfg(windows)]
        assert!(!has_ctrl_or_alt(KeyModifiers::CONTROL | KeyModifiers::ALT));
        #[cfg(not(windows))]
        assert!(has_ctrl_or_alt(KeyModifiers::CONTROL | KeyModifiers::ALT));
    }
}
