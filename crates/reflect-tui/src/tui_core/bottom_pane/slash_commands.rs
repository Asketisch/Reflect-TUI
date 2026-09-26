//! 用于过滤和匹配内置及模型服务层级 slash 命令的共享辅助函数。
//!
//! composer 和命令弹窗使用相同的沙箱及功能门控规则。
//! 将它们集中于此可使这些调用点保持精简，
//! 并确保它们保持同步。
use std::str::FromStr;

use crate::utils_fuzzy_match::fuzzy_match;

use crate::tui_core::slash_command::SlashCommand;
use crate::tui_core::slash_command::built_in_slash_commands;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceTierCommand {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
}

/// 插件 slash 命令弹窗条目(bootstrap 期从插件命令注册表快照)。
///
/// `name` 是命令全名(如 `demo:hello`,不带前导 `/`),与
/// `reflect_plugin::expand_user_input` 的查找键一致;`description`
/// 来自命令 md frontmatter,可为空。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PluginCommandEntry {
    pub(crate) name: String,
    pub(crate) description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SlashCommandItem {
    Builtin(SlashCommand),
    ServiceTier(ServiceTierCommand),
    /// 插件命令(bootstrap 期快照)。派发走纯文本提交 →
    /// `dispatch_slash` 未命中 → `SubmitAsUser` → 提交前由
    /// `expand_plugin_submission` 展开为命令 md 正文。
    Plugin(PluginCommandEntry),
}

impl SlashCommandItem {
    pub(crate) fn command(&self) -> &str {
        match self {
            Self::Builtin(cmd) => cmd.command(),
            Self::ServiceTier(command) => &command.name,
            Self::Plugin(entry) => &entry.name,
        }
    }

    pub(crate) fn supports_inline_args(&self) -> bool {
        match self {
            Self::Builtin(cmd) => cmd.supports_inline_args(),
            Self::ServiceTier(_) => false,
            // 插件命令的参数语义是「名字后的全文当 $ARGUMENTS」,
            // 由文本提交路径自然携带,不走 builtin inline-args 补全。
            Self::Plugin(_) => false,
        }
    }

    pub(crate) fn available_in_side_conversation(&self) -> bool {
        match self {
            Self::Builtin(cmd) => cmd.available_in_side_conversation(),
            Self::ServiceTier(_) => false,
            Self::Plugin(_) => true,
        }
    }

