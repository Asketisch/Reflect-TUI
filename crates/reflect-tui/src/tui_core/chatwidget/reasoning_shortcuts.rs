//! 步进活动模型推理力度的键盘快捷键。
//!
//! 主聊天界面把 `Alt+,` 和 `Alt+.` 当作对
//! 当前模型配置的微调。本模块把该行为与更庞大的 `ChatWidget` 按键
//! 分发器分离，同时继续复用与设置弹窗相同的
//! 模型选择和 Plan 模式范围路径。
//!
//! 快捷键状态机刻意保持狭窄：只有在没有 modal 或弹窗
//! 持有输入时才处理按键，它把未设置的推理锚定到
//! 当前模型预置的默认值，并且只遍历活动模型声明的力度。
//! 不支持的力度锚定到模型默认值，若默认值缺失则锚定到第一个
//! 声明的力度，然后按声明顺序步进。升高永远不静默越过 Max 或 Ultra；
//! 这些力度需要显式的进阶推理选择器。

use crate::protocol_compat::config_types::ModeKind;
use crate::protocol_compat::openai_models::ModelPreset;
use crate::protocol_compat::openai_models::ReasoningEffort as ReasoningEffortConfig;
use crossterm::event::KeyEvent;

use super::ChatWidget;
use super::PARENT_OWNED_INPUT_MESSAGE;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::key_hint::KeyBindingListExt;

/// 推理级快捷键请求的方向。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReasoningShortcutDirection {
    Lower,
    Raise,
}

impl ReasoningShortcutDirection {
    fn bound_message(self, effort: &ReasoningEffortConfig) -> String {
        let label = ChatWidget::reasoning_effort_sentence_label(effort);
        match self {
            Self::Lower => format!("Reasoning is already at the lowest level ({label})."),
            Self::Raise => format!("Reasoning is already at the highest level ({label})."),
        }
    }
}

impl ChatWidget {
    /// 在常规按键分发之前处理主界面的推理快捷键。
    ///
    /// 返回 `true` 表示该按键被识别为推理快捷键并已完全
    /// 处理，即使处理只是在边界产生了一条信息性消息。
    /// 返回 `false` 则把按键留给正常的聊天输入流，
    /// 这在弹窗或 modal 获得焦点时很重要。
    ///
    /// 调用方应通过本方法路由已识别的快捷键，而不是
    /// 直接变更推理状态。它应用常规模式变更而不
    /// 持久化。在 Plan 模式下，快捷键只应用于活动的
    /// Plan 模式覆盖项，并跳过全局与 Plan 作用域的提示。
    pub(super) fn handle_reasoning_shortcut(&mut self, key_event: KeyEvent) -> bool {
        let direction = if self
            .chat_keymap
            .decrease_reasoning_effort
            .is_pressed(key_event)
        {
            ReasoningShortcutDirection::Lower
        } else if self
            .chat_keymap
            .increase_reasoning_effort
            .is_pressed(key_event)
        {
            ReasoningShortcutDirection::Raise
        } else {
            return false;
        };

        if !self.bottom_pane.no_modal_or_popup_active() {
            return false;
        }

        if self.blocks_direct_input {
            self.add_error_message(PARENT_OWNED_INPUT_MESSAGE.to_string());
            return true;
        }

        if !self.is_session_configured() {
            self.add_info_message(
                "Reasoning shortcuts are disabled until startup completes.".to_string(),
                /*hint*/ None,
            );
            return true;
        }

        let current_model = self.current_model().to_string();
        let Some(preset) = self.current_model_preset() else {
            self.add_info_message(
                format!("Reasoning shortcuts are unavailable for {current_model}."),
                /*hint*/ None,
            );
            return true;
        };

        let choices = reasoning_choices(&preset);
        let configured_effort = self
            .effective_reasoning_effort()
            .unwrap_or_else(|| preset.default_reasoning_effort.clone());
        let current_effort = if choices.contains(&configured_effort) {
            configured_effort
        } else if choices.contains(&preset.default_reasoning_effort) {
            preset.default_reasoning_effort
        } else {
            choices
                .first()
                .cloned()
                .unwrap_or(preset.default_reasoning_effort)
        };
        let Some(next_effort) =
            next_reasoning_effort(&choices, Some(current_effort.clone()), direction)
        else {
            self.add_info_message(direction.bound_message(&current_effort), /*hint*/ None);
            return true;
        };

        if direction == ReasoningShortcutDirection::Raise
            && Self::is_advanced_reasoning_effort(&next_effort)
        {
            let advanced_label = choices
                .iter()
                .filter(|effort| Self::is_advanced_reasoning_effort(effort))
                .map(Self::reasoning_effort_label)
                .collect::<Vec<_>>()
                .join(" and ");
            let verb = if advanced_label.contains(" and ") {
                "are"
            } else {
                "is"
            };
            let model_path = if current_model.starts_with("reflect-auto-") {
                current_model
            } else {
                format!("All models → {current_model}")
            };
            self.add_info_message(
                format!(
                    "{advanced_label} {verb} available under /model → {model_path} → More reasoning…"
                ),
                /*hint*/ None,
            );
            return true;
        }

        if self.collaboration_modes_enabled() && self.active_mode_kind() == ModeKind::Plan {
            let warning = self.ultra_reasoning_concurrency_warning(&next_effort);
            self.app_event_tx
                .send(AppEvent::UpdatePlanModeReasoningEffort(Some(next_effort)));
            if let Some(warning) = warning {
                self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                    crate::tui_core::history_cell::new_warning_event(warning),
                )));
            }
        } else {
            self.app_event_tx
                .send(AppEvent::UpdateReasoningEffort(Some(next_effort)));
        }

        true
    }

    fn current_model_preset(&self) -> Option<ModelPreset> {
        let current_model = self.current_model();
        self.model_catalog
            .try_list_models()
            .ok()?
            .into_iter()
            .find(|preset| preset.model == current_model)
    }
}

