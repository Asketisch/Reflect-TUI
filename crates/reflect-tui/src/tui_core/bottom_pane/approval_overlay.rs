//! 高风险操作的审批弹窗渲染与决策路由。
//!
//! 本模块将代理的审批请求（exec/apply-patch/MCP elicitation）转换为带
//! 操作专属选项和快捷键的列表选择视图。它承担两项重要约定：
//!
//! 1. 选择后总是向应用发出一个明确的决策事件。
//! 2. MCP elicitation 即使使用了自定义按键绑定，也始终将 `Esc` 映射为 `Cancel`，
//!    从而避免关闭时静默变成“无信息继续”。
//!
//! 本模块不判断某个操作是否可以安全执行；它只展示选项并路由用户的决策。

use std::collections::HashMap;
use std::path::PathBuf;

use crate::app_server_protocol::AdditionalPermissionProfile;
use crate::app_server_protocol::CommandExecutionApprovalDecision;
use crate::app_server_protocol::FileChangeApprovalDecision;
use crate::app_server_protocol::FileSystemAccessMode;
use crate::app_server_protocol::FileSystemPath;
use crate::app_server_protocol::FileSystemSandboxEntry;
use crate::app_server_protocol::FileSystemSpecialPath;
use crate::app_server_protocol::McpServerElicitationAction;
use crate::app_server_protocol::NetworkApprovalContext;
use crate::app_server_protocol::NetworkApprovalProtocol;
use crate::app_server_protocol::NetworkPolicyRuleAction;
use crate::app_server_protocol::RequestId;
use crate::features::Features;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::request_permissions::PermissionGrantScope;
use crate::protocol_compat::request_permissions::RequestPermissionProfile;
use crate::tui_core::app::app_server_requests::ResolvedAppServerRequest;
#[cfg(test)]
use crate::tui_core::app_command::AppCommand as Op;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::bottom_pane::BottomPaneView;
use crate::tui_core::bottom_pane::CancellationEvent;
use crate::tui_core::bottom_pane::list_selection_view::ListSelectionView;
use crate::tui_core::bottom_pane::list_selection_view::SelectionItem;
use crate::tui_core::bottom_pane::list_selection_view::SelectionViewParams;
use crate::tui_core::bottom_pane::popup_consts::accept_cancel_hint_line;
use crate::tui_core::diff_model::FileChange;
use crate::tui_core::exec_command::strip_bash_lc_and_escape;
use crate::tui_core::history_cell;
use crate::tui_core::history_cell::ReviewDecision;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::keymap::ApprovalKeymap;
use crate::tui_core::keymap::ListKeymap;
use crate::tui_core::keymap::primary_binding;
use crate::tui_core::render::highlight::highlight_bash_to_lines;
use crate::tui_core::render::renderable::ColumnRenderable;
use crate::tui_core::render::renderable::Renderable;
use crate::utils_absolute_path::AbsolutePathBuf;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;

/// 来自代理、需要用户审批的请求。
#[derive(Clone, Debug)]
pub(crate) enum ApprovalRequest {
    Exec(ExecApprovalRequest),
    Permissions(PermissionsApprovalRequest),
    ApplyPatch(ApplyPatchApprovalRequest),
    McpElicitation(McpElicitationApprovalRequest),
}

#[derive(Clone, Debug)]
pub(crate) struct ExecApprovalRequest {
    pub thread_id: ThreadId,
    pub thread_label: Option<String>,
    pub id: String,
    pub environment_id: Option<String>,
    pub command: Vec<String>,
    pub reason: Option<String>,
    pub available_decisions: Vec<CommandExecutionApprovalDecision>,
    pub network_approval_context: Option<NetworkApprovalContext>,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
}

#[derive(Clone, Debug)]
pub(crate) struct PermissionsApprovalRequest {
    pub thread_id: ThreadId,
    pub thread_label: Option<String>,
    pub call_id: String,
    pub environment_id: Option<String>,
    pub reason: Option<String>,
    pub permissions: RequestPermissionProfile,
}

#[derive(Clone, Debug)]
pub(crate) struct ApplyPatchApprovalRequest {
    pub thread_id: ThreadId,
    pub thread_label: Option<String>,
    pub id: String,
    pub reason: Option<String>,
    pub cwd: AbsolutePathBuf,
    pub changes: HashMap<PathBuf, FileChange>,
}

#[derive(Clone, Debug)]
pub(crate) struct McpElicitationApprovalRequest {
    pub thread_id: ThreadId,
    pub thread_label: Option<String>,
    pub server_name: String,
    pub request_id: RequestId,
    pub message: String,
}

