//! 顶层协议枚举。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

/// 服务器发送给客户端的通知。
///
/// 存根内联了 UI 的 `match` 分支所引用的所有变体；UI 仅需用 `_` 确认
/// 的变体则使用占位单元负载。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerNotification {
    Error(ErrorNotification),
    ThreadStarted(ThreadStartedNotification),
    ThreadStatusChanged(serde_json::Value),
    ThreadArchived(serde_json::Value),
    ThreadDeleted(serde_json::Value),
    ThreadUnarchived(serde_json::Value),
    ThreadClosed(serde_json::Value),
    SkillsChanged(serde_json::Value),
    ThreadNameUpdated(ThreadNameUpdatedNotification),
    ThreadGoalUpdated(ThreadGoalUpdatedNotification),
    ThreadGoalCleared(ThreadGoalClearedNotification),
    EnvironmentConnected(serde_json::Value),
    EnvironmentDisconnected(serde_json::Value),
    ThreadSettingsUpdated(ThreadSettingsUpdatedNotification),
    ThreadTokenUsageUpdated(ThreadTokenUsageUpdatedNotification),
    TurnStarted(TurnStartedNotification),
    HookStarted(HookStartedNotification),
    TurnCompleted(TurnCompletedNotification),
    HookCompleted(HookCompletedNotification),
    TurnDiffUpdated(TurnDiffUpdatedNotification),
    TurnPlanUpdated(TurnPlanUpdatedNotification),
    ItemStarted(ItemStartedNotification),
    ItemGuardianApprovalReviewStarted(ItemGuardianApprovalReviewStartedNotification),
    ItemGuardianApprovalReviewCompleted(ItemGuardianApprovalReviewCompletedNotification),
    ItemCompleted(ItemCompletedNotification),
    RawResponseItemCompleted(serde_json::Value),
    RawResponseCompleted(serde_json::Value),
    AgentMessageDelta(AgentMessageDeltaNotification),
    PlanDelta(PlanDeltaNotification),
    CommandExecOutputDelta(serde_json::Value),
    ProcessOutputDelta(serde_json::Value),
    ProcessExited(serde_json::Value),
    CommandExecutionOutputDelta(CommandExecutionOutputDeltaNotification),
    TerminalInteraction(TerminalInteractionNotification),
    FileChangeOutputDelta(FileChangeOutputDeltaNotification),
    FileChangePatchUpdated(serde_json::Value),
    ServerRequestResolved(ServerRequestResolvedNotification),
    McpToolCallProgress(serde_json::Value),
    McpServerOauthLoginCompleted(serde_json::Value),
    McpServerStatusUpdated(McpServerStatusUpdatedNotification),
    AccountUpdated(AccountUpdatedNotification),
    AccountRateLimitsUpdated(serde_json::Value),
    AppListUpdated(serde_json::Value),
    RemoteControlStatusChanged(serde_json::Value),
    ExternalAgentConfigImportProgress(serde_json::Value),
    ExternalAgentConfigImportCompleted(serde_json::Value),
    FsChanged(serde_json::Value),
    ReasoningSummaryTextDelta(ReasoningSummaryTextDeltaNotification),
    ReasoningSummaryPartAdded(ReasoningSummaryPartAddedNotification),
    ReasoningTextDelta(ReasoningTextDeltaNotification),
    ContextCompacted(serde_json::Value),
    ModelRerouted(serde_json::Value),
    ModelVerification(ModelVerificationNotification),
    TurnModerationMetadata(serde_json::Value),
    ModelSafetyBufferingUpdated(ModelSafetyBufferingUpdatedNotification),
    Warning(WarningNotification),
    GuardianWarning(GuardianWarningNotification),
    DeprecationNotice(DeprecationNoticeNotification),
    ConfigWarning(ConfigWarningNotification),
    FuzzyFileSearchSessionUpdated(serde_json::Value),
    FuzzyFileSearchSessionCompleted(serde_json::Value),
    ThreadRealtimeStarted(serde_json::Value),
    ThreadRealtimeItemAdded(serde_json::Value),
    ThreadRealtimeTranscriptDelta(serde_json::Value),
    ThreadRealtimeTranscriptDone(serde_json::Value),
    ThreadRealtimeOutputAudioDelta(serde_json::Value),
    ThreadRealtimeSdp(serde_json::Value),
    ThreadRealtimeError(serde_json::Value),
    ThreadRealtimeClosed(serde_json::Value),
    WindowsWorldWritableWarning(serde_json::Value),
    WindowsSandboxSetupCompleted(serde_json::Value),
    AccountLoginCompleted(AccountLoginCompletedNotification),
}

