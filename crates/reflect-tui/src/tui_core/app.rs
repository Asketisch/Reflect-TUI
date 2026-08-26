//! app 模块(主应用控制器)的桩。
//!
//! Reflect 拥有自己的适配层;此桩存在是为了让那些引用 `crate::tui_core::app::App`
//! 的 vendored UI 代码能够编译。上游 `app/` 模块包含的会话/回合/请求管理,
//! Reflect 的处理方式不同。

pub mod app_server_requests {
    /// 已解析的 app server 请求(vendored 代码中按变体匹配的枚举)。
    #[derive(Debug, Clone)]
    pub enum ResolvedAppServerRequest {
        ExecApproval {
            id: String,
        },
        FileChangeApproval {
            id: String,
        },
        PermissionsApproval {
            id: String,
        },
        McpElicitation {
            request_id: crate::app_server_protocol::RequestId,
            server_name: String,
        },
        UserInput {
            call_id: String,
        },
        ExecRequest {
            id: String,
        },
        McpToolCallApproval {
            id: String,
        },
        NetworkAccessApproval {
            id: String,
        },
        ApplyPatchApproval {
            id: String,
        },
    }

    impl Default for ResolvedAppServerRequest {
        fn default() -> Self {
            Self::ExecApproval { id: String::new() }
        }
    }
}

use std::sync::Arc;

use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::pager_overlay::Overlay;

/// `Option<AppEventSender>` 的封装,转发 `.send()` 与 `.clone()`,
/// 这样 vendored 代码可以调用 `self.app.app_event_tx.send(...)` 而无需 unwrap。
#[derive(Debug, Clone, Default)]
pub struct AppEventTx(pub Option<AppEventSender>);

impl AppEventTx {
    pub fn send(&self, event: crate::tui_core::app_event::AppEvent) {
        if let Some(tx) = &self.0 {
            tx.send(event);
        }
    }
}

impl std::ops::Deref for AppEventTx {
    type Target = Option<AppEventSender>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// 应用控制器桩。
///
/// 此结构体有大量 `pub` 字段,以便访问 `app.field_name` 的 vendored UI 代码能够编译。
/// Reflect 适配器提供真正的实现,大多数字段默认值为空/None。
#[derive(Default)]
pub struct App {
    pub bottom_pane: BottomPaneState,
    pub config: crate::tui_core::legacy_core::config::Config,
    pub approval: ApprovalState,
    pub features: FeaturesState,
    pub animations: AnimationsState,
    pub backtrack: BacktrackState,
    pub chat_widget: ChatWidgetState,
    pub transcript_cells: Vec<Arc<dyn HistoryCell>>,
    pub overlay: Option<Overlay>,
    pub session: SessionState,
    pub cwd: std::path::PathBuf,
    pub thread_id: String,
    pub app_event_tx: AppEventTx,
    pub keymap: crate::tui_core::keymap::RuntimeKeymap,
    pub deferred_history_lines: Vec<String>,
    pub terminal_title: String,
    pub task_running: bool,
    pub animations_enabled: bool,
    pub rate_limits: Vec<String>,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }
}

/// 底部面板状态桩。
#[derive(Debug, Default)]
pub struct BottomPaneState {
    pub visible: bool,
    pub height: u16,
}

/// 审批状态桩。
#[derive(Debug, Default)]
pub struct ApprovalState {
    pub pending: bool,
}

/// 功能特性状态桩。
#[derive(Debug, Default)]
pub struct FeaturesState {
    pub features: crate::tui_core::legacy_core::config::FeaturesConfig,
}

/// 动画状态桩。
#[derive(Debug, Default)]
pub struct AnimationsState {
    pub enabled: bool,
}

/// 回溯状态桩。
#[derive(Debug, Default)]
pub struct BacktrackState {
    pub enabled: bool,
    pub base_id: Option<String>,
    pub nth_user_message: usize,
    pub overlay_preview_active: bool,
    pub primed: bool,
}

/// 聊天组件状态桩。
#[derive(Debug, Default)]
pub struct ChatWidgetState {
    pub active: bool,
}

impl ChatWidgetState {
    pub fn active_cell_transcript_key(
        &self,
    ) -> Option<crate::tui_core::chatwidget::ActiveCellTranscriptKey> {
        None
    }
    pub fn active_cell_transcript_hyperlink_lines(
        &self,
        _width: u16,
    ) -> Option<Vec<crate::tui_core::terminal_hyperlinks::HyperlinkLine>> {
        None
    }
    pub fn add_error_message(&mut self, _msg: String) {}
    pub fn add_info_message(&mut self, _msg: String, _hint: Option<String>) {}
    pub fn clear_esc_backtrack_hint(&mut self) {}
    pub fn composer_is_empty(&self) -> bool {
        true
    }
    pub fn restore_user_message_to_composer(&mut self, _msg: impl std::fmt::Display) {}
    pub fn show_esc_backtrack_hint(&mut self) {}
    pub fn side_conversation_active(&self) -> bool {
        false
    }
    pub fn thread_id(&self) -> Option<crate::protocol_compat::ThreadId> {
        None
    }
}

/// 会话状态桩。
#[derive(Debug, Default)]
pub struct SessionState {
    pub id: String,
}

impl App {
    pub fn history_line_wrap_policy(&self) -> String {
        String::new()
    }
    pub fn alt_screen_active(&self) -> bool {
        true
    }
    pub fn streaming_event_tx(&self) -> Option<String> {
        None
    }
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }
    pub fn close_undo_transcript(&mut self) {}
    pub fn prefix_transcript_cells_with(&mut self, _prefix: &str) {}
    pub fn append_transcript_cells(&mut self, _cells: Vec<Arc<dyn HistoryCell>>) {}
    pub fn clear_transcript(&mut self) {}
    pub fn last_session_info(&self) -> String {
        String::new()
    }
    pub fn status_line_formatter(&self) -> String {
        String::new()
    }
}
