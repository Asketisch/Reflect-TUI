//! 为 `/keymap` 提供的引导式键盘重映射 UI。
//!
//! 此模块拥有从解析的 [`RuntimeKeymap`] 开始的交互式编辑流程，
//! 并生成新的根级 [`TuiKeymap`] 覆盖。选择器和操作菜单向用户显示当前活动绑定，
//! 可能来自默认值、全局回退或显式配置，而写入始终
//! 指向用户选择的具体 `tui.keymap.<context>.<action>` 槽位。
//!
//! 此流程故意分为三个步骤：选择操作，选择
//! 替换/添加/移除绑定，然后精确捕获一个终端
//! 按键事件。验证在捕获后通过复用运行时键盘映射
//! 解析进行，因此冲突规则保持在 `keymap.rs` 中集中，而不是
//! 在 UI 中重复。
//!
//! 此模块不直接持久化配置文件。它发出带有
//! 已编辑配置的应用事件，因此应用层可以决定如何保存、重新加载和
//! 表面错误。

mod actions;
mod debug;
mod picker;

pub(crate) use actions::KeymapActionFilter;
pub(crate) use debug::build_keymap_debug_view;
pub(crate) use picker::KEYMAP_PICKER_VIEW_ID;
#[cfg(test)]
pub(crate) use picker::build_keymap_picker_params;
#[cfg(test)]
pub(crate) use picker::build_keymap_picker_params_for_selected_action;
pub(crate) use picker::build_keymap_picker_params_for_selected_action_with_filter;
pub(crate) use picker::build_keymap_picker_params_with_filter;

use crate::config_compat::types::KeybindingSpec;
use crate::config_compat::types::KeybindingsSpec;
use crate::config_compat::types::MAX_FUNCTION_KEY;
use crate::config_compat::types::TuiKeymap;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event::KeymapEditIntent;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::bottom_pane::BottomPaneView;
use crate::tui_core::bottom_pane::CancellationEvent;
use crate::tui_core::bottom_pane::ColumnWidthMode;
use crate::tui_core::bottom_pane::SelectionItem;
use crate::tui_core::bottom_pane::SelectionViewParams;
use crate::tui_core::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::keymap::RuntimeKeymap;
use crate::tui_core::render::renderable::ColumnRenderable;
use crate::tui_core::render::renderable::Renderable;
use actions::KEYMAP_ACTIONS;
use actions::action_label;
use actions::binding_slot;
use actions::bindings_for_action;
use actions::format_binding_summary;
#[cfg(test)]
use debug::KeymapDebugView;

pub(crate) const KEYMAP_ACTION_MENU_VIEW_ID: &str = "keymap-action-menu";
pub(crate) const KEYMAP_REPLACE_BINDING_MENU_VIEW_ID: &str = "keymap-replace-binding-menu";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum KeymapEditOutcome {
    /// 此次编辑生成了新的配置快照以及面向用户的状态消息。
    Updated {
        keymap_config: Box<TuiKeymap>,
        bindings: Vec<String>,
        message: String,
    },
    /// 请求的编辑解析后得到的有效绑定集合与原来相同。
    Unchanged { message: String },
}

fn key_binding_span(binding: &str) -> ratatui::text::Span<'static> {
    if binding == "unbound" {
        binding.to_string().dim()
    } else {
        binding.to_string().cyan()
    }
}

fn keymap_action_menu_hint_line() -> Line<'static> {
    Line::from(vec![
        "enter".cyan(),
        " select · ".dim(),
        "esc".cyan(),
        " back".dim(),
    ])
}

fn open_capture_action(
    context: String,
    action: String,
    intent: KeymapEditIntent,
) -> Box<dyn Fn(&AppEventSender) + Send + Sync> {
    Box::new(move |tx| {
        tx.send(AppEvent::OpenKeymapCapture {
            context: context.clone(),
            action: action.clone(),
            intent: intent.clone(),
        });
    })
}