impl Default for ServerNotification {
    fn default() -> Self {
        ServerNotification::Warning(WarningNotification::default())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerRequestResolvedNotification {
    pub thread_id: String,
    pub request_id: RequestId,
}

/// 由服务器发起并发送给客户端的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerRequest {
    CommandExecutionRequestApproval {
        request_id: RequestId,
        params: CommandExecutionRequestApprovalParams,
    },
    FileChangeRequestApproval {
        request_id: RequestId,
        params: FileChangeRequestApprovalParams,
    },
    ToolRequestUserInput {
        request_id: RequestId,
        params: ToolRequestUserInputParams,
    },
    McpServerElicitationRequest {
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    },
    PermissionsRequestApproval {
        request_id: RequestId,
        params: PermissionsRequestApprovalParams,
    },
    DynamicToolCall {
        request_id: RequestId,
        params: serde_json::Value,
    },
    ChatgptAuthTokensRefresh {
        request_id: RequestId,
        params: serde_json::Value,
    },
    AttestationGenerate {
        request_id: RequestId,
        params: serde_json::Value,
    },
    CurrentTimeRead {
        request_id: RequestId,
        params: serde_json::Value,
    },
    ApplyPatchApproval {
        request_id: RequestId,
        params: serde_json::Value,
    },
    ExecCommandApproval {
        request_id: RequestId,
        params: serde_json::Value,
    },
}

impl Default for ServerRequest {
    fn default() -> Self {
        ServerRequest::CurrentTimeRead {
            request_id: RequestId::default(),
            params: serde_json::Value::Null,
        }
    }
}

impl ServerRequest {
    /// 存根
    pub fn id(&self) -> &RequestId {
        match self {
            ServerRequest::CommandExecutionRequestApproval { request_id, .. }
            | ServerRequest::FileChangeRequestApproval { request_id, .. }
            | ServerRequest::ToolRequestUserInput { request_id, .. }
            | ServerRequest::McpServerElicitationRequest { request_id, .. }
            | ServerRequest::PermissionsRequestApproval { request_id, .. }
            | ServerRequest::DynamicToolCall { request_id, .. }
            | ServerRequest::ChatgptAuthTokensRefresh { request_id, .. }
            | ServerRequest::AttestationGenerate { request_id, .. }
            | ServerRequest::CurrentTimeRead { request_id, .. }
            | ServerRequest::ApplyPatchApproval { request_id, .. }
            | ServerRequest::ExecCommandApproval { request_id, .. } => request_id,
        }
    }
}

/// 由客户端发起并发送给服务器的请求。
///
/// 仅 TUI 实际发出的变体非空；其余上游变体在存根中被合并为 `Other`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientRequest {
    OneOffCommandExec {
        request_id: RequestId,
        params: CommandExecParams,
    },
    HooksList {
        request_id: RequestId,
        params: HooksListParams,
    },
    ConfigBatchWrite {
        request_id: RequestId,
        params: ConfigBatchWriteParams,
    },
    LoginAccount {
        request_id: RequestId,
        params: LoginAccountParams,
    },
    CancelLoginAccount {
        request_id: RequestId,
        params: CancelLoginAccountParams,
    },
    Other {
        request_id: RequestId,
        method: String,
        params: serde_json::Value,
    },
}

impl Default for ClientRequest {
    fn default() -> Self {
        ClientRequest::Other {
            request_id: RequestId::default(),
            method: String::new(),
            params: serde_json::Value::Null,
        }
    }
}

impl ClientRequest {
    /// 存根
    pub fn id(&self) -> &RequestId {
        match self {
            ClientRequest::OneOffCommandExec { request_id, .. }
            | ClientRequest::HooksList { request_id, .. }
            | ClientRequest::ConfigBatchWrite { request_id, .. }
            | ClientRequest::LoginAccount { request_id, .. }
            | ClientRequest::CancelLoginAccount { request_id, .. }
            | ClientRequest::Other { request_id, .. } => request_id,
        }
    }
}

impl ExecPolicyAmendment {
    pub fn into_core(&self) -> crate::protocol_compat::approvals::ExecPolicyAmendment {
        crate::protocol_compat::approvals::ExecPolicyAmendment {
            command: self.command.clone(),
        }
    }
}

impl NetworkPolicyAmendment {
    pub fn into_core(&self) -> crate::protocol_compat::approvals::NetworkPolicyAmendment {
        crate::protocol_compat::approvals::NetworkPolicyAmendment {
            host: self.host.clone(),
            action: match self.action {
                NetworkPolicyRuleAction::Allow => {
                    crate::protocol_compat::approvals::NetworkPolicyRuleAction::Allow
                }
                NetworkPolicyRuleAction::Deny => {
                    crate::protocol_compat::approvals::NetworkPolicyRuleAction::Deny
                }
            },
        }
    }
}

impl AskForApproval {
    pub fn to_core(&self) -> AskForApproval {
        *self
    }
}

impl From<&AskForApproval> for AskForApproval {
    fn from(value: &AskForApproval) -> Self {
        *value
    }
}

impl std::fmt::Display for AskForApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AskForApproval::UnlessTrusted => "unless-trusted",
            AskForApproval::OnRequest => "on-request",
            AskForApproval::Never => "never",
        };
        f.write_str(s)
    }
}

impl From<&str> for AskForApproval {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "on-request" | "on_request" => AskForApproval::OnRequest,
            "never" => AskForApproval::Never,
            _ => AskForApproval::UnlessTrusted,
        }
    }
}

impl From<String> for AskForApproval {
    fn from(s: String) -> Self {
        AskForApproval::from(s.as_str())
    }
}

impl From<&String> for AskForApproval {
    fn from(s: &String) -> Self {
        AskForApproval::from(s.as_str())
    }
}

impl std::str::FromStr for AskForApproval {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(AskForApproval::from(s))
    }
}

impl SandboxPolicy {
    pub fn to_core(&self) -> () {}
}
