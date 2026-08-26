//! keymap 绑定解析/验证子系统。从 keymap.rs 抽出。

use super::*;

/// 拒绝单个有效上下文映射中的重复按键。
///
/// 这有意允许同一按键出现在不同上下文中；处理器
/// 一次只求值一个上下文。
pub(super) fn validate_unique<const N: usize>(
    context: &str,
    pairs: [(&'static str, &[KeyBinding]); N],
) -> Result<(), String> {
    let mut seen: HashMap<(KeyCode, KeyModifiers), &'static str> = HashMap::new();
    for (action, bindings) in pairs {
        for binding in bindings {
            let key = binding.parts();
            if let Some(previous) = seen.insert(key, action) {
                return Err(format!(
                    "Ambiguous `tui.keymap.{context}` bindings: `{previous}` and `{action}` use the same key. \
Set unique keys in `~/.reflect/config.toml` and retry. \
See the Reflect keymap documentation for supported actions and examples."
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_no_shadow_with_allowed_overlaps<
    const N: usize,
    const M: usize,
    const A: usize,
>(
    context: &str,
    primary: [(&'static str, &[KeyBinding]); N],
    shadowed: [(&'static str, &[KeyBinding]); M],
    allowed_overlaps: [(&'static str, &'static str, KeyBinding); A],
) -> Result<(), String> {
    let mut seen: HashMap<(KeyCode, KeyModifiers), &'static str> = HashMap::new();
    for (action, bindings) in primary {
        for binding in bindings {
            seen.insert(binding.parts(), action);
        }
    }
    for (action, bindings) in shadowed {
        for binding in bindings {
            let key = binding.parts();
            if let Some(previous) = seen.get(&key) {
                if allowed_overlaps.iter().any(
                    |(allowed_primary, allowed_shadowed, allowed_binding)| {
                        *allowed_primary == *previous
                            && *allowed_shadowed == action
                            && allowed_binding.parts() == key
                    },
                ) {
                    continue;
                }
                return Err(format!(
                    "Ambiguous `tui.keymap.{context}` bindings: `{previous}` shadows `{action}` with the same key. \
Set unique keys in `~/.reflect/config.toml` and retry. \
See the Reflect keymap documentation for supported actions and examples."
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_no_reserved<const N: usize, const A: usize>(
    context: &str,
    pairs: [(&'static str, &[KeyBinding]); N],
    reserved: &[(&'static str, KeyBinding)],
    allowed_overlaps: [(&'static str, &'static str, KeyBinding); A],
) -> Result<(), String> {
    for (action, bindings) in pairs {
        for binding in bindings {
            let key = binding.parts();
            if let Some((reserved_action, _)) = reserved
                .iter()
                .find(|(_, reserved_binding)| reserved_binding.parts() == key)
            {
                if allowed_overlaps.iter().any(
                    |(allowed_action, allowed_reserved_action, allowed_binding)| {
                        *allowed_action == action
                            && *allowed_reserved_action == *reserved_action
                            && allowed_binding.parts() == key
                    },
                ) {
                    continue;
                }
                return Err(format!(
                    "Ambiguous `tui.keymap.{context}` bindings: `{action}` uses a key reserved by `{reserved_action}`. \
Set a different key in `~/.reflect/config.toml` and retry. \
See the Reflect keymap documentation for supported actions and examples."
                ));
            }
        }
    }
    Ok(())
}

pub(super) const MAIN_RESERVED_BINDINGS: &[(&str, KeyBinding)] = &[
    (
        "fixed.interrupt_or_quit",
        key_hint::ctrl(KeyCode::Char('c')),
    ),
    ("fixed.quit", key_hint::ctrl(KeyCode::Char('d'))),
    ("fixed.paste_image", key_hint::ctrl(KeyCode::Char('v'))),
    ("fixed.paste_image", key_hint::ctrl_alt(KeyCode::Char('v'))),
    (
        "fixed.cycle_collaboration_mode",
        key_hint::shift(KeyCode::Tab),
    ),
    ("fixed.backtrack", key_hint::plain(KeyCode::Esc)),
    ("fixed.previous_agent", key_hint::alt(KeyCode::Left)),
    ("fixed.next_agent", key_hint::alt(KeyCode::Right)),
    ("fixed.slash_command", key_hint::plain(KeyCode::Char('/'))),
    ("fixed.shell_command", key_hint::plain(KeyCode::Char('!'))),
    ("fixed.file_paths", key_hint::plain(KeyCode::Char('@'))),
    (
        "fixed.connector_mentions",
        key_hint::plain(KeyCode::Char('$')),
    ),
];

pub(super) const TRANSCRIPT_BACKTRACK_RESERVED_BINDINGS: &[(&str, KeyBinding)] = &[
    (
        "fixed.transcript_edit_previous",
        key_hint::plain(KeyCode::Esc),
    ),
    (
        "fixed.transcript_edit_previous",
        key_hint::plain(KeyCode::Left),
    ),
    (
        "fixed.transcript_edit_next",
        key_hint::plain(KeyCode::Right),
    ),
    (
        "fixed.transcript_confirm_edit",
        key_hint::plain(KeyCode::Enter),
    ),
];

/// 按 上下文 -> 全局 -> 内置默认 的优先级解析一个动作。
///
/// `path` 应是上下文特定的配置路径，这样解析错误能
/// 把用户指向他们尝试设置的覆盖项。
///
/// 配置的空列表具有权威性：它返回空绑定集合，
/// 且不会继续回退到全局或内置默认。这正是 composer 提交等
/// 全局可复用动作能够显式解绑的原理。
pub(super) fn resolve_bindings_with_global_fallback(
    configured: Option<&KeybindingsSpec>,
    global: Option<&KeybindingsSpec>,
    fallback: &[KeyBinding],
    path: &str,
) -> Result<Vec<KeyBinding>, String> {
    if let Some(configured) = configured {
        return parse_bindings(configured, path);
    }
    if let Some(global) = global {
        return parse_bindings(global, path);
    }
    Ok(fallback.to_vec())
}

/// 在没有全局回退时解析上下文中的一个动作绑定。
///
/// 缺失值继承内置默认；配置的值（包括空列表）
/// 会为该动作替换默认值。
pub(super) fn resolve_bindings(
    configured: Option<&KeybindingsSpec>,
    fallback: &[KeyBinding],
    path: &str,
) -> Result<Vec<KeyBinding>, String> {
    let Some(spec) = configured else {
        return Ok(fallback.to_vec());
    };
    parse_bindings(spec, path)
}

pub(super) fn configured_bindings_to_preserve<const N: usize>(
    pairs: [(Option<&KeybindingsSpec>, &[KeyBinding]); N],
) -> Vec<KeyBinding> {
    let mut configured_bindings = Vec::new();
    for (configured, resolved) in pairs {
        if configured.is_none() {
            continue;
        }
        for binding in resolved {
            if !configured_bindings.contains(binding) {
                configured_bindings.push(*binding);
            }
        }
    }
    configured_bindings
}

pub(super) fn configured_main_surface_alias_is_used(keymap: &TuiKeymap, alias: &str) -> bool {
    let mut global = keymap.global.clone();
    if keymap.composer.submit.is_some() {
        global.submit = None;
    }
    if keymap.composer.queue.is_some() {
        global.queue = None;
    }
    if keymap.composer.toggle_shortcuts.is_some() {
        global.toggle_shortcuts = None;
    }

    // 推理快捷键在 composer/编辑器按键处理之前运行，因此回退
    // 别名必须让位于同一主界面输入路径上的任何显式绑定。
    configured_context_alias_is_used(&global, alias)
        || configured_context_alias_is_used(&keymap.chat, alias)
        || configured_context_alias_is_used(&keymap.composer, alias)
        || configured_context_alias_is_used(&keymap.editor, alias)
        || configured_context_alias_is_used(&keymap.vim_normal, alias)
        || configured_context_alias_is_used(&keymap.vim_operator, alias)
        || configured_context_alias_is_used(&keymap.vim_text_object, alias)
}

pub(super) fn configured_context_alias_is_used(context: &impl Serialize, alias: &str) -> bool {
    let Ok(value) = serde_json::to_value(context) else {
        return false;
    };
    keymap_value_contains_alias(&value, alias)
}

pub(super) fn keymap_value_contains_alias(value: &serde_json::Value, alias: &str) -> bool {
    match value {
        serde_json::Value::String(value) => value == alias,
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| keymap_value_contains_alias(value, alias)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| keymap_value_contains_alias(value, alias)),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) | serde_json::Value::Null => {
            false
        }
    }
}

pub(super) fn resolve_new_default_bindings(
    configured: Option<&KeybindingsSpec>,
    fallback: &[KeyBinding],
    configured_bindings_to_preserve: &[KeyBinding],
    path: &str,
) -> Result<Vec<KeyBinding>, String> {
    let Some(spec) = configured else {
        return Ok(fallback
            .iter()
            .copied()
            .filter(|binding| !configured_bindings_to_preserve.contains(binding))
            .collect());
    };
    parse_bindings(spec, path)
}

/// 将单个按键绑定值（`string` 或 `list[string]`）解析为具体绑定。
///
/// 重复条目在保持首次出现顺序的同时被去重，使
/// 第一个按键能继续作为主 UI 提示。
pub(super) fn parse_bindings(
    spec: &KeybindingsSpec,
    path: &str,
) -> Result<Vec<KeyBinding>, String> {
    let mut parsed = Vec::new();
    for raw in spec.specs() {
        let binding = parse_keybinding(raw.as_str()).ok_or_else(|| {
            format!(
                "Invalid `{path}` = `{}`. Use values like `ctrl-a`, `shift-enter`, or `page-down`. \
See the Reflect keymap documentation for supported actions and examples.",
                raw.as_str()
            )
        })?;

        if !parsed.contains(&binding) {
            parsed.push(binding);
        }
    }
    Ok(parsed)
}

/// 解析一条已归一化的按键绑定规格，例如 `ctrl-a` 或 `shift-enter`。
///
/// 规格应已由配置反序列化归一化，但此解析器
/// 仍保持严格，以保证运行时错误信息精确。
pub(super) fn parse_keybinding(spec: &str) -> Option<KeyBinding> {
    let mut parts = spec.split('-');
    let mut modifiers = KeyModifiers::NONE;
    let mut key_name = None;

    for part in parts.by_ref() {
        match part {
            "ctrl" => modifiers |= KeyModifiers::CONTROL,
            "alt" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            other => {
                key_name = Some(other.to_string());
                break;
            }
        }
    }

    let mut key_name = key_name?;
    for trailing in parts {
        key_name.push('-');
        key_name.push_str(trailing);
    }

    let key = match key_name.as_str() {
        "enter" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "esc" => KeyCode::Esc,
        "delete" => KeyCode::Delete,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "page-up" => KeyCode::PageUp,
        "page-down" => KeyCode::PageDown,
        "space" => KeyCode::Char(' '),
        "minus" => KeyCode::Char('-'),
        other if other.len() == 1 => KeyCode::Char(char::from(other.as_bytes()[0])),
        other if other.starts_with('f') => {
            let number = other[1..].parse::<u8>().ok()?;
            if (1..=MAX_FUNCTION_KEY).contains(&(number as u32)) {
                KeyCode::F(number)
            } else {
                return None;
            }
        }
        _ => return None,
    };

    Some(KeyBinding::new(key, modifiers))
}
