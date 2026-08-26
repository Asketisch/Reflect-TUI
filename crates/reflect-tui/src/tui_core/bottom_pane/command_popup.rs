use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::WidgetRef;

use super::popup_consts::MAX_POPUP_ROWS;
use super::scroll_state::ScrollState;
use super::selection_popup_common::ColumnWidthConfig;
use super::selection_popup_common::ColumnWidthMode;
use super::selection_popup_common::GenericDisplayRow;
use super::selection_popup_common::measure_rows_height_with_col_width_mode;
use super::selection_popup_common::render_rows_with_col_width_mode;
use super::slash_commands::BuiltinCommandFlags;
use super::slash_commands::ServiceTierCommand;
use super::slash_commands::SlashCommandItem;
use super::slash_commands::commands_for_input;
use crate::tui_core::render::Insets;
use crate::tui_core::render::RectExt;
use crate::tui_core::slash_command::SlashCommand;

// 在默认弹出列表中隐藏别名命令，使每个唯一操作只出现一次。
// `quit` 是 `exit` 的别名，`btw` 是 `side` 的别名，因此这里跳过这些别名。
const ALIAS_COMMANDS: &[SlashCommand] = &[SlashCommand::Quit, SlashCommand::Btw];
const COMMAND_COLUMN_WIDTH: ColumnWidthConfig = ColumnWidthConfig::new(
    ColumnWidthMode::AutoAllRows,
    /*name_column_width*/ None,
);

/// 弹出列表中的可选条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommandItem {
    Builtin(SlashCommand),
    ServiceTier(ServiceTierCommand),
    /// 由旧版 `dispatch_slash` 字符串匹配器处理的斜杠命令
    /// （不在 `SlashCommand` 枚举中）。为可发现性和自动补全面列出；
    /// 通过 `ComposerAction::Submitted` 以纯文本形式派发。
    Legacy { name: &'static str, description: &'static str },
}

pub(crate) struct CommandPopup {
    command_filter: String,
    commands: Vec<CommandItem>,
    state: ScrollState,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CommandPopupFlags {
    pub(crate) collaboration_modes_enabled: bool,
    pub(crate) connectors_enabled: bool,
    pub(crate) plugins_command_enabled: bool,
    pub(crate) token_activity_command_enabled: bool,
    pub(crate) service_tier_commands_enabled: bool,
    pub(crate) goal_command_enabled: bool,
    pub(crate) personality_command_enabled: bool,
    pub(crate) windows_degraded_sandbox_active: bool,
    pub(crate) side_conversation_active: bool,
}

impl From<CommandPopupFlags> for BuiltinCommandFlags {
    fn from(value: CommandPopupFlags) -> Self {
        Self {
            collaboration_modes_enabled: value.collaboration_modes_enabled,
            connectors_enabled: value.connectors_enabled,
            plugins_command_enabled: value.plugins_command_enabled,
            token_activity_command_enabled: value.token_activity_command_enabled,
            service_tier_commands_enabled: value.service_tier_commands_enabled,
            goal_command_enabled: value.goal_command_enabled,
            personality_command_enabled: value.personality_command_enabled,
            allow_elevate_sandbox: value.windows_degraded_sandbox_active,
            side_conversation_active: value.side_conversation_active,
        }
    }
}

impl CommandPopup {
    pub(crate) fn new(
        flags: CommandPopupFlags,
        service_tier_commands: Vec<ServiceTierCommand>,
    ) -> Self {
        // 保持内置命令的可用性与编写器一致。
        let mut commands = commands_for_input(flags.into(), &service_tier_commands)
            .into_iter()
            .filter_map(|command| match command {
                SlashCommandItem::Builtin(cmd) => (!cmd.command().starts_with("debug")
                    && cmd != SlashCommand::Apps)
                    .then_some(CommandItem::Builtin(cmd)),
                SlashCommandItem::ServiceTier(command) => Some(CommandItem::ServiceTier(command)),
            })
            .collect::<Vec<_>>();
        // 追加由 `dispatch_slash` 处理的旧版命令（不在
        // SlashCommand 枚举中）。这些命令可从弹出列表中发现并自动补全，
        // 但以纯文本形式派发。
        for &(name, desc) in crate::history_render::legacy_popup_commands() {
            commands.push(CommandItem::Legacy {
                name,
                description: desc,
            });
        }
        Self {
            command_filter: String::new(),
            commands,
            state: ScrollState::new(),
        }
    }