fn action_menu_item(
    name: &str,
    description: &str,
    selected_description: String,
    context: &str,
    action: &str,
    intent: KeymapEditIntent,
) -> SelectionItem {
    SelectionItem {
        name: name.to_string(),
        description: Some(description.to_string()),
        selected_description: Some(selected_description),
        actions: vec![open_capture_action(
            context.to_string(),
            action.to_string(),
            intent,
        )],
        ..Default::default()
    }
}

/// 在用户选择快捷键行后构建特定操作的菜单。
///
/// 菜单基于活动运行时绑定和根配置状态：
/// 活动绑定决定替换/添加选项是否可用，而
/// 配置状态决定"移除自定义绑定"是否可以恢复回退
/// 行为。传递过期的上下文/操作字符串会产生通用回退
/// 菜单而不是 panic，因为选择视图可以比配置重新加载更长寿。
pub(crate) fn build_keymap_action_menu_params(
    context: String,
    action: String,
    runtime_keymap: &RuntimeKeymap,
    keymap_config: &TuiKeymap,
) -> SelectionViewParams {
    let current_bindings =
        active_binding_specs(runtime_keymap, &context, &action).unwrap_or_else(|_| Vec::new());
    let current_binding = if current_bindings.is_empty() {
        "unbound".to_string()
    } else {
        current_bindings.join(", ")
    };
    let active_binding_count = current_bindings.len();
    let custom_binding = has_custom_binding(keymap_config, &context, &action).unwrap_or(false);
    let descriptor = KEYMAP_ACTIONS
        .iter()
        .find(|descriptor| descriptor.context == context && descriptor.action == action);
    let context_label = descriptor
        .map(|descriptor| descriptor.context_label)
        .unwrap_or(context.as_str())
        .to_string();
    let description = descriptor
        .map(|descriptor| descriptor.description)
        .unwrap_or("Configure this shortcut.");
    let remove_disabled_reason = (!custom_binding)
        .then(|| "There is no custom root binding for this action to remove.".to_string());
    let label = action_label(&action);
    let remove_context = context.clone();
    let remove_action = action.clone();
    let config_path = format!("tui.keymap.{context}.{action}");
    let source = if custom_binding {
        "Custom root override".cyan()
    } else {
        "Default keymap".dim()
    };
    let mut header = ColumnRenderable::new();
    header.push(Line::from("Edit Shortcut".bold()));
    header.push(Line::from(vec![
        label.bold(),
        " · ".dim(),
        context_label.dim(),
    ]));
    header.push(Line::from(vec![
        "Current ".dim(),
        key_binding_span(&current_binding),
        " · ".dim(),
        source,
    ]));
    header.push(Line::from(vec![
        "Config ".dim(),
        format!("`{config_path}`").cyan(),
    ]));
    header.push(Line::from(description.to_string().dim()));

    let mut items = Vec::new();
    match active_binding_count {
        0 => {
            items.push(action_menu_item(
                "Set key",
                "Capture a key for this unbound action.",
                "Capture one key and bind this action.".to_string(),
                &context,
                &action,
                KeymapEditIntent::ReplaceAll,
            ));
        }
        1 => {
            items.push(action_menu_item(
                "Replace binding",
                "Capture a replacement key.",
                format!("Capture one key and replace `{current_binding}`."),
                &context,
                &action,
                KeymapEditIntent::ReplaceAll,
            ));
            items.push(action_menu_item(
                "Add alternate binding",
                "Keep the current binding and add another key.",
                format!("Capture one key and keep `{current_binding}` as an alternate."),
                &context,
                &action,
                KeymapEditIntent::AddAlternate,
            ));
        }
        _ => {
            let replace_one_context = context.clone();
            let replace_one_action = action.clone();
            items.push(SelectionItem {
                name: "Replace one binding...".to_string(),
                description: Some("Choose which existing binding to replace.".to_string()),
                selected_description: Some(
                    "Pick one current binding, then capture its replacement.".to_string(),
                ),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenKeymapReplaceBindingMenu {
                        context: replace_one_context.clone(),
                        action: replace_one_action.clone(),
                    });
                })],
                ..Default::default()
            });
            items.push(action_menu_item(
                "Replace all bindings",
                "Replace every current binding with one key.",
                format!("Capture one key and replace `{current_binding}`."),
                &context,
                &action,
                KeymapEditIntent::ReplaceAll,
            ));
            items.push(action_menu_item(
                "Add alternate binding",
                "Keep current bindings and add another key.",
                format!("Capture one key and keep `{current_binding}`."),
                &context,
                &action,
                KeymapEditIntent::AddAlternate,
            ));
        }
    }
    items.push(SelectionItem {
        name: "Remove custom binding".to_string(),
        description: Some(if custom_binding {
            "Restore the default keymap binding.".to_string()
        } else {
            "No root override to remove.".to_string()
        }),
        selected_description: Some(
            "Delete the root override and use the default keymap again.".to_string(),
        ),
        disabled_reason: remove_disabled_reason,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::KeymapCleared {
                context: remove_context.clone(),
                action: remove_action.clone(),
            });
        })],
        ..Default::default()
    });
    items.push(SelectionItem {
        name: "Back to shortcuts".to_string(),
        description: Some("Return to the shortcut list.".to_string()),
        dismiss_on_select: true,
        ..Default::default()
    });

    SelectionViewParams {
        view_id: Some(KEYMAP_ACTION_MENU_VIEW_ID),
        header: Box::new(header),
        footer_note: Some(Line::from(vec![
            "Changes write the root ".dim(),
            "`tui.keymap.*`".cyan(),
            " override.".dim(),
        ])),
        footer_hint: Some(keymap_action_menu_hint_line()),
        items,
        col_width_mode: ColumnWidthMode::Fixed,
        ..Default::default()
    }
}

