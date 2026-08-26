//! 会话 / 回合。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Turn {
    pub id: String,
    pub items: Vec<ThreadItem>,
    pub items_view: TurnItemsView,
    pub status: TurnStatus,
    pub error: Option<TurnError>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TurnItemsView {
    NotLoaded,
    Summary,
    #[default]
    Full,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TurnStatus {
    #[default]
    Completed,
    Interrupted,
    Failed,
    InProgress,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnError {
    pub message: String,
    pub reflect_error_info: Option<ReflectErrorInfo>,
    pub additional_details: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ReflectErrorInfo {
    #[default]
    ContextWindowExceeded,
    SessionBudgetExceeded,
    UsageLimitExceeded,
    ServerOverloaded,
    CyberPolicy,
    HttpConnectionFailed {
        http_status_code: Option<u16>,
    },
    ResponseStreamConnectionFailed {
        http_status_code: Option<u16>,
    },
    InternalServerError,
    Unauthorized,
    BadRequest,
    ThreadRollbackFailed,
    SandboxError,
    ResponseStreamDisconnected {
        http_status_code: Option<u16>,
    },
    ResponseTooManyFailedAttempts {
        http_status_code: Option<u16>,
    },
    ActiveTurnNotSteerable {
        turn_kind: NonSteerableTurnKind,
    },
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NonSteerableTurnKind {
    #[default]
    Review,
    Compact,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnCompletedNotification {
    pub thread_id: String,
    pub turn: Turn,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnStartedNotification {
    pub thread_id: String,
    pub turn: Turn,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadStartedNotification {
    pub thread_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnDiffUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub diff: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnPlanUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub explanation: Option<String>,
    pub plan: Vec<TurnPlanStep>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnPlanStep {
    pub step: String,
    pub status: TurnPlanStepStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TurnPlanStepStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadTokenUsage {
    pub total: TokenUsageBreakdown,
    pub last: TokenUsageBreakdown,
    pub model_context_window: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsageBreakdown {
    pub total_tokens: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadTokenUsageUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub token_usage: ThreadTokenUsage,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadNameUpdatedNotification {
    pub thread_id: String,
    pub thread_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadGoalUpdatedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub goal: ThreadGoal,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadGoalClearedNotification {
    pub thread_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadGoal {
    pub thread_id: String,
    pub objective: String,
    pub status: ThreadGoalStatus,
    pub token_budget: Option<i64>,
    pub tokens_used: i64,
    pub time_used_seconds: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThreadGoalStatus {
    #[default]
    Active,
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited,
    Complete,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadSettings {
    pub cwd: PathBuf,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
    pub sandbox_policy: SandboxPolicy,
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub model: String,
    pub model_provider: String,
    pub service_tier: Option<String>,
    pub effort: Option<String>,
    pub summary: Option<String>,
    pub collaboration_mode: Option<crate::protocol_compat::config_types::CollaborationMode>,
    pub multi_agent_mode: Option<String>,
    pub personality: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThreadSettingsUpdatedNotification {
    pub thread_id: String,
    pub thread_settings: ThreadSettings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AskForApproval {
    #[default]
    UnlessTrusted,
    OnRequest,
    Never,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ApprovalsReviewer {
    #[default]
    User,
    AutoReview,
}

impl ApprovalsReviewer {
    pub fn to_core(self) -> crate::config_compat::types::ApprovalsReviewer {
        match self {
            Self::User => crate::config_compat::types::ApprovalsReviewer::User,
            Self::AutoReview => crate::config_compat::types::ApprovalsReviewer::AutoReview,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SandboxPolicy {
    #[default]
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivePermissionProfile {
    pub name: String,
    pub source: Option<String>,
}

impl From<crate::protocol_compat::models::ActivePermissionProfile> for ActivePermissionProfile {
    fn from(_value: crate::protocol_compat::models::ActivePermissionProfile) -> Self {
        Self {
            name: String::new(),
            source: None,
        }
    }
}