fn reasoning_choices(preset: &ModelPreset) -> Vec<ReasoningEffortConfig> {
    let (mut choices, mut advanced_choices): (Vec<_>, Vec<_>) = preset
        .supported_reasoning_efforts
        .iter()
        .map(|option| option.effort.clone())
        .partition(|effort| !ChatWidget::is_advanced_reasoning_effort(effort));
    advanced_choices.sort_by_key(|effort| matches!(effort, ReasoningEffortConfig::Ultra));
    choices.extend(advanced_choices);
    if choices.is_empty() {
        choices.push(preset.default_reasoning_effort.clone());
    }
    choices
}

fn next_reasoning_effort(
    choices: &[ReasoningEffortConfig],
    current_effort: Option<ReasoningEffortConfig>,
    direction: ReasoningShortcutDirection,
) -> Option<ReasoningEffortConfig> {
    let current_effort = current_effort?;
    if let Some(current_index) = choices.iter().position(|choice| choice == &current_effort) {
        return match direction {
            ReasoningShortcutDirection::Lower => current_index
                .checked_sub(1)
                .and_then(|index| choices.get(index))
                .cloned(),
            ReasoningShortcutDirection::Raise => choices.get(current_index + 1).cloned(),
        };
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn next_reasoning_effort_raises_from_default_anchor() {
        let choices = vec![
            ReasoningEffortConfig::Low,
            ReasoningEffortConfig::Medium,
            ReasoningEffortConfig::High,
            ReasoningEffortConfig::XHigh,
        ];

        assert_eq!(
            next_reasoning_effort(
                &choices,
                Some(ReasoningEffortConfig::Medium),
                ReasoningShortcutDirection::Raise,
            ),
            Some(ReasoningEffortConfig::High)
        );
    }

    #[test]
    fn next_reasoning_effort_lowers_from_default_anchor() {
        let choices = vec![
            ReasoningEffortConfig::Low,
            ReasoningEffortConfig::Medium,
            ReasoningEffortConfig::High,
        ];

        assert_eq!(
            next_reasoning_effort(
                &choices,
                Some(ReasoningEffortConfig::Medium),
                ReasoningShortcutDirection::Lower,
            ),
            Some(ReasoningEffortConfig::Low)
        );
    }

    #[test]
    fn next_reasoning_effort_does_not_infer_position_for_unsupported_current() {
        let choices = vec![ReasoningEffortConfig::Low, ReasoningEffortConfig::High];

        assert_eq!(
            (
                next_reasoning_effort(
                    &choices,
                    Some(ReasoningEffortConfig::Medium),
                    ReasoningShortcutDirection::Raise,
                ),
                next_reasoning_effort(
                    &choices,
                    Some(ReasoningEffortConfig::Medium),
                    ReasoningShortcutDirection::Lower,
                ),
            ),
            (None, None)
        );
    }

    #[test]
    fn next_reasoning_effort_uses_advertised_order_for_custom_levels() {
        let custom_effort = ReasoningEffortConfig::Custom("future".to_string());
        let choices = vec![
            ReasoningEffortConfig::High,
            ReasoningEffortConfig::Low,
            custom_effort.clone(),
        ];

        assert_eq!(
            (
                next_reasoning_effort(
                    &choices,
                    Some(ReasoningEffortConfig::High),
                    ReasoningShortcutDirection::Raise,
                ),
                next_reasoning_effort(
                    &choices,
                    Some(custom_effort),
                    ReasoningShortcutDirection::Lower,
                ),
            ),
            (
                Some(ReasoningEffortConfig::Low),
                Some(ReasoningEffortConfig::Low),
            )
        );
    }

    #[test]
    fn next_reasoning_effort_clamps_at_bounds() {
        let choices = vec![
            ReasoningEffortConfig::Low,
            ReasoningEffortConfig::Medium,
            ReasoningEffortConfig::High,
        ];

        assert_eq!(
            next_reasoning_effort(
                &choices,
                Some(ReasoningEffortConfig::Low),
                ReasoningShortcutDirection::Lower,
            ),
            None
        );
        assert_eq!(
            next_reasoning_effort(
                &choices,
                Some(ReasoningEffortConfig::High),
                ReasoningShortcutDirection::Raise,
            ),
            None
        );
    }

    #[test]
    fn next_reasoning_effort_single_option_is_noop() {
        let choices = vec![ReasoningEffortConfig::High];

        assert_eq!(
            next_reasoning_effort(
                &choices,
                Some(ReasoningEffortConfig::High),
                ReasoningShortcutDirection::Raise,
            ),
            None
        );
        assert_eq!(
            next_reasoning_effort(
                &choices,
                Some(ReasoningEffortConfig::High),
                ReasoningShortcutDirection::Lower,
            ),
            None
        );
    }
}
