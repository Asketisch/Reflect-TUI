//! `ChatWidget` 与 `/keymap` 选择器流程的集成点。
//!
//! 选择器模型、按键捕获视图和编辑语义定义在 [`crate::tui_core::keymap_setup`] 中。
//! 本模块仅保留 `ChatWidget` 自己负责的部分：在底部面板中打开这些视图、
//! 在编辑后将用户路由回正确的选择器行，以及将已提交的键位配置同步回活跃的 widget 状态。
//! 把这些方法放在 `chatwidget.rs` 之外，可以避免让主会话/事件处理界面同时承担
//! `/keymap` 的导航细节。
//!
//! 重要的不变量是：任何被接受的键位编辑必须同时更新三处：
//! 已存储的 `Config.tui_keymap`、应用级快捷键使用的已缓存的“复制响应”绑定，
//! 以及底部面板的运行时键位绑定。仅更新其中一处会让界面看起来接受了一次重新映射，
//! 而某些处理器仍然响应旧的按键。

use crate::config_compat::types::TuiKeymap;
use crate::terminal_detection::terminal_info;

use super::ChatWidget;
use super::queued_message_edit_hint_binding;
use crate::tui_core::app_event::KeymapEditIntent;
use crate::tui_core::keymap::RuntimeKeymap;
use crate::tui_core::keymap_setup;

impl ChatWidget {
    /// 使用当前的 `tui.keymap` 配置打开根 `/keymap` 选择器。
    ///
    /// 在构建选择器行之前先校验已持久化的键位，因为后续每个选择器界面都需要
    /// 有效的运行时绑定（包括预设默认值和用户覆盖）。如果配置无效，
    /// 用户会看到解析错误，而不是一个可能基于陈旧运行时状态提交编辑的不完整选择器。
    pub(crate) fn open_keymap_picker(&mut self) {
        match RuntimeKeymap::from_config(&self.config.tui_keymap) {
            Ok(runtime_keymap) => {
                let params = keymap_setup::build_keymap_picker_params_with_filter(
                    &runtime_keymap,
                    &self.config.tui_keymap,
                    self.keymap_action_filter(),
                );
                self.bottom_pane.show_selection_view(params);
            }
            Err(err) => {
                self.add_error_message(format!("Invalid `tui.keymap` configuration: {err}"));
            }
        }
    }

    /// 打开单个键位动作的“每动作”菜单。
    ///
    /// 调用者从选中该动作的应用事件中传入已解析的运行时键位。如果在这里重新计算，
    /// 一旦在选择器事件和此处理器之间又应用了其他键位编辑，就会出现为不同配置显示菜单的风险。
    pub(crate) fn open_keymap_action_menu(
        &mut self,
        context: String,
        action: String,
        runtime_keymap: &RuntimeKeymap,
    ) {
        let params = keymap_setup::build_keymap_action_menu_params(
            context,
            action,
            runtime_keymap,
            &self.config.tui_keymap,
        );
        self.bottom_pane.show_selection_view(params);
    }

    /// 为“设置”、“替换”或“备用绑定”编辑打开按键捕获视图。
    ///
    /// 按键捕获视图负责原始按键解析，但 `ChatWidget` 提供事件发送者，
    /// 这样捕获到的按键可以与菜单选择走同一条应用事件路径。绕过该路径
    /// 会跳过配置持久化，并使运行时键位缓存保持不变。
    pub(crate) fn open_keymap_capture(
        &mut self,
        context: String,
        action: String,
        intent: KeymapEditIntent,
        runtime_keymap: &RuntimeKeymap,
    ) {
        let view = keymap_setup::build_keymap_capture_view(
            context,
            action,
            intent,
            runtime_keymap,
            self.app_event_tx.clone(),
        );
        self.bottom_pane.show_view(Box::new(view));
        self.request_redraw();
    }

    /// 使用当前运行时绑定打开按键检查器。
    pub(crate) fn open_keymap_debug(&mut self, runtime_keymap: &RuntimeKeymap) {
        let view = keymap_setup::build_keymap_debug_view(runtime_keymap, &self.config.tui_keymap);
        self.bottom_pane.show_view(Box::new(view));
        self.request_redraw();
    }

    /// 打开允许用户选择要替换的现有绑定的菜单。
    ///
    /// 该菜单仅用于具有多个有效绑定的动作。所选择的绑定会通过后续的捕获意图传递，
    /// 以避免在替换编辑时意外折叠掉本应保留可用的备用绑定。
    pub(crate) fn open_keymap_replace_binding_menu(
        &mut self,
        context: String,
        action: String,
        runtime_keymap: &RuntimeKeymap,
    ) {
        let params =
            keymap_setup::build_keymap_replace_binding_menu_params(context, action, runtime_keymap);
        self.bottom_pane.show_selection_view(params);
    }

    /// 返回到根选择器，并选中已编辑的动作。
    ///
    /// 首选路径是就地替换任何活跃的键位选择器子菜单，使底部面板的返回栈
    /// 不会在每次编辑后堆积陈旧的菜单。如果预期的视图栈不再活跃，
    /// 则回退到显示一个全新的选择器，而不是把用户留在过时的界面上。
    pub(crate) fn return_to_keymap_picker(
        &mut self,
        context: &str,
        action: &str,
        runtime_keymap: &RuntimeKeymap,
    ) {
        let params = keymap_setup::build_keymap_picker_params_for_selected_action_with_filter(
            runtime_keymap,
            &self.config.tui_keymap,
            self.keymap_action_filter(),
            context,
            action,
        );
        let replaced = self.bottom_pane.replace_active_views_with_selection_view(
            &[
                keymap_setup::KEYMAP_PICKER_VIEW_ID,
                keymap_setup::KEYMAP_ACTION_MENU_VIEW_ID,
                keymap_setup::KEYMAP_REPLACE_BINDING_MENU_VIEW_ID,
            ],
            params,
        );
        if !replaced {
            let params = keymap_setup::build_keymap_picker_params_for_selected_action_with_filter(
                runtime_keymap,
                &self.config.tui_keymap,
                self.keymap_action_filter(),
                context,
                action,
            );
            self.bottom_pane.show_selection_view(params);
        }
        self.request_redraw();
    }

    fn keymap_action_filter(&self) -> keymap_setup::KeymapActionFilter {
        keymap_setup::KeymapActionFilter {
            fast_mode_enabled: self.fast_mode_enabled(),
        }
    }

    /// 将已提交的键位编辑应用到活跃的聊天 widget。
    ///
    /// 调用者负责在调用本方法前持久化配置文件。本方法会作为一个整体
    /// 更新内存中的配置、应用级复制绑定缓存以及底部面板的键位绑定；
    /// 如果调用者只更新 `self.config.tui_keymap`，那么可见的选择器状态
    /// 和活跃的按键处理器在下次重启前会一直不一致。
    pub(crate) fn apply_keymap_update(
        &mut self,
        keymap_config: TuiKeymap,
        runtime_keymap: &RuntimeKeymap,
    ) {
        self.config.tui_keymap = keymap_config;
        self.copy_last_response_binding = runtime_keymap.app.copy.clone();
        self.chat_keymap = runtime_keymap.chat.clone();
        self.queued_message_edit_hint_binding = queued_message_edit_hint_binding(
            &self.chat_keymap.edit_queued_message,
            terminal_info(),
        );
        self.bottom_pane
            .set_queued_message_edit_binding(self.queued_message_edit_hint_binding);
        self.bottom_pane.set_keymap_bindings(runtime_keymap);
        self.request_redraw();
    }
}
