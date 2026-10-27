//! ChatComposer 纯函数辅助。从 chat_composer.rs 抽出。

use super::*;

/// 当用户输入超过允许的最大长度时显示的消息。
pub(super) fn user_input_too_large_message(actual_chars: usize) -> String {
    format!(
        "Message exceeds the maximum length of {MAX_USER_INPUT_TEXT_CHARS} characters ({actual_chars} provided)."
    )
}

/// 检查在父级拥有的线程(parent-owned thread)模式下是否允许某个斜杠命令。
///
/// 只允许不带参数(bare)的、属于导航/管理类的斜杠命令。
pub(super) fn parent_owned_command_is_allowed(command: SlashCommand, args: &str) -> bool {
    args.is_empty()
        && matches!(
            command,
            SlashCommand::Feedback
                | SlashCommand::New
                | SlashCommand::Clear
                | SlashCommand::Resume
                | SlashCommand::App
                | SlashCommand::Side
                | SlashCommand::Btw
                | SlashCommand::Agent
                | SlashCommand::MultiAgents
                | SlashCommand::Vim
                | SlashCommand::Keymap
                | SlashCommand::ElevateSandbox
                | SlashCommand::SandboxReadRoot
                | SlashCommand::Experimental
                | SlashCommand::Memories
                | SlashCommand::Quit
                | SlashCommand::Exit
                | SlashCommand::Logout
                | SlashCommand::Copy
                | SlashCommand::Raw
                | SlashCommand::Diff
                | SlashCommand::Mention
                | SlashCommand::Skills
                | SlashCommand::Import
                | SlashCommand::Hooks
                | SlashCommand::Status
                | SlashCommand::Usage
                | SlashCommand::Ide
                | SlashCommand::DebugConfig
                | SlashCommand::Title
                | SlashCommand::Statusline
                | SlashCommand::Theme
                | SlashCommand::Pets
                | SlashCommand::Ps
                | SlashCommand::Stop
                | SlashCommand::MemoryDrop
                | SlashCommand::MemoryUpdate
                | SlashCommand::Mcp
                | SlashCommand::Apps
                | SlashCommand::Plugins
                | SlashCommand::Rollout
        )
}

/// 构建一行提示,用于替换环境页脚而不增加布局高度。
pub(super) fn plan_mode_nudge_line() -> Line<'static> {
    Line::from(vec![
        "Create a plan?".magenta(),
        "  ".into(),
        key_hint::shift(KeyCode::Tab).into(),
        " use Plan mode".into(),
        "   ".into(),
        key_hint::plain(KeyCode::Esc).into(),
        " dismiss".into(),
    ])
}

/// 解析要在页脚中显示的插入换行键。
///
/// 当终端支持增强键(enhanced keys)且绑定中包含 Shift+Enter 时优先使用它;
/// 否则回退到第一个不是普通 Enter 的绑定,最后兜底使用第一个绑定。
pub(super) fn footer_insert_newline_key(
    bindings: &[KeyBinding],
    enhanced_keys_supported: bool,
) -> Option<KeyBinding> {
    let shift_enter = key_hint::shift(KeyCode::Enter);
    if enhanced_keys_supported && bindings.contains(&shift_enter) {
        return Some(shift_enter);
    }

    let plain_enter = key_hint::plain(KeyCode::Enter);
    bindings
        .iter()
        .copied()
        .find(|binding| *binding != plain_enter)
        .or_else(|| bindings.first().copied())
}

/// 从 skill 的元数据中提取非空描述。
pub(super) fn skill_description(skill: &SkillMetadata) -> Option<String> {
    let description = skill
        .interface
        .as_ref()
        .and_then(|interface| interface.short_description.as_deref())
        .or(skill.short_description.as_deref())
        .unwrap_or_else(|| skill.description.as_deref().unwrap_or(""));
    let trimmed = description.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