impl ApprovalRequest {
    fn thread_id(&self) -> ThreadId {
        match self {
            ApprovalRequest::Exec(request) => request.thread_id,
            ApprovalRequest::Permissions(request) => request.thread_id,
            ApprovalRequest::ApplyPatch(request) => request.thread_id,
            ApprovalRequest::McpElicitation(request) => request.thread_id,
        }
    }

    fn thread_label(&self) -> Option<&str> {
        match self {
            ApprovalRequest::Exec(request) => request.thread_label.as_deref(),
            ApprovalRequest::Permissions(request) => request.thread_label.as_deref(),
            ApprovalRequest::ApplyPatch(request) => request.thread_label.as_deref(),
            ApprovalRequest::McpElicitation(request) => request.thread_label.as_deref(),
        }
    }

    pub(super) fn matches_resolved_request(&self, request: &ResolvedAppServerRequest) -> bool {
        match (self, request) {
            (
                ApprovalRequest::Exec(request),
                ResolvedAppServerRequest::ExecApproval { id: resolved_id },
            ) => request.id == *resolved_id,
            (
                ApprovalRequest::Permissions(request),
                ResolvedAppServerRequest::PermissionsApproval { id },
            ) => request.call_id == *id,
            (
                ApprovalRequest::ApplyPatch(request),
                ResolvedAppServerRequest::FileChangeApproval { id: resolved_id },
            ) => request.id == *resolved_id,
            (
                ApprovalRequest::McpElicitation(request),
                ResolvedAppServerRequest::McpElicitation {
                    server_name: resolved_server_name,
                    request_id: resolved_request_id,
                },
            ) => {
                request.server_name == *resolved_server_name
                    && request.request_id == *resolved_request_id
            }
            _ => false,
        }
    }
}

/// 询问用户批准或拒绝一个或多个请求的模态浮层。
pub(crate) struct ApprovalOverlay {
    current_request: Option<ApprovalRequest>,
    queue: Vec<ApprovalRequest>,
    app_event_tx: AppEventSender,
    list: ListSelectionView,
    options: Vec<ApprovalOption>,
    current_complete: bool,
    done: bool,
    features: Features,
    approval_keymap: ApprovalKeymap,
    list_keymap: ListKeymap,
}

impl ApprovalOverlay {
    pub fn new(
        request: ApprovalRequest,
        app_event_tx: AppEventSender,
        features: Features,
        approval_keymap: ApprovalKeymap,
        list_keymap: ListKeymap,
    ) -> Self {
        let mut view = Self {
            current_request: None,
            queue: Vec::new(),
            app_event_tx: app_event_tx.clone(),
            list: ListSelectionView::new(Default::default(), app_event_tx, list_keymap.clone()),
            options: Vec::new(),
            current_complete: false,
            done: false,
            features,
            approval_keymap,
            list_keymap,
        };
        view.set_current(request);
        view
    }

    pub fn enqueue_request(&mut self, req: ApprovalRequest) {
        self.queue.push(req);
    }

    fn dismiss_resolved_request(&mut self, request: &ResolvedAppServerRequest) -> bool {
        let queue_len = self.queue.len();
        self.queue
            .retain(|queued_request| !queued_request.matches_resolved_request(request));
        if self
            .current_request
            .as_ref()
            .is_some_and(|current_request| current_request.matches_resolved_request(request))
        {
            self.current_complete = true;
            self.advance_queue();
            return true;
        }

        self.queue.len() != queue_len
    }

    fn set_current(&mut self, request: ApprovalRequest) {
        self.current_complete = false;
        let header = build_header(&request);
        let (options, params) = Self::build_options(
            &request,
            header,
            &self.features,
            &self.approval_keymap,
            &self.list_keymap,
        );
        self.current_request = Some(request);
        self.options = options;
        self.list =
            ListSelectionView::new(params, self.app_event_tx.clone(), self.list_keymap.clone());
    }

    fn build_options(
        request: &ApprovalRequest,
        header: Box<dyn Renderable>,
        _features: &Features,
        approval_keymap: &ApprovalKeymap,
        list_keymap: &ListKeymap,
    ) -> (Vec<ApprovalOption>, SelectionViewParams) {
        let (options, title) = match request {
            ApprovalRequest::Exec(request) => (
                exec_options(
                    &request.available_decisions,
                    request.network_approval_context.as_ref(),
                    request.additional_permissions.as_ref(),
                    approval_keymap,
                ),
                request.network_approval_context.as_ref().map_or_else(
                    || "Would you like to run the following command?".to_string(),
                    |network_approval_context| {
                        format!(
                            "Do you want to approve network access to \"{}\"?",
                            network_approval_context.host
                        )
                    },
                ),
            ),
            ApprovalRequest::Permissions(_) => (
                permissions_options(approval_keymap),
                "Would you like to grant these permissions?".to_string(),
            ),
            ApprovalRequest::ApplyPatch(_) => (
                patch_options(approval_keymap),
                "Would you like to make the following edits?".to_string(),
            ),
            ApprovalRequest::McpElicitation(request) => (
                elicitation_options(approval_keymap),
                format!("{} needs your approval.", request.server_name),
            ),
        };

        let header = Box::new(ColumnRenderable::with([
            Line::from(title.bold()).into(),
            Line::from("").into(),
            header,
        ]));

        let items = options
            .iter()
            .map(|opt| SelectionItem {
                name: opt.label.clone(),
                display_shortcut: opt.shortcuts.first().copied(),
                dismiss_on_select: false,
                ..Default::default()
            })
            .collect();

        let params = SelectionViewParams {
            footer_hint: Some(approval_footer_hint(request, approval_keymap, list_keymap)),
            items,
            header,
            ..Default::default()
        };

        (options, params)
    }