    /// 根据当前编写器文本更新过滤字符串。传入的文本应以 '/' 开头。
    /// 第一行中 *第一个* '/' 之后的内容将成为用于
    /// 缩小可用命令列表的活跃过滤条件。
    pub(crate) fn on_composer_text_change(&mut self, text: String) {
        let first_line = text.lines().next().unwrap_or("");
        let previous_filter = self.command_filter.clone();

        if let Some(stripped) = first_line.strip_prefix('/') {
            // 提取斜杠后的 *第一个* 词元（非空白字符序列），
            // 使 `/clear something` 仍能显示 `/clear` 的帮助。
            let token = stripped.trim_start();
            let cmd_token = token.split_whitespace().next().unwrap_or("");

            // 更新过滤条件并保留原始大小写（命令目前全为小写，
            // 但将来可能改变）。
            self.command_filter = cmd_token.to_string();
        } else {
            // 编写器文本不再以 '/' 开头。重置过滤条件，以便弹出列表
            // 若仍因某些原因显示时，展示 *完整* 的命令列表。
            self.command_filter.clear();
        }

        if self.command_filter != previous_filter {
            self.state.reset();
        }

        // 根据新的过滤列表重置或钳制选中索引。
        let matches_len = self.filtered_items().len();
        self.state.clamp_selection(matches_len);
        self.state
            .ensure_visible(matches_len, MAX_POPUP_ROWS.min(matches_len));
    }

    /// 计算给定宽度下弹出列表的合适高度。
    /// 考虑换行描述，避免长提示文本溢出。
    pub(crate) fn calculate_required_height(&self, width: u16) -> u16 {
        let rows = self.rows_from_matches(self.filtered());

        measure_rows_height_with_col_width_mode(
            &rows,
            &self.state,
            MAX_POPUP_ROWS,
            width,
            COMMAND_COLUMN_WIDTH,
        )
    }

    /// 在内置命令与用户提示上计算精确/前缀匹配，
    /// 并附带可选的高亮索引。内置命令与提示保持原有的展示顺序。
    fn filtered(&self) -> Vec<(CommandItem, Option<Vec<usize>>)> {
        let filter = self.command_filter.trim();
        let mut out: Vec<(CommandItem, Option<Vec<usize>>)> = Vec::new();
        if filter.is_empty() {
            for command in self.commands.iter() {
                if matches!(command, CommandItem::Builtin(cmd) if ALIAS_COMMANDS.contains(cmd)) {
                    continue;
                }
                out.push((command.clone(), None));
            }
            return out;
        }

        let filter_lower = filter.to_lowercase();
        let filter_chars = filter.chars().count();
        let mut exact: Vec<(CommandItem, Option<Vec<usize>>)> = Vec::new();
        let mut prefix: Vec<(CommandItem, Option<Vec<usize>>)> = Vec::new();
        let indices_for = |offset| Some((offset..offset + filter_chars).collect());

        let mut push_match =
            |item: CommandItem, display: &str, name: Option<&str>, name_offset: usize| {
                let display_lower = display.to_lowercase();
                let name_lower = name.map(str::to_lowercase);
                let display_exact = display_lower == filter_lower;
                let name_exact = name_lower.as_deref() == Some(filter_lower.as_str());
                if display_exact || name_exact {
                    let offset = if display_exact { 0 } else { name_offset };
                    exact.push((item, indices_for(offset)));
                    return;
                }
                let display_prefix = display_lower.starts_with(&filter_lower);
                let name_prefix = name_lower
                    .as_ref()
                    .is_some_and(|name| name.starts_with(&filter_lower));
                if display_prefix || name_prefix {
                    let offset = if display_prefix { 0 } else { name_offset };
                    prefix.push((item, indices_for(offset)));
                }
            };

        for command in self.commands.iter() {
            let display = command.command();
            push_match(command.clone(), display, None, 0);
        }

        out.extend(exact);
        out.extend(prefix);
        out
    }

    fn filtered_items(&self) -> Vec<CommandItem> {
        self.filtered().into_iter().map(|(c, _)| c).collect()
    }

