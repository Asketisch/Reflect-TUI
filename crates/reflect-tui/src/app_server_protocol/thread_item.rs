//! 会话条目（thread items）。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserMessage {
    pub id: String,
    pub client_id: Option<String>,
    pub content: Vec<UserInput>,
}

/// 汇总所有情况并镜像上游变体的 `ThreadItem` 枚举。承载大型嵌套负载的
/// 变体保留其字段，以便 UI 的 `match` 分支仍能编译；元组变体被内联为
/// 结构体变体以支持默认值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadItem {
    UserMessage {
        id: String,
        client_id: Option<String>,
        content: Vec<UserInput>,
    },
    HookPrompt {
        id: String,
        fragments: Vec<HookPromptFragment>,
    },
    AgentMessage {
        id: String,
        text: String,
        phase: Option<MessagePhase>,
        memory_citation: Option<MemoryCitation>,
    },
    Plan {
        id: String,
        text: String,
    },
    Reasoning {
        id: String,
        summary: Vec<String>,
        content: Vec<String>,
    },
    CommandExecution {
        id: String,
        command: String,
        cwd: String,
        process_id: Option<String>,
        source: CommandExecutionSource,
        status: CommandExecutionStatus,
        command_actions: Vec<CommandAction>,
        aggregated_output: Option<String>,
        exit_code: Option<i32>,
        duration_ms: Option<i64>,
    },
    FileChange {
        id: String,
        changes: Vec<FileUpdateChange>,
        status: PatchApplyStatus,
    },
    McpToolCall {
        id: String,
        server: String,
        tool: String,
        status: McpToolCallStatus,
        arguments: serde_json::Value,
        app_context: Option<McpToolCallAppContext>,
        mcp_app_resource_uri: Option<String>,
        plugin_id: Option<String>,
        result: Option<Box<McpToolCallResult>>,
        error: Option<McpToolCallError>,
        duration_ms: Option<i64>,
    },
    DynamicToolCall {
        id: String,
        namespace: Option<String>,
        tool: String,
        arguments: serde_json::Value,
        status: DynamicToolCallStatus,
        content_items: Option<Vec<serde_json::Value>>,
        success: Option<bool>,
        duration_ms: Option<i64>,
    },
    CollabAgentToolCall {
        id: String,
        tool: CollabAgentTool,
        status: CollabAgentToolCallStatus,
        sender_thread_id: String,
        receiver_thread_ids: Vec<String>,
        prompt: Option<String>,
        model: Option<String>,
        reasoning_effort: Option<String>,
        agents_states: HashMap<String, CollabAgentState>,
    },
    SubAgentActivity {
        id: String,
        kind: SubAgentActivityKind,
        agent_thread_id: String,
        agent_path: String,
    },
    WebSearch(WebSearchItem),
    ImageView {
        id: String,
        path: String,
    },
    Sleep(SleepItem),
    ImageGeneration(ImageGenerationItem),
    EnteredReviewMode {
        id: String,
        review: String,
    },
    ExitedReviewMode {
        id: String,
    },
    ContextCompaction {
        id: String,
        summary: Option<String>,
    },
}

impl Default for ThreadItem {
    fn default() -> Self {
        ThreadItem::UserMessage {
            id: String::new(),
            client_id: None,
            content: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookPromptFragment {
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MessagePhase {
    #[default]
    Final,
    Streaming,
    Analysis,
}

impl MessagePhase {
    pub fn to_core(&self) -> Option<crate::protocol_compat::models::MessagePhase> {
        match self {
            Self::Final => Some(crate::protocol_compat::models::MessagePhase::FinalAnswer),
            Self::Streaming => Some(crate::protocol_compat::models::MessagePhase::Commentary),
            Self::Analysis => Some(crate::protocol_compat::models::MessagePhase::Commentary),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryCitation {
    pub entries: Vec<MemoryCitationEntry>,
    pub thread_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryCitationEntry {
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub note: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebSearchItem {
    pub id: String,
    pub query: String,
    pub action: Option<WebSearchAction>,
    pub results: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SleepItem {
    pub id: String,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageGenerationItem {
    pub id: String,
    pub prompt: String,
    pub url: Option<String>,
    pub revised_prompt: String,
    pub saved_path: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpToolCallAppContext {
    pub resource_uri: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpToolCallResult {
    pub content: Vec<serde_json::Value>,
    pub structured_content: Option<serde_json::Value>,
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpToolCallError {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    Read {
        command: String,
        name: String,
        path: PathBuf,
    },
    ListFiles {
        command: String,
        path: Option<String>,
    },
    Search {
        command: String,
        query: Option<String>,
        path: Option<String>,
    },
    Unknown {
        command: String,
    },
}

impl Default for CommandAction {
    fn default() -> Self {
        CommandAction::Unknown {
            command: String::new(),
        }
    }
}

impl CommandAction {
    /// 转换为 core 的 ParsedCommand。
    pub fn into_core(self) -> crate::protocol_compat::parse_command::ParsedCommand {
        crate::protocol_compat::parse_command::ParsedCommand::default()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommandExecutionSource {
    #[default]
    Agent,
    UserShell,
    UnifiedExecStartup,
    UnifiedExecInteraction,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommandExecutionStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    Declined,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpToolCallStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DynamicToolCallStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PatchApplyStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
    Declined,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PatchChangeKind {
    #[default]
    Add,
    Delete,
    Update {
        move_path: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileUpdateChange {
    pub path: String,
    pub kind: PatchChangeKind,
    pub diff: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CollabAgentTool {
    #[default]
    SpawnAgent,
    SendInput,
    ResumeAgent,
    Wait,
    CloseAgent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CollabAgentToolCallStatus {
    #[default]
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CollabAgentStatus {
    #[default]
    PendingInit,
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
    NotFound,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollabAgentState {
    pub status: CollabAgentStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubAgentActivityKind {
    #[default]
    Started,
    Interacted,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSearchAction {
    Search {
        query: Option<String>,
        queries: Option<Vec<String>>,
    },
    OpenPage {
        url: Option<String>,
    },
    FindInPage {
        url: Option<String>,
        pattern: Option<String>,
    },
    Other,
}

impl Default for WebSearchAction {
    fn default() -> Self {
        WebSearchAction::Other
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemStartedNotification {
    pub item: ThreadItem,
    pub thread_id: String,
    pub turn_id: String,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemCompletedNotification {
    pub item: ThreadItem,
    pub thread_id: String,
    pub turn_id: String,
    pub completed_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentMessageDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReasoningSummaryTextDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
    pub summary_index: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReasoningSummaryPartAddedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub summary_index: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReasoningTextDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
    pub content_index: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalInteractionNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub process_id: String,
    pub stdin: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandExecutionOutputDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileChangeOutputDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorNotification {
    pub error: TurnError,
    pub will_retry: bool,
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WarningNotification {
    pub thread_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardianWarningNotification {
    pub thread_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeprecationNoticeNotification {
    pub summary: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigWarningNotification {
    pub summary: String,
    pub details: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelSafetyBufferingUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub model: String,
    pub use_cases: Vec<String>,
    pub reasons: Vec<String>,
    pub show_buffering_ui: bool,
    pub faster_model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelVerificationNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub verifications: Vec<ModelVerification>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModelVerification {
    #[default]
    TrustedAccessForCyber,
}
