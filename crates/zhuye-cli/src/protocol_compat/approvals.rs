use super::*;

/// 对执行策略的修订，用于将某个命令前缀加入白名单。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExecPolicyAmendment {
    pub command: Vec<String>,
}

impl ExecPolicyAmendment {
    pub fn new(command: Vec<String>) -> Self {
        Self { command }
    }

    pub fn command(&self) -> &[String] {
        &self.command
    }
}

impl From<Vec<String>> for ExecPolicyAmendment {
    fn from(command: Vec<String>) -> Self {
        Self { command }
    }
}

/// 审批请求使用的网络协议族。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkApprovalProtocol {
    #[default]
    Http,
    #[serde(alias = "https_connect", alias = "http-connect")]
    Https,
    Socks5Tcp,
    Socks5Udp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkApprovalContext {
    pub host: String,
    pub protocol: NetworkApprovalProtocol,
}

/// 附加在网络策略修订上的允许 / 拒绝动作。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicyRuleAction {
    #[default]
    Allow,
    Deny,
}

/// 由 guardian 审查方分配的粗略风险等级。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianRiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

/// 会话记录对受审动作的授权直接程度。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianUserAuthorization {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
}

/// guardian 评估的最终允许 / 拒绝结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardianAssessmentOutcome {
    #[default]
    Allow,
    Deny,
}

/// guardian 评估的生命周期状态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardianAssessmentStatus {
    #[default]
    InProgress,
    Approved,
    Denied,
    TimedOut,
    Aborted,
}

/// 最终评估决策的来源。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardianAssessmentDecisionSource {
    #[default]
    Agent,
}

/// 受审命令的来源。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardianCommandSource {
    #[default]
    Shell,
    UnifiedExec,
}

/// 由 guardian 审查的规范动作载荷。
///
/// 该桩类型把上游的每个变体扁平化为普通字符串字段，这样 TUI
/// 无需引入绝对路径相关的辅助工具即可进行模式匹配。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GuardianAssessmentAction {
    Command {
        source: GuardianCommandSource,
        command: String,
        cwd: std::path::PathBuf,
    },
    Execve {
        source: GuardianCommandSource,
        program: String,
        argv: Vec<String>,
        cwd: std::path::PathBuf,
    },
    ApplyPatch {
        cwd: std::path::PathBuf,
        files: Vec<std::path::PathBuf>,
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
        permissions: crate::protocol_compat::request_permissions::RequestPermissionProfile,
    },
}

impl Default for GuardianAssessmentAction {
    fn default() -> Self {
        Self::Command {
            source: GuardianCommandSource::default(),
            command: String::new(),
            cwd: std::path::PathBuf::new(),
        }
    }
}

/// 应用到网络策略表的修订。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicyAmendment {
    pub host: String,
    pub action: NetworkPolicyRuleAction,
}

/// 单个 guardian 审查生命周期的快照。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GuardianAssessmentEvent {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_item_id: Option<String>,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<i64>,
    pub status: GuardianAssessmentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<GuardianRiskLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_authorization: Option<GuardianUserAuthorization>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_source: Option<GuardianAssessmentDecisionSource>,
    pub action: GuardianAssessmentAction,
}