    fn rows_from_matches(
        &self,
        matches: Vec<(CommandItem, Option<Vec<usize>>)>,
    ) -> Vec<GenericDisplayRow> {
        matches
            .into_iter()
            .map(|(item, indices)| {
                let name = format!("/{}", item.command());
                let description = item.description().to_string();
                GenericDisplayRow {
                    name,
                    name_prefix_spans: Vec::new(),
                    match_indices: indices.map(|v| v.into_iter().map(|i| i + 1).collect()),
                    display_shortcut: None,
                    description: Some(description),
                    category_tag: None,
                    wrap_indent: None,
                    is_disabled: false,
                    disabled_reason: None,
                }
            })
            .collect()
    }

    /// 将选中光标上移一步。
    pub(crate) fn move_up(&mut self) {
        let len = self.filtered_items().len();
        self.state.move_up_wrap(len);
        self.state.ensure_visible(len, MAX_POPUP_ROWS.min(len));
    }

    /// 将选中光标下移一步。
    pub(crate) fn move_down(&mut self) {
        let matches_len = self.filtered_items().len();
        self.state.move_down_wrap(matches_len);
        self.state
            .ensure_visible(matches_len, MAX_POPUP_ROWS.min(matches_len));
    }

    /// 返回当前选中的命令（若有）。
    pub(crate) fn selected_item(&self) -> Option<CommandItem> {
        let matches = self.filtered_items();
        self.state
            .selected_idx
            .and_then(|idx| matches.get(idx).cloned())
    }
}

impl CommandItem {
    pub(crate) fn command(&self) -> &str {
        match self {
            Self::Builtin(cmd) => cmd.command(),
            Self::ServiceTier(command) => &command.name,
            Self::Legacy { name, .. } => name,
        }
    }

    fn description(&self) -> &str {
        match self {
            Self::Builtin(cmd) => cmd.description(),
            Self::ServiceTier(command) => &command.description,
            Self::Legacy { description, .. } => description,
        }
    }
}

impl WidgetRef for CommandPopup {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let rows = self.rows_from_matches(self.filtered());
        render_rows_with_col_width_mode(
            area.inset(Insets::tlbr(
                /*top*/ 0, /*left*/ 2, /*bottom*/ 0, /*right*/ 0,
            )),
            buf,
            &rows,
            &self.state,
            MAX_POPUP_ROWS,
            "no matches",
            COMMAND_COLUMN_WIDTH,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn filter_includes_init_when_typing_prefix() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        // 模拟编写器行以 '/in' 开头，使弹出列表按前缀
        // 过滤匹配的命令。
        popup.on_composer_text_change("/in".to_string());

        // 通过选中的命令访问过滤后的列表，并确保
        // 匹配项中包含新的 "init" 命令。
        let matches = popup.filtered_items();
        let has_init = matches.iter().any(|item| match item {
            CommandItem::Builtin(cmd) => cmd.command() == "init",
            CommandItem::ServiceTier(_) => false,
            CommandItem::Legacy { .. } => false,
        });
        assert!(
            has_init,
            "expected '/init' to appear among filtered commands"
        );
    }

    #[test]
    fn selecting_init_by_exact_match() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/init".to_string());

        // 当存在精确匹配时，默认选中的命令
        // 应为该命令。
        let selected = popup.selected_item();
       match selected {
           Some(CommandItem::Builtin(cmd)) => assert_eq!(cmd.command(), "init"),
           Some(CommandItem::ServiceTier(command)) => {
               panic!("expected init command, got service tier {command:?}")
           }
           Some(CommandItem::Legacy { .. }) => panic!("expected init command, got legacy"),
           None => panic!("expected a selected command for exact match"),
       }
    }