    fn apply_selection(&mut self, actual_idx: usize) {
        if self.current_complete {
            return;
        }
        let Some(option) = self.options.get(actual_idx) else {
            return;
        };
        if let Some(request) = self.current_request.as_ref() {
            match (request, &option.decision) {
                (ApprovalRequest::Exec(request), ApprovalDecision::Command(decision)) => {
                    self.handle_exec_decision(&request.id, &request.command, decision.clone());
                }
                (
                    ApprovalRequest::Permissions(request),
                    ApprovalDecision::Permissions(decision),
                ) => {
                    self.handle_permissions_decision(
                        &request.call_id,
                        &request.permissions,
                        *decision,
                    );
                }
                (ApprovalRequest::ApplyPatch(request), ApprovalDecision::FileChange(decision)) => {
                    self.handle_patch_decision(&request.id, decision.clone());
                }
                (
                    ApprovalRequest::McpElicitation(request),
                    ApprovalDecision::McpElicitation(decision),
                ) => {
                    self.handle_elicitation_decision(
                        &request.server_name,
                        &request.request_id,
                        *decision,
                    );
                }
                _ => {}
            }
        }

        self.current_complete = true;
        self.advance_queue();
    }

    fn handle_exec_decision(
        &self,
        id: &str,
        command: &[String],
        decision: CommandExecutionApprovalDecision,
    ) {
        let Some(request) = self.current_request.as_ref() else {
            return;
        };
        if request.thread_label().is_none() {
            let network_approval_context = match request {
                ApprovalRequest::Exec(request) => request.network_approval_context.as_ref(),
                ApprovalRequest::Permissions(_)
                | ApprovalRequest::ApplyPatch(_)
                | ApprovalRequest::McpElicitation(_) => None,
            };
            let subject = if let Some(network_approval_context) = network_approval_context {
                history_cell::ApprovalDecisionSubject::NetworkAccess {
                    target: network_approval_target(network_approval_context, command),
                }
            } else if let Some(target) = network_approval_command_target(command) {
                history_cell::ApprovalDecisionSubject::NetworkAccess {
                    target: target.to_string(),
                }
            } else {
                history_cell::ApprovalDecisionSubject::Command(command.to_vec())
            };
            let cell = history_cell::new_approval_decision_cell(
                subject,
                command_decision_to_review_decision(&decision),
                history_cell::ApprovalDecisionActor::User,
            );
            self.app_event_tx.send(AppEvent::InsertHistoryCell(cell));
        }
        let thread_id = request.thread_id();
        self.app_event_tx
            .exec_approval(thread_id, id.to_string(), decision);
    }

