//! Guardian / 自动审查。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuardianApprovalReviewStatus {
    #[default]
    InProgress,
    Approved,
    Denied,
    TimedOut,
    Aborted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AutoReviewDecisionSource {
    #[default]
    Agent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuardianRiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuardianUserAuthorization {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardianApprovalReview {
    pub status: GuardianApprovalReviewStatus,
    pub risk_level: Option<GuardianRiskLevel>,
    pub user_authorization: Option<GuardianUserAuthorization>,
    pub rationale: Option<String>,
}

impl From<GuardianApprovalReviewAction>
    for crate::protocol_compat::approvals::GuardianAssessmentAction
{
    fn from(_value: GuardianApprovalReviewAction) -> Self {
        Self::ApplyPatch {
            cwd: PathBuf::new(),
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardianApprovalReviewAction {
    Command {
        source: GuardianCommandSource,
        command: String,
        cwd: PathBuf,
    },
    Execve {
        source: GuardianCommandSource,
        program: String,
        argv: Vec<String>,
        cwd: PathBuf,
    },
    ApplyPatch {
        cwd: PathBuf,
        files: Vec<PathBuf>,
    },
    NetworkAccess {
        target: String,
        host: String,
        protocol: NetworkApprovalProtocol,
        port: u16,
    },
    McpToolCall {
        server: String,
        tool_name: String,
        connector_id: Option<String>,
        connector_name: Option<String>,
        tool_title: Option<String>,
    },
    RequestPermissions {
        reason: Option<String>,
        permissions: RequestPermissionProfile,
    },
}

impl Default for GuardianApprovalReviewAction {
    fn default() -> Self {
        GuardianApprovalReviewAction::Command {
            source: GuardianCommandSource::default(),
            command: String::new(),
            cwd: PathBuf::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GuardianCommandSource {
    #[default]
    Shell,
    UnifiedExec,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemGuardianApprovalReviewStartedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub started_at_ms: i64,
    pub review_id: String,
    pub target_item_id: Option<String>,
    pub review: GuardianApprovalReview,
    pub action: GuardianApprovalReviewAction,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemGuardianApprovalReviewCompletedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub review_id: String,
    pub target_item_id: Option<String>,
    pub decision_source: AutoReviewDecisionSource,
    pub review: GuardianApprovalReview,
    pub action: GuardianApprovalReviewAction,
}