    #[test]
    fn model_is_first_suggestion_for_mo() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/mo".to_string());
        let matches = popup.filtered_items();
       match matches.first() {
           Some(CommandItem::Builtin(cmd)) => assert_eq!(cmd.command(), "model"),
           Some(CommandItem::ServiceTier(command)) => {
               panic!("expected model command, got service tier {command:?}")
           }
           Some(CommandItem::Legacy { .. }) => panic!("expected model command, got legacy"),
           None => panic!("expected at least one match for '/mo'"),
       }
    }

    #[test]
    fn service_tier_command_uses_catalog_name_and_description() {
        let mut popup = CommandPopup::new(
            CommandPopupFlags {
                service_tier_commands_enabled: true,
                ..CommandPopupFlags::default()
            },
            vec![ServiceTierCommand {
                id: "priority".to_string(),
                name: "fast".to_string(),
                description: "Fastest inference with increased plan usage".to_string(),
            }],
        );
        popup.on_composer_text_change("/fa".to_string());

        match popup.selected_item() {
            Some(CommandItem::ServiceTier(command)) => assert_eq!(
                command,
                ServiceTierCommand {
                    id: "priority".to_string(),
                    name: "fast".to_string(),
                    description: "Fastest inference with increased plan usage".to_string(),
                }
            ),
            other => panic!("expected fast service tier to be selected, got {other:?}"),
        }
        let rows = popup.rows_from_matches(popup.filtered());
        assert_eq!(
            rows.first().and_then(|row| row.description.as_deref()),
            Some("Fastest inference with increased plan usage")
        );
    }

    #[test]
    fn filtered_commands_keep_presentation_order_for_prefix() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/m".to_string());

        let cmds: Vec<String> = popup
            .filtered_items()
            .into_iter()
            .map(|item| match item {
                CommandItem::Builtin(cmd) => cmd.command().to_string(),
                CommandItem::ServiceTier(command) => command.name,
                CommandItem::Legacy { name, .. } => name.to_string(),
            })
            .collect();
       assert_eq!(
           cmds,
           vec![
               "model".to_string(),
               "memories".to_string(),
               "mention".to_string(),
                "mcp".to_string(),
                "mode".to_string(), // 旧版命令附加在内置命令之后
           ]
       );
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn app_command_popup_snapshot() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/app".to_string());

        let width = 72;
        let area = Rect::new(
            /*x*/ 0,
            /*y*/ 0,
            width,
            popup.calculate_required_height(width),
        );
        let mut buf = Buffer::empty(area);
        popup.render_ref(area, &mut buf);

        insta::assert_snapshot!("command_popup_app", format!("{buf:?}"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn default_command_popup_items_snapshot() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/".to_string());

        let commands = popup
            .filtered_items()
            .into_iter()
            .map(|item| {
                let command = item.command();
                let description = item.description();
                format!("/{command} - {description}")
            })
            .collect::<Vec<_>>()
            .join("\n");

        insta::assert_snapshot!("command_popup_default_items", commands);
    }

    #[test]
    fn prefix_filter_limits_matches_for_ac() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/ac".to_string());

        let cmds: Vec<String> = popup
            .filtered_items()
            .into_iter()
            .map(|item| match item {
                CommandItem::Builtin(cmd) => cmd.command().to_string(),
                CommandItem::ServiceTier(command) => command.name,
                CommandItem::Legacy { name, .. } => name.to_string(),
            })
            .collect();
        assert!(
            !cmds.iter().any(|cmd| cmd == "compact"),
            "expected prefix search for '/ac' to exclude 'compact', got {cmds:?}"
        );
    }

    #[test]
    fn changing_filter_resets_selection_after_scrolling() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/".to_string());

        for _ in 0..MAX_POPUP_ROWS {
            popup.move_down();
        }
        assert!(popup.state.scroll_top > 0);

        popup.on_composer_text_change("/st".to_string());

        assert_eq!(
            popup.selected_item(),
            Some(CommandItem::Builtin(SlashCommand::Status))
        );
        assert_eq!(popup.state.scroll_top, 0);
        let width = 72;
        let area = Rect::new(
            /*x*/ 0,
            /*y*/ 0,
            width,
            popup.calculate_required_height(width),
        );
        let mut buf = Buffer::empty(area);
        popup.render_ref(area, &mut buf);
        insta::assert_snapshot!(
            "command_popup_filter_reset_after_scroll",
            format!("{buf:?}")
        );
    }

    #[test]
    fn quit_hidden_in_empty_filter_but_shown_for_prefix() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/".to_string());
        let items = popup.filtered_items();
        assert!(!items.contains(&CommandItem::Builtin(SlashCommand::Quit)));

        popup.on_composer_text_change("/qu".to_string());
        let items = popup.filtered_items();
        assert!(items.contains(&CommandItem::Builtin(SlashCommand::Quit)));
    }

    #[test]
    fn btw_hidden_in_empty_filter_but_shown_for_prefix() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/".to_string());
        let items = popup.filtered_items();
        assert!(!items.contains(&CommandItem::Builtin(SlashCommand::Btw)));

        popup.on_composer_text_change("/bt".to_string());
        let items = popup.filtered_items();
        assert!(items.contains(&CommandItem::Builtin(SlashCommand::Btw)));
    }

    #[test]
    fn plan_command_hidden_when_collaboration_modes_disabled() {
        let mut popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        popup.on_composer_text_change("/".to_string());

        let cmds: Vec<String> = popup
            .filtered_items()
            .into_iter()
            .map(|item| match item {
                CommandItem::Builtin(cmd) => cmd.command().to_string(),
                CommandItem::ServiceTier(command) => command.name,
                CommandItem::Legacy { name, .. } => name.to_string(),
            })
            .collect();
        assert!(
            !cmds.iter().any(|cmd| cmd == "plan"),
            "expected '/plan' to be hidden when collaboration modes are disabled, got {cmds:?}"
        );
    }

    #[test]
    fn plan_command_visible_when_collaboration_modes_enabled() {
        let mut popup = CommandPopup::new(
            CommandPopupFlags {
                collaboration_modes_enabled: true,
                connectors_enabled: false,
                plugins_command_enabled: false,
                token_activity_command_enabled: false,
                service_tier_commands_enabled: false,
                goal_command_enabled: false,
                personality_command_enabled: true,
                windows_degraded_sandbox_active: false,
                side_conversation_active: false,
            },
            Vec::new(),
        );
        popup.on_composer_text_change("/plan".to_string());

        match popup.selected_item() {
            Some(CommandItem::Builtin(cmd)) => assert_eq!(cmd.command(), "plan"),
            Some(CommandItem::ServiceTier(command)) => {
                panic!("expected plan command, got service tier {command:?}")
            }
            other => panic!("expected plan to be selected for exact match, got {other:?}"),
        }
    }

    #[test]
    fn personality_command_hidden_when_disabled() {
        let mut popup = CommandPopup::new(
            CommandPopupFlags {
                collaboration_modes_enabled: true,
                connectors_enabled: false,
                plugins_command_enabled: false,
                token_activity_command_enabled: false,
                service_tier_commands_enabled: false,
                goal_command_enabled: false,
                personality_command_enabled: false,
                windows_degraded_sandbox_active: false,
                side_conversation_active: false,
            },
            Vec::new(),
        );
        popup.on_composer_text_change("/pers".to_string());

        let cmds: Vec<String> = popup
            .filtered_items()
            .into_iter()
            .map(|item| match item {
                CommandItem::Builtin(cmd) => cmd.command().to_string(),
                CommandItem::ServiceTier(command) => command.name,
                CommandItem::Legacy { name, .. } => name.to_string(),
            })
            .collect();
        assert!(
            !cmds.iter().any(|cmd| cmd == "personality"),
            "expected '/personality' to be hidden when disabled, got {cmds:?}"
        );
    }

    #[test]
    fn personality_command_visible_when_enabled() {
        let mut popup = CommandPopup::new(
            CommandPopupFlags {
                collaboration_modes_enabled: true,
                connectors_enabled: false,
                plugins_command_enabled: false,
                token_activity_command_enabled: false,
                service_tier_commands_enabled: false,
                goal_command_enabled: false,
                personality_command_enabled: true,
                windows_degraded_sandbox_active: false,
                side_conversation_active: false,
            },
            Vec::new(),
        );
        popup.on_composer_text_change("/personality".to_string());

        match popup.selected_item() {
            Some(CommandItem::Builtin(cmd)) => assert_eq!(cmd.command(), "personality"),
            Some(CommandItem::ServiceTier(command)) => {
                panic!("expected personality command, got service tier {command:?}")
            }
            other => panic!("expected personality to be selected for exact match, got {other:?}"),
        }
    }

    #[test]
    fn debug_commands_are_hidden_from_popup() {
        let popup = CommandPopup::new(CommandPopupFlags::default(), Vec::new());
        let cmds: Vec<String> = popup
            .filtered_items()
            .into_iter()
            .map(|item| match item {
                CommandItem::Builtin(cmd) => cmd.command().to_string(),
                CommandItem::ServiceTier(command) => command.name,
                CommandItem::Legacy { name, .. } => name.to_string(),
            })
            .collect();

        assert!(
            !cmds.iter().any(|name| name.starts_with("debug")),
            "expected no /debug* command in popup menu, got {cmds:?}"
        );
    }
}