    fn handle_permissions_decision(
        &self,
        call_id: &str,
        permissions: &RequestPermissionProfile,
        decision: PermissionsDecision,
    ) {
        let Some(request) = self.current_request.as_ref() else {
            return;
        };
        let granted_permissions = match decision {
            PermissionsDecision::GrantForTurn
            | PermissionsDecision::GrantForTurnWithStrictAutoReview
            | PermissionsDecision::GrantForSession => permissions.clone(),
            PermissionsDecision::Deny => Default::default(),
        };
        let scope = if matches!(decision, PermissionsDecision::GrantForSession) {
            PermissionGrantScope::Session
        } else {
            PermissionGrantScope::Turn
        };
        let strict_auto_review = matches!(
            decision,
            PermissionsDecision::GrantForTurnWithStrictAutoReview
        );
        if request.thread_label().is_none() {
            let message = if granted_permissions.is_empty() {
                "You did not grant additional permissions"
            } else if strict_auto_review {
                "You granted additional permissions with strict auto review"
            } else if matches!(scope, PermissionGrantScope::Session) {
                "You granted additional permissions for this session"
            } else {
                "You granted additional permissions"
            };
            self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                crate::tui_core::history_cell::PlainHistoryCell::new(vec![message.into()]),
            )));
        }
        let thread_id = request.thread_id();
        self.app_event_tx.request_permissions_response(
            thread_id,
            call_id.to_string(),
            crate::protocol_compat::request_permissions::RequestPermissionsResponse {
                permissions: granted_permissions,
                scope,
                strict_auto_review,
            },
        );
    }

    fn handle_patch_decision(&self, id: &str, decision: FileChangeApprovalDecision) {
        let Some(thread_id) = self
            .current_request
            .as_ref()
            .map(ApprovalRequest::thread_id)
        else {
            return;
        };
        self.app_event_tx
            .patch_approval(thread_id, id.to_string(), decision);
    }

    fn handle_elicitation_decision(
        &self,
        server_name: &str,
        request_id: &RequestId,
        decision: McpServerElicitationAction,
    ) {
        let Some(thread_id) = self
            .current_request
            .as_ref()
            .map(ApprovalRequest::thread_id)
        else {
            return;
        };
        self.app_event_tx.resolve_elicitation(
            thread_id,
            server_name.to_string(),
            request_id.clone(),
            decision,
            /*content*/ None,
            /*meta*/ None,
        );
    }

    fn advance_queue(&mut self) {
        if let Some(next) = self.queue.pop() {
            self.set_current(next);
        } else {
            self.done = true;
        }
    }

    fn cancel_current_request(&mut self) {
        if self.done {
            return;
        }
        if !self.current_complete
            && let Some(request) = self.current_request.as_ref()
        {
            match request {
                ApprovalRequest::Exec(request) => {
                    self.handle_exec_decision(
                        &request.id,
                        &request.command,
                        CommandExecutionApprovalDecision::Cancel,
                    );
                }
                ApprovalRequest::Permissions(request) => {
                    self.handle_permissions_decision(
                        &request.call_id,
                        &request.permissions,
                        PermissionsDecision::Deny,
                    );
                }
                ApprovalRequest::ApplyPatch(request) => {
                    self.handle_patch_decision(&request.id, FileChangeApprovalDecision::Cancel);
                }
                ApprovalRequest::McpElicitation(request) => {
                    self.handle_elicitation_decision(
                        &request.server_name,
                        &request.request_id,
                        McpServerElicitationAction::Cancel,
                    );
                }
            }
        }
        self.queue.clear();
        self.done = true;
    }

    /// 在委托给列表导航之前，先处理审批专属的快捷键。
    ///
    /// `open_fullscreen` 在这里处理，因为它与列表项选择相互独立，
    /// 无论当前高亮的是哪一行都应生效。
    fn try_handle_shortcut(&mut self, key_event: &KeyEvent) -> bool {
        if key_event.kind == KeyEventKind::Press
            && self.approval_keymap.open_fullscreen.is_pressed(*key_event)
            && let Some(request) = self.current_request.as_ref()
        {
            self.app_event_tx
                .send(AppEvent::FullScreenApprovalRequest(request.clone()));
            return true;
        }

        if key_event.kind == KeyEventKind::Press
            && self.approval_keymap.open_thread.is_pressed(*key_event)
            && let Some(request) = self.current_request.as_ref()
            && request.thread_label().is_some()
        {
            self.app_event_tx
                .send(AppEvent::SelectAgentThread(request.thread_id()));
            return true;
        }

        if self.list_keymap.cancel.is_pressed(*key_event) {
            self.cancel_current_request();
            return true;
        }

        if let Some(idx) = self
            .options
            .iter()
            .position(|opt| opt.shortcuts.iter().any(|s| s.is_press(*key_event)))
        {
            self.apply_selection(idx);
            true
        } else {
            false
        }
    }
}

impl BottomPaneView for ApprovalOverlay {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if self.try_handle_shortcut(&key_event) {
            return;
        }
        self.list.handle_key_event(key_event);
        if let Some(idx) = self.list.take_last_selected_index() {
            self.apply_selection(idx);
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.cancel_current_request();
        CancellationEvent::Handled
    }

    fn is_complete(&self) -> bool {
        self.done
    }

    fn try_consume_approval_request(
        &mut self,
        request: ApprovalRequest,
    ) -> Option<ApprovalRequest> {
        self.enqueue_request(request);
        None
    }

    fn dismiss_app_server_request(&mut self, request: &ResolvedAppServerRequest) -> bool {
        self.dismiss_resolved_request(request)
    }

    fn terminal_title_requires_action(&self) -> bool {
        true
    }
}

impl Renderable for ApprovalOverlay {
    fn desired_height(&self, width: u16) -> u16 {
        self.list.desired_height(width)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.list.render(area, buf);
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.list.cursor_pos(area)
    }
}

// ── options/header 构建辅助（外移子模块） ──
mod options;
use options::*;

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
