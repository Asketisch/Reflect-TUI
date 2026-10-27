//! 侧边对话模式的聊天 widget 钩子。
//!
//! app 级侧边线程生命周期位于 `app::side`；本模块负责侧边模式切换的
//! 聊天界面部分，如输入框占位符、页脚标签以及内联 `/side` 消息提交行为。

use super::*;

impl ChatWidget {
    pub(crate) fn submit_user_message_as_plain_user_turn(
        &mut self,
        user_message: UserMessage,
    ) -> Option<AppCommand> {
        self.submit_user_message_with_shell_escape_policy(user_message, ShellEscapePolicy::Disallow)
    }

    pub(crate) fn set_side_conversation_active(&mut self, active: bool) {
        self.active_side_conversation = active;
        let placeholder = if active {
            self.side_placeholder_text.clone()
        } else {
            self.normal_placeholder_text.clone()
        };
        self.bottom_pane.set_placeholder_text(placeholder);
        self.bottom_pane.set_side_conversation_active(active);
        if self.blocks_direct_input && !active {
            self.bottom_pane.set_parent_owned_thread();
        }
    }

    pub(crate) fn side_conversation_active(&self) -> bool {
        self.active_side_conversation
    }

    pub(crate) fn set_side_conversation_context_label(&mut self, label: Option<String>) {
        self.bottom_pane.set_side_conversation_context_label(label);
    }
}
