//! 应用事件和常见出站 TUI 命令的便捷发送器。
//!
//! 这包装了原始通道，使调用点可以提交类型化的 `AppCommand`
//! 而无需重复事件构造或会话日志记录行为。

use std::path::PathBuf;

use crate::app_server_protocol::CommandExecutionApprovalDecision;
use crate::app_server_protocol::FileChangeApprovalDecision;
use crate::app_server_protocol::McpServerElicitationAction;
use crate::app_server_protocol::RequestId as AppServerRequestId;
use crate::app_server_protocol::ReviewTarget;
use crate::app_server_protocol::ToolRequestUserInputResponse;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::request_permissions::RequestPermissionsResponse;
use crate::tui_core::app_command::AppCommand;
use tokio::sync::mpsc::UnboundedSender;

use crate::tui_core::app_event::AppEvent;
use crate::tui_core::session_log;

#[derive(Clone, Debug)]
pub struct AppEventSender {
    pub app_event_tx: UnboundedSender<AppEvent>,
}

impl AppEventSender {
    pub(crate) fn new(app_event_tx: UnboundedSender<AppEvent>) -> Self {
        Self { app_event_tx }
    }

    /// 向应用事件通道发送事件。如果失败，我们吞掉
    /// 错误并记录日志。
    pub(crate) fn send(&self, event: AppEvent) {
        // 为高保真会话回放记录入站事件。
        // 避免双重记录 Op；它们在提交点被记录。
        if !matches!(event, AppEvent::ReflectOp(_)) {
            session_log::log_inbound_app_event(&event);
        }
        if let Err(e) = self.app_event_tx.send(event) {
            tracing::error!("failed to send event: {e}");
        }
    }

    pub(crate) fn interrupt(&self) {
        self.send(AppEvent::ReflectOp(AppCommand::interrupt()));
    }

    pub(crate) fn compact(&self) {
        self.send(AppEvent::ReflectOp(AppCommand::compact()));
    }

    pub(crate) fn set_thread_name(&self, name: String) {
        self.send(AppEvent::ReflectOp(AppCommand::set_thread_name(name)));
    }

    pub(crate) fn review(&self, target: ReviewTarget) {
        self.send(AppEvent::ReflectOp(AppCommand::review(target)));
    }

    pub(crate) fn list_skills(&self, cwds: Vec<PathBuf>, force_reload: bool) {
        self.send(AppEvent::ReflectOp(AppCommand::list_skills(
            cwds,
            force_reload,
        )));
    }

    pub(crate) fn user_input_answer(&self, id: String, response: ToolRequestUserInputResponse) {
        self.send(AppEvent::ReflectOp(AppCommand::user_input_answer(
            id, response,
        )));
    }

    pub(crate) fn exec_approval(
        &self,
        thread_id: ThreadId,
        id: String,
        decision: CommandExecutionApprovalDecision,
    ) {
        self.send(AppEvent::SubmitThreadOp {
            thread_id,
            op: AppCommand::exec_approval(id, /*turn_id*/ None, decision),
        });
    }

    pub(crate) fn request_permissions_response(
        &self,
        thread_id: ThreadId,
        id: String,
        response: RequestPermissionsResponse,
    ) {
        self.send(AppEvent::SubmitThreadOp {
            thread_id,
            op: AppCommand::request_permissions_response(id, response),
        });
    }

    pub(crate) fn patch_approval(
        &self,
        thread_id: ThreadId,
        id: String,
        decision: FileChangeApprovalDecision,
    ) {
        self.send(AppEvent::SubmitThreadOp {
            thread_id,
            op: AppCommand::patch_approval(id, decision),
        });
    }

    pub(crate) fn resolve_elicitation(
        &self,
        thread_id: ThreadId,
        server_name: String,
        request_id: AppServerRequestId,
        decision: McpServerElicitationAction,
        content: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) {
        self.send(AppEvent::SubmitThreadOp {
            thread_id,
            op: AppCommand::resolve_elicitation(server_name, request_id, decision, content, meta),
        });
    }
}
