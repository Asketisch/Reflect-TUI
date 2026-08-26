//! Hooks。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookEventName {
    #[default]
    PreToolUse,
    PermissionRequest,
    PostToolUse,
    PreCompact,
    PostCompact,
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    SubagentStart,
    SubagentStop,
    Stop,
}

impl HookEventName {
    /// 按声明顺序遍历所有枚举变体。
    pub fn iter() -> impl Iterator<Item = Self> {
        [
            Self::PreToolUse,
            Self::PermissionRequest,
            Self::PostToolUse,
            Self::PreCompact,
            Self::PostCompact,
            Self::SessionStart,
            Self::SessionEnd,
            Self::UserPromptSubmit,
            Self::SubagentStart,
            Self::SubagentStop,
            Self::Stop,
        ]
        .into_iter()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookHandlerType {
    #[default]
    Command,
    Prompt,
    Agent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookExecutionMode {
    #[default]
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookScope {
    #[default]
    Thread,
    Turn,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookSource {
    System,
    #[default]
    User,
    Project,
    Mdm,
    SessionFlags,
    Plugin,
    CloudRequirements,
    CloudManagedConfig,
    LegacyManagedConfigFile,
    LegacyManagedConfigMdm,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookTrustStatus {
    #[default]
    Managed,
    Untrusted,
    Trusted,
    Modified,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookRunStatus {
    #[default]
    Running,
    Completed,
    Failed,
    Blocked,
    Stopped,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HookOutputEntryKind {
    #[default]
    Warning,
    Stop,
    Feedback,
    Context,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookOutputEntry {
    pub kind: HookOutputEntryKind,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookRunSummary {
    pub id: String,
    pub event_name: HookEventName,
    pub handler_type: HookHandlerType,
    pub execution_mode: HookExecutionMode,
    pub scope: HookScope,
    pub source_path: PathBuf,
    pub source: HookSource,
    pub display_order: i64,
    pub status: HookRunStatus,
    pub status_message: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub entries: Vec<HookOutputEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookStartedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub run: HookRunSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookCompletedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub run: HookRunSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookMetadata {
    pub key: String,
    pub event_name: HookEventName,
    pub handler_type: HookHandlerType,
    pub matcher: Option<String>,
    pub command: Option<String>,
    pub timeout_sec: u64,
    pub status_message: Option<String>,
    pub additional_context_limit: Option<usize>,
    pub source_path: PathBuf,
    pub source: HookSource,
    pub plugin_id: Option<String>,
    pub display_order: i64,
    pub enabled: bool,
    pub is_managed: bool,
    pub current_hash: String,
    pub trust_status: HookTrustStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookErrorInfo {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HooksListParams {
    pub cwds: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HooksListResponse {
    pub data: Vec<HooksListEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HooksListEntry {
    pub cwd: PathBuf,
    pub hooks: Vec<HookMetadata>,
    pub warnings: Vec<String>,
    pub errors: Vec<HookErrorInfo>,
}
