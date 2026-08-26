//! 在 app-server 路由、聊天展示和状态 UI 之间共享的标准 TUI 会话状态。
//!
//! app-server API 是会话生命周期事件的边界。当这些响应进入 TUI 后，
//! 本模块持有应用编排和小部件所使用的较小内部状态结构。

use std::path::PathBuf;

use crate::app_server_protocol::AskForApproval;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::config_types::CollaborationMode;
use crate::protocol_compat::config_types::Personality;
use crate::protocol_compat::models::ActivePermissionProfile;
use crate::protocol_compat::models::PermissionProfile;
use crate::utils_absolute_path::AbsolutePathBuf;
use crate::utils_path_uri::PathUri;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionNetworkProxyRuntime {
    pub(crate) http_addr: String,
    pub(crate) socks_addr: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MessageHistoryMetadata {
    pub(crate) log_id: u64,
    pub(crate) entry_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ThreadSessionState {
    pub(crate) thread_id: ThreadId,
    pub(crate) forked_from_id: Option<ThreadId>,
    pub(crate) fork_parent_title: Option<String>,
    pub(crate) thread_name: Option<String>,
    pub(crate) model: String,
    pub(crate) model_provider_id: String,
    pub(crate) service_tier: Option<String>,
    pub(crate) approval_policy: AskForApproval,
    pub(crate) approvals_reviewer: crate::protocol_compat::config_types::ApprovalsReviewer,
    /// TUI 展示界面所使用的权限快照。旧版 app-server 响应在接入时会使用响应的 cwd
    /// 转换为 profile，以便已缓存的会话不会重新解释与 cwd 绑定的授权。
    /// 除非用户在 TUI 中显式更改了权限，否则轮次请求不得将此快照视为本地权限覆盖。
    pub(crate) permission_profile: PermissionProfile,
    /// 生成 `permission_profile` 的具名或隐式内置 profile（当服务端知晓时）。
    pub(crate) active_permission_profile: Option<ActivePermissionProfile>,
    pub(crate) cwd: AbsolutePathBuf,
    pub(crate) runtime_workspace_roots: Vec<AbsolutePathBuf>,
    pub(crate) instruction_source_paths: Vec<PathUri>,
    pub(crate) reasoning_effort: Option<crate::protocol_compat::openai_models::ReasoningEffort>,
    pub(crate) collaboration_mode: Option<Box<CollaborationMode>>,
    pub(crate) personality: Option<Personality>,
    pub(crate) message_history: Option<MessageHistoryMetadata>,
    pub(crate) network_proxy: Option<SessionNetworkProxyRuntime>,
    pub(crate) rollout_path: Option<PathBuf>,
}

impl ThreadSessionState {
    pub(crate) fn set_cwd_retargeting_implicit_runtime_workspace_root(
        &mut self,
        cwd: AbsolutePathBuf,
    ) {
        let previous_cwd = std::mem::replace(&mut self.cwd, cwd.clone());
        if !self.runtime_workspace_roots.contains(&previous_cwd) {
            return;
        }

        let previous_roots = std::mem::take(&mut self.runtime_workspace_roots);
        self.runtime_workspace_roots.push(cwd);
        for root in previous_roots {
            if root != previous_cwd && !self.runtime_workspace_roots.contains(&root) {
                self.runtime_workspace_roots.push(root);
            }
        }
    }
}