pub(crate) fn build_keymap_replace_binding_menu_params(
    context: String,
    action: String,
    runtime_keymap: &RuntimeKeymap,
) -> SelectionViewParams {
    let bindings = active_binding_specs(runtime_keymap, &context, &action).unwrap_or_default();
    let label = action_label(&action);
    let mut header = ColumnRenderable::new();
    header.push(Line::from("Replace Binding".bold()));
    header.push(Line::from(vec![
        label.bold(),
        " · ".dim(),
        format!("{context}.{action}").dim(),
    ]));
    header.push(Line::from("Choose the binding to replace.".dim()));

    let items = bindings
        .into_iter()
        .map(|binding| {
            let capture_context = context.clone();
            let capture_action = action.clone();
            let old_key = binding.clone();
            SelectionItem {
                name: binding.clone(),
                description: Some("Replace this binding.".to_string()),
                selected_description: Some(format!("Capture a new key to replace `{binding}`.")),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenKeymapCapture {
                        context: capture_context.clone(),
                        action: capture_action.clone(),
                        intent: KeymapEditIntent::ReplaceOne {
                            old_key: old_key.clone(),
                        },
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            }
        })
        .collect();

    SelectionViewParams {
        view_id: Some(KEYMAP_REPLACE_BINDING_MENU_VIEW_ID),
        header: Box::new(header),
        footer_hint: Some(keymap_action_menu_hint_line()),
        items,
        col_width_mode: ColumnWidthMode::Fixed,
        ..Default::default()
    }
}

pub(crate) fn build_keymap_conflict_params(
    context: String,
    action: String,
    key: String,
    intent: KeymapEditIntent,
    error: String,
) -> SelectionViewParams {
    let retry_context = context.clone();
    let retry_action = action.clone();
    let retry_intent = intent;
    SelectionViewParams {
        title: Some("Shortcut Conflict".to_string()),
        subtitle: Some(format!("{context}.{action} cannot use `{key}`.")),
        footer_note: Some(Line::from(error)),
        footer_hint: Some(standard_popup_hint_line()),
        items: vec![
            SelectionItem {
                name: "Pick another key".to_string(),
                description: Some("Return to key capture for this action.".to_string()),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenKeymapCapture {
                        context: retry_context.clone(),
                        action: retry_action.clone(),
                        intent: retry_intent.clone(),
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Cancel".to_string(),
                description: Some("Leave keymap unchanged.".to_string()),
                dismiss_on_select: true,
                ..Default::default()
            },
        ],
        col_width_mode: ColumnWidthMode::Fixed,
        ..Default::default()
    }
}

/// 为所选键盘映射编辑构建临时捕获视图。
///
/// 该视图显示来自最新运行时映射的当前绑定摘要，
/// 然后将捕获的键委托回应用程序事件循环。未知
/// 操作呈现为未绑定，以便最终的编辑路径可以报告
/// 过时的选择并提供精确的错误。
pub(crate) fn build_keymap_capture_view(
    context: String,
    action: String,
    intent: KeymapEditIntent,
    runtime_keymap: &RuntimeKeymap,
    app_event_tx: AppEventSender,
) -> KeymapCaptureView {
    let current_binding = format_binding_summary(
        bindings_for_action(runtime_keymap, &context, &action).unwrap_or(&[]),
    );
    let label = action_label(&action);
    KeymapCaptureView::new(
        context,
        action,
        intent,
        label,
        current_binding,
        app_event_tx,
    )
}

#[cfg(test)]
fn keymap_with_replacement(
    keymap: &TuiKeymap,
    context: &str,
    action: &str,
    key: &str,
) -> Result<TuiKeymap, String> {
    keymap_with_bindings(keymap, context, action, &[key.to_string()])
}

/// 将捕获的键应用于一个操作并返回编辑的根配置。
///
/// 当前有效的绑定来自 `runtime_keymap`，因此添加
/// 仅默认操作的替代键首先将这些默认值实例化到
/// 根配置中，然后再附加捕获的键。替换绑定通过要求所选的 `old_key` 仍然处于活动状态来防止
/// 过时菜单；否则用户可能会覆盖在菜单打开后更改的绑定。
pub(crate) fn keymap_with_edit(
    keymap: &TuiKeymap,
    runtime_keymap: &RuntimeKeymap,
    context: &str,
    action: &str,
    key: &str,
    intent: &KeymapEditIntent,
) -> Result<KeymapEditOutcome, String> {
    let current_bindings = active_binding_specs(runtime_keymap, context, action)?;
    let next_bindings = match intent {
        KeymapEditIntent::ReplaceAll => vec![key.to_string()],
        KeymapEditIntent::AddAlternate => {
            if current_bindings.iter().any(|binding| binding == key) {
                return Ok(KeymapEditOutcome::Unchanged {
                    message: format!("No change: `{context}.{action}` already uses `{key}`."),
                });
            }
            let mut bindings = current_bindings.clone();
            bindings.push(key.to_string());
            bindings
        }
        KeymapEditIntent::ReplaceOne { old_key } => {
            if !current_bindings.iter().any(|binding| binding == old_key) {
                return Err(format!(
                    "`{context}.{action}` no longer uses `{old_key}`. Reopen /keymap and choose a binding again."
                ));
            }
            let bindings = current_bindings
                .iter()
                .map(|binding| {
                    if binding == old_key {
                        key.to_string()
                    } else {
                        binding.clone()
                    }
                })
                .collect::<Vec<_>>();
            dedup_bindings(bindings)
        }
    };

    if next_bindings == current_bindings {
        return Ok(KeymapEditOutcome::Unchanged {
            message: format!("No change: `{context}.{action}` already uses `{key}`."),
        });
    }

    let message = match intent {
        KeymapEditIntent::ReplaceAll => format!("Remapped `{context}.{action}` to `{key}`."),
        KeymapEditIntent::AddAlternate => format!("Added `{key}` to `{context}.{action}`."),
        KeymapEditIntent::ReplaceOne { old_key } => {
            format!("Replaced `{old_key}` with `{key}` for `{context}.{action}`.")
        }
    };

    Ok(KeymapEditOutcome::Updated {
        keymap_config: Box::new(keymap_with_bindings(
            keymap,
            context,
            action,
            &next_bindings,
        )?),
        bindings: next_bindings,
        message,
    })
}

fn keymap_with_bindings(
    keymap: &TuiKeymap,
    context: &str,
    action: &str,
    keys: &[String],
) -> Result<TuiKeymap, String> {
    let mut keymap = keymap.clone();
    let slot = binding_slot(&mut keymap, context, action).ok_or_else(|| {
        format!("Unknown keymap action `{context}.{action}`. Reopen /keymap and choose an action.")
    })?;
    *slot = Some(match keys {
        [key] => KeybindingsSpec::One(KeybindingSpec(key.clone())),
        keys => KeybindingsSpec::Many(
            keys.iter()
                .map(|key| KeybindingSpec(key.clone()))
                .collect::<Vec<_>>(),
        ),
    });
    Ok(keymap)
}

/// 返回一个运行时操作的活动配置键规范。
///
/// 这将解析的 [`crate::tui_core::key_hint::KeyBinding`] 值转换回
/// 规范配置字符串以进行显示和需要保留
/// 现有绑定的编辑操作。调用者应将错误视为过时的 UI 状态，
/// 因为有效的菜单项应始终指向已知的操作。
pub(crate) fn active_binding_specs(
    runtime_keymap: &RuntimeKeymap,
    context: &str,
    action: &str,
) -> Result<Vec<String>, String> {
    let bindings = bindings_for_action(runtime_keymap, context, action).ok_or_else(|| {
        format!("Unknown keymap action `{context}.{action}`. Reopen /keymap and choose an action.")
    })?;
    bindings
        .iter()
        .map(|binding| binding_to_config_key_spec(*binding))
        .collect()
}

fn dedup_bindings(bindings: Vec<String>) -> Vec<String> {
    bindings.into_iter().fold(Vec::new(), |mut deduped, key| {
        if !deduped.contains(&key) {
            deduped.push(key);
        }
        deduped
    })
}

/// 移除一个操作的根级自定义绑定。
///
/// 用 `None` 清除槽位与设置空绑定列表不同：
/// `None` 恢复默认/全局回退行为，而空列表
/// 在运行时解析中显式解除操作的绑定。
pub(crate) fn keymap_without_custom_binding(
    keymap: &TuiKeymap,
    context: &str,
    action: &str,
) -> Result<TuiKeymap, String> {
    let mut keymap = keymap.clone();
    let slot = binding_slot(&mut keymap, context, action).ok_or_else(|| {
        format!("Unknown keymap action `{context}.{action}`. Reopen /keymap and choose an action.")
    })?;
    *slot = None;
    Ok(keymap)
}

fn has_custom_binding(keymap: &TuiKeymap, context: &str, action: &str) -> Result<bool, String> {
    let mut keymap = keymap.clone();
    let slot = binding_slot(&mut keymap, context, action).ok_or_else(|| {
        format!("Unknown keymap action `{context}.{action}`. Reopen /keymap and choose an action.")
    })?;
    Ok(slot.is_some())
}

/// 捕获单个按键事件以待处理 `/keymap` 编辑的底部窗格视图。
///
/// 视图故意是短暂的：它呈现指令，接受一个
/// 按键，并将捕获的按键发送到应用层。它不会突变
/// 配置本身，因为突变需要最新的运行时键盘映射来检测
/// 冲突和过时选择。
pub(crate) struct KeymapCaptureView {
    context: String,
    action: String,
    intent: KeymapEditIntent,
    label: String,
    current_binding: String,
    app_event_tx: AppEventSender,
    complete: bool,
    error_message: Option<String>,
}

impl KeymapCaptureView {
    fn new(
        context: String,
        action: String,
        intent: KeymapEditIntent,
        label: String,
        current_binding: String,
        app_event_tx: AppEventSender,
    ) -> Self {
        Self {
            context,
            action,
            intent,
            label,
            current_binding,
            app_event_tx,
            complete: false,
            error_message: None,
        }
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let wrap_width = usize::from(width.max(1));
        let mut lines = vec![
            Line::from("Remap Shortcut".bold()),
            Line::from(vec![
                "Action: ".dim(),
                self.label.clone().into(),
                "  ".into(),
                format!("{}.{}", self.context, self.action).dim(),
            ]),
            Line::from(vec!["Current: ".dim(), self.current_binding.clone().cyan()]),
            Line::from("Press the new key now. Esc cancels.".dim()),
        ];

        if let Some(error) = &self.error_message {
            lines.push(Line::from(""));
            let options = textwrap::Options::new(wrap_width)
                .initial_indent("Error: ")
                .subsequent_indent("       ");
            lines.extend(
                textwrap::wrap(error, options)
                    .into_iter()
                    .map(|line| Line::from(line.into_owned().red())),
            );
        }

        lines
    }
}

impl Renderable for KeymapCaptureView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.lines(area.width)).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.lines(width).len() as u16
    }
}