    pub(crate) fn available_during_task(&self) -> bool {
        match self {
            Self::Builtin(cmd) => cmd.available_during_task(),
            Self::ServiceTier(_) => true,
            Self::Plugin(_) => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BuiltinCommandFlags {
    pub(crate) collaboration_modes_enabled: bool,
    pub(crate) connectors_enabled: bool,
    pub(crate) plugins_command_enabled: bool,
    pub(crate) token_activity_command_enabled: bool,
    pub(crate) service_tier_commands_enabled: bool,
    pub(crate) goal_command_enabled: bool,
    pub(crate) personality_command_enabled: bool,
    pub(crate) allow_elevate_sandbox: bool,
    pub(crate) side_conversation_active: bool,
}

/// 返回当前输入下应可见/可用的内置命令。
pub(crate) fn builtins_for_input(flags: BuiltinCommandFlags) -> Vec<(&'static str, SlashCommand)> {
    built_in_slash_commands()
        .into_iter()
        .filter(|(_, cmd)| flags.allow_elevate_sandbox || *cmd != SlashCommand::ElevateSandbox)
        .filter(|(_, cmd)| flags.collaboration_modes_enabled || *cmd != SlashCommand::Plan)
        .filter(|(_, cmd)| flags.connectors_enabled || *cmd != SlashCommand::Apps)
        .filter(|(_, cmd)| flags.plugins_command_enabled || *cmd != SlashCommand::Plugins)
        .filter(|(_, cmd)| flags.token_activity_command_enabled || *cmd != SlashCommand::Usage)
        .filter(|(_, cmd)| flags.goal_command_enabled || *cmd != SlashCommand::Goal)
        .filter(|(_, cmd)| flags.personality_command_enabled || *cmd != SlashCommand::Personality)
        .filter(|(_, cmd)| !flags.side_conversation_active || cmd.available_in_side_conversation())
        .collect()
}

pub(crate) fn commands_for_input(
    flags: BuiltinCommandFlags,
    service_tier_commands: &[ServiceTierCommand],
    plugin_commands: &[PluginCommandEntry],
) -> Vec<SlashCommandItem> {
    let mut commands = Vec::new();
    let tiers_enabled = flags.service_tier_commands_enabled;
    for (_, cmd) in builtins_for_input(flags) {
        commands.push(SlashCommandItem::Builtin(cmd));
        if cmd == SlashCommand::Model && tiers_enabled {
            commands.extend(
                service_tier_commands
                    .iter()
                    .cloned()
                    .map(SlashCommandItem::ServiceTier),
            );
        }
    }
    // 插件命令追加在内置命令之后(与 legacy 命令在弹窗中的次序一致)。
    commands.extend(
        plugin_commands
            .iter()
            .cloned()
            .map(SlashCommandItem::Plugin),
    );
    commands
        .into_iter()
        .filter(|cmd| !flags.side_conversation_active || cmd.available_in_side_conversation())
        .collect()
}

/// 应用功能门控后，按可识别的名称或别名查找单个内置命令。
///
/// 中文说明：Side-conversation and token-activity gating are intentionally enforced by dispatch rather than
/// 中文说明：command lookup so a typed command can produce a specific unavailable message while the popup
/// 中文说明：still hides it.
pub(crate) fn find_builtin_command(name: &str, flags: BuiltinCommandFlags) -> Option<SlashCommand> {
    let cmd = SlashCommand::from_str(name).ok().or_else(|| {
        let repeated_os = name.strip_prefix('g')?.strip_suffix("al")?;
        (!repeated_os.is_empty() && repeated_os.bytes().all(|byte| byte == b'o'))
            .then_some(SlashCommand::Goal)
    })?;
    builtins_for_input(BuiltinCommandFlags {
        token_activity_command_enabled: true,
        side_conversation_active: false,
        ..flags
    })
    .into_iter()
    .any(|(_, visible_cmd)| visible_cmd == cmd)
    .then_some(cmd)
}

pub(crate) fn find_slash_command(
    name: &str,
    flags: BuiltinCommandFlags,
    service_tier_commands: &[ServiceTierCommand],
    plugin_commands: &[PluginCommandEntry],
) -> Option<SlashCommandItem> {
    if let Some(cmd) = find_builtin_command(name, flags) {
        return Some(SlashCommandItem::Builtin(cmd));
    }

    let tiers_enabled = flags.service_tier_commands_enabled;
    if tiers_enabled
        && let Some(command) = service_tier_commands
            .iter()
            .find(|command| command.name == name)
            .cloned()
    {
        return Some(SlashCommandItem::ServiceTier(command));
    }

    // 插件命令按全名精确匹配(与 expand_user_input 的查找一致)。
    plugin_commands
        .iter()
        .find(|entry| entry.name == name)
        .cloned()
        .map(SlashCommandItem::Plugin)
}

pub(crate) fn has_slash_command_prefix(
    name: &str,
    flags: BuiltinCommandFlags,
    service_tier_commands: &[ServiceTierCommand],
    plugin_commands: &[PluginCommandEntry],
) -> bool {
    commands_for_input(flags, service_tier_commands, plugin_commands)
        .into_iter()
        .any(|command| fuzzy_match(command.command(), name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::slice::from_ref;

    fn all_enabled_flags() -> BuiltinCommandFlags {
        BuiltinCommandFlags {
            collaboration_modes_enabled: true,
            connectors_enabled: true,
            plugins_command_enabled: true,
            token_activity_command_enabled: true,
            service_tier_commands_enabled: true,
            goal_command_enabled: true,
            personality_command_enabled: true,
            allow_elevate_sandbox: true,
            side_conversation_active: false,
        }
    }

    #[test]
    fn debug_command_still_resolves_for_dispatch() {
        let cmd = find_builtin_command("debug-config", all_enabled_flags());
        assert_eq!(cmd, Some(SlashCommand::DebugConfig));
    }

    #[test]
    fn clear_command_resolves_for_dispatch() {
        assert_eq!(
            find_builtin_command("clear", all_enabled_flags()),
            Some(SlashCommand::Clear)
        );
    }

    #[test]
    fn goal_command_allows_extra_os_for_dispatch() {
        assert_eq!(
            find_builtin_command("goooooooooooal", all_enabled_flags()),
            Some(SlashCommand::Goal)
        );
    }

    #[test]
    fn stop_command_resolves_for_dispatch() {
        assert_eq!(
            find_builtin_command("stop", all_enabled_flags()),
            Some(SlashCommand::Stop)
        );
    }

    #[test]
    fn clean_command_alias_resolves_for_dispatch() {
        assert_eq!(
            find_builtin_command("clean", all_enabled_flags()),
            Some(SlashCommand::Stop)
        );
    }

    #[test]
    fn service_tier_commands_are_hidden_when_disabled() {
        let mut flags = all_enabled_flags();
        flags.service_tier_commands_enabled = false;
        let commands = vec![ServiceTierCommand {
            id: "priority".to_string(),
            name: "fast".to_string(),
            description: "fastest inference".to_string(),
        }];

        assert_eq!(find_slash_command("fast", flags, &commands, &[]), None);
    }

    #[test]
    fn all_service_tiers_are_exposed_as_commands_after_model() {
        let commands = vec![
            ServiceTierCommand {
                id: "priority".to_string(),
                name: "fast".to_string(),
                description: "fastest inference".to_string(),
            },
            ServiceTierCommand {
                id: "batch".to_string(),
                name: "slow".to_string(),
                description: "slower inference with lower priority".to_string(),
            },
        ];

        let items = commands_for_input(all_enabled_flags(), &commands, &[]);
        let model_idx = items
            .iter()
            .position(|item| matches!(item, SlashCommandItem::Builtin(SlashCommand::Model)))
            .expect("model command should be visible");
        let inserted = items
            .into_iter()
            .skip(model_idx + 1)
            .take(commands.len())
            .collect::<Vec<_>>();
        let expected = commands
            .into_iter()
            .map(SlashCommandItem::ServiceTier)
            .collect::<Vec<_>>();

        assert_eq!(inserted, expected);
    }

    #[test]
    fn goal_command_is_hidden_when_disabled() {
        let mut flags = all_enabled_flags();
        flags.goal_command_enabled = false;
        assert_eq!(find_builtin_command("goal", flags), None);
    }

    #[test]
    fn usage_command_is_hidden_from_input_when_account_token_activity_is_disabled() {
        let mut flags = all_enabled_flags();
        flags.token_activity_command_enabled = false;
        assert_eq!(
            builtins_for_input(flags)
                .into_iter()
                .find(|(_, command)| *command == SlashCommand::Usage),
            None
        );
    }

    #[test]
    fn usage_command_exact_lookup_still_resolves_when_account_token_activity_is_disabled() {
        let mut flags = all_enabled_flags();
        flags.token_activity_command_enabled = false;
        assert_eq!(
            find_builtin_command("usage", flags),
            Some(SlashCommand::Usage)
        );
    }

    #[test]
    fn side_conversation_hides_commands_without_side_flag() {
        let commands = builtins_for_input(BuiltinCommandFlags {
            side_conversation_active: true,
            ..all_enabled_flags()
        })
        .into_iter()
        .map(|(_, command)| command)
        .collect::<Vec<_>>();

        assert_eq!(
            commands,
            vec![
                SlashCommand::Ide,
                SlashCommand::Copy,
                SlashCommand::Raw,
                SlashCommand::Diff,
                SlashCommand::Mention,
                SlashCommand::Status,
                SlashCommand::Usage,
            ]
        );
    }

    #[test]
    fn side_conversation_exact_lookup_still_resolves_hidden_commands_for_dispatch_error() {
        assert_eq!(
            find_builtin_command(
                "review",
                BuiltinCommandFlags {
                    side_conversation_active: true,
                    ..all_enabled_flags()
                },
            ),
            Some(SlashCommand::Review)
        );
    }

    #[test]
    fn side_conversation_exact_lookup_still_resolves_service_tier_commands_for_dispatch_error() {
        let command = ServiceTierCommand {
            id: "priority".to_string(),
            name: "fast".to_string(),
            description: "fastest inference".to_string(),
        };
        let flags = BuiltinCommandFlags {
            side_conversation_active: true,
            ..all_enabled_flags()
        };

        assert_eq!(
            find_slash_command("fast", flags, from_ref(&command), &[]),
            Some(SlashCommandItem::ServiceTier(command))
        );
    }
}