impl BottomPaneView for KeymapCaptureView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Release {
            return;
        }

        if key_event.code == KeyCode::Esc {
            self.complete = true;
            return;
        }

        match key_event_to_config_key_spec(key_event) {
            Ok(key) => {
                self.app_event_tx.send(AppEvent::KeymapCaptured {
                    context: self.context.clone(),
                    action: self.action.clone(),
                    key,
                    intent: self.intent.clone(),
                });
                self.complete = true;
            }
            Err(error) => {
                self.error_message = Some(error);
            }
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        true
    }
}

fn key_event_to_config_key_spec(key_event: KeyEvent) -> Result<String, String> {
    binding_to_config_key_spec(KeyBinding::from_event(key_event))
}

fn binding_to_config_key_spec(binding: KeyBinding) -> Result<String, String> {
    let (code, modifiers) = binding.parts();
    key_parts_to_config_key_spec(code, modifiers)
}

fn key_parts_to_config_key_spec(
    code: KeyCode,
    mut modifiers: KeyModifiers,
) -> Result<String, String> {
    let (code, normalized_modifiers) =
        crate::tui_core::key_hint::normalize_key_parts(code, modifiers);
    modifiers = normalized_modifiers;

    let supported_modifiers = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT;
    if !modifiers.difference(supported_modifiers).is_empty() {
        return Err(
            "Only ctrl, alt, and shift modifiers can be stored in `tui.keymap`.".to_string(),
        );
    }

    let key = match code {
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Esc => "esc".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "page-up".to_string(),
        KeyCode::PageDown => "page-down".to_string(),
        KeyCode::F(number) if (1..=MAX_FUNCTION_KEY).contains(&(number as u32)) => {
            format!("f{number}")
        }
        KeyCode::F(_) => {
            return Err(format!(
                "Only function keys F1 through F{MAX_FUNCTION_KEY} can be stored in `tui.keymap`."
            ));
        }
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(mut ch) => {
            if ch == '-' {
                return Ok(format_key_spec(modifiers, "minus"));
            }
            if !ch.is_ascii() || ch.is_ascii_control() {
                return Err("Only printable ASCII keys can be stored in `tui.keymap`.".to_string());
            }
            if ch.is_ascii_uppercase() {
                modifiers.insert(KeyModifiers::SHIFT);
                ch = ch.to_ascii_lowercase();
            }
            ch.to_string()
        }
        _ => {
            return Err("That key is not supported by `tui.keymap`.".to_string());
        }
    };

    Ok(format_key_spec(modifiers, &key))
}

fn format_key_spec(modifiers: KeyModifiers, key: &str) -> String {
    let mut parts = Vec::new();
    if modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift");
    }
    parts.push(key);
    parts.join("-")
}

#[cfg(test)]
#[cfg(test)]
mod tests;
