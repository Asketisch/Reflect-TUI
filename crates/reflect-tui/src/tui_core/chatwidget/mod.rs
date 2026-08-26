//! ChatWidget 主模块（facade + 核心状态机）。
//!
//! 注：本文件超过 800 行红线，但 `impl ChatWidget`（约 1012 行，97 个方法）为内聚的完整
//! 状态机，与上游逐行对应，按 AGENTS.md「内聚完整状态机例外」保留。

//! Reflect TUI 主聊天界面的核心模块。
//!
//! `ChatWidget` 消费协议事件，构建并更新历史单元格（history cells），同时驱动主视图和
//! 覆盖层 UI 的渲染。
//!
//! 该 UI 既包含已提交的对话记录单元格（已定型的 `HistoryCell`），也包含一个进行中的活跃
//! 单元格（`ChatWidget.active_cell`），后者在流式输出过程中可以原地变更（通常表示一个
//! 合并后的 exec/工具组）。对话记录覆盖层（`Ctrl+T`）渲染已提交的单元格，外加一个从当前
//! 活跃单元格派生出的、仅供渲染的缓存实时尾部，因此进行中的工具调用能立即显示出来。
//!
//! 对话记录覆盖层由 `App::overlay_forward_event` 保持同步，它在绘制期间通过
//! `active_cell_transcript_key()` 与 `active_cell_transcript_hyperlink_lines()`
//! 同步实时尾部。缓存键的设计目标是：当活跃单元格原地变更，或其对话记录输出
//! 随时间变化时改变取值，从而让覆盖层能够刷新其缓存的尾部，而不必在每次绘制时重建。
//!
//! 底部面板暴露一个单一的「任务运行中」指示器，用于驱动旋转动画（spinner）与中断提示。
//! 本模块将该指示器视为派生的 UI 忙碌状态：当一轮 agent 回合进行中，以及 MCP 服务器启动
//! 进行中时都会置位。这些生命周期被独立追踪（`agent_turn_running` 与
//! `mcp_startup_status`），并通过 `update_task_running_state` 同步。
//!
//! 对于支持前言（preamble）的模型，助手输出可能包含最终答案之前的说明性文字。流式输出期间
//! 我们会隐藏状态行，以避免重复的进度指示；一旦前言完成且流队列清空，我们会重新显示状态行，
//! 使用户仍能在输出间隙看到回合进行中的状态。
//!
//! 斜杠命令的解析位于底部面板的输入框（composer），但斜杠命令的接受逻辑在这里。这种拆分让
//! 输入框在清空输入前暂存一条记忆条目（recall entry），而本模块在分发后像记录普通提交文本
//! 一样记录尝试执行的斜杠命令。
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use crate::app_server_protocol::AddCreditsNudgeCreditType;
use crate::app_server_protocol::AddCreditsNudgeEmailStatus;
use crate::app_server_protocol::AppSummary;
use crate::app_server_protocol::CollabAgentTool;
use crate::app_server_protocol::CollabAgentToolCallStatus;
use crate::app_server_protocol::CommandExecutionRequestApprovalParams;
use crate::app_server_protocol::CommandExecutionSource as ExecCommandSource;
use crate::app_server_protocol::CreditsSnapshot;
use crate::app_server_protocol::ErrorNotification;
use crate::app_server_protocol::FileChangeRequestApprovalParams;
use crate::app_server_protocol::GuardianApprovalReviewAction;
use crate::app_server_protocol::ItemCompletedNotification;
use crate::app_server_protocol::ItemStartedNotification;
use crate::app_server_protocol::McpServerElicitationRequest;
use crate::app_server_protocol::McpServerElicitationRequestParams;
use crate::app_server_protocol::McpServerStatusDetail;
use crate::app_server_protocol::ModelVerification as AppServerModelVerification;
use crate::app_server_protocol::RateLimitReachedType;
use crate::app_server_protocol::RateLimitSnapshot;
use crate::app_server_protocol::ReflectErrorInfo as AppServerReflectErrorInfo;
use crate::app_server_protocol::RequestId as AppServerRequestId;
use crate::app_server_protocol::ReviewTarget;
use crate::app_server_protocol::ServerNotification;
use crate::app_server_protocol::ServerRequest;
use crate::app_server_protocol::SkillMetadata;
use crate::app_server_protocol::SkillsListResponse;
use crate::app_server_protocol::ThreadGoal as AppThreadGoal;
use crate::app_server_protocol::ThreadGoalStatus as AppThreadGoalStatus;
use crate::app_server_protocol::ThreadItem;
use crate::app_server_protocol::ThreadSettings;
use crate::app_server_protocol::ThreadSettingsUpdatedNotification;
use crate::app_server_protocol::ThreadTokenUsage;
use crate::app_server_protocol::ToolRequestUserInputParams;
use crate::app_server_protocol::Turn;
use crate::app_server_protocol::TurnCompletedNotification;
use crate::app_server_protocol::TurnPlanStepStatus;
use crate::app_server_protocol::TurnStatus;
use crate::app_server_protocol::UserInput;
use crate::config_compat::ConfigLayerStackOrdering;
use crate::config_compat::ConstraintResult;
use crate::config_compat::types::ApprovalsReviewer;
use crate::config_compat::types::Notifications;
use crate::config_compat::types::WindowsSandboxModeToml;
use crate::connectors::AppInfo;
use crate::features::FEATURES;
use crate::features::Feature;
#[cfg(test)]
use crate::git_utils::CommitLogEntry;
use crate::git_utils::current_branch_name;
use crate::git_utils::get_git_repo_root;
use crate::git_utils::local_git_branches;
use crate::git_utils::recent_commits;
use crate::otel::RuntimeMetricsSummary;
use crate::otel::SessionTelemetry;
use crate::plugin::PluginCapabilitySummary;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::account::PlanType;
use crate::protocol_compat::approvals::GuardianAssessmentAction;
use crate::protocol_compat::approvals::GuardianAssessmentDecisionSource;
use crate::protocol_compat::approvals::GuardianAssessmentEvent;
use crate::protocol_compat::approvals::GuardianAssessmentStatus;
use crate::protocol_compat::config_types::CollaborationMode;
use crate::protocol_compat::config_types::CollaborationModeMask;
use crate::protocol_compat::config_types::ModeKind;
use crate::protocol_compat::config_types::Personality;
use crate::protocol_compat::config_types::Settings;
#[cfg(any(target_os = "windows", test))]
use crate::protocol_compat::config_types::WindowsSandboxLevel;
use crate::protocol_compat::items::AgentMessageContent;
use crate::protocol_compat::items::AgentMessageItem;
use crate::protocol_compat::models::MessagePhase;
use crate::protocol_compat::plan_tool::PlanItemArg as UpdatePlanItemArg;
use crate::protocol_compat::plan_tool::StepStatus as UpdatePlanItemStatus;
use crate::protocol_compat::request_permissions::RequestPermissionsEvent;
use crate::protocol_compat::user_input::ByteRange;
use crate::protocol_compat::user_input::TextElement;
use crate::terminal_detection::Multiplexer;
use crate::terminal_detection::TerminalInfo;
use crate::terminal_detection::TerminalName;
use crate::terminal_detection::terminal_info;
use crate::tui_core::app::app_server_requests::ResolvedAppServerRequest;
use crate::tui_core::app_command::AppCommand;
use crate::tui_core::app_event::HistoryLookupResponse;
use crate::tui_core::app_server_approval_conversions::file_update_changes_to_display;
use crate::tui_core::approval_events::ApplyPatchApprovalRequestEvent;
use crate::tui_core::approval_events::ExecApprovalRequestEvent;
use crate::tui_core::bottom_pane::StatusLineItem;
use crate::tui_core::bottom_pane::StatusLineSetupView;
use crate::tui_core::bottom_pane::StatusSurfacePreviewData;
use crate::tui_core::bottom_pane::StatusSurfacePreviewItem;
use crate::tui_core::bottom_pane::TerminalTitleItem;
use crate::tui_core::bottom_pane::TerminalTitleSetupView;
use crate::tui_core::diff_model::FileChange;
use crate::tui_core::git_action_directives::parse_assistant_markdown;
use crate::tui_core::legacy_core::config::Config;
use crate::tui_core::legacy_core::config::PermissionProfileSnapshot;
use crate::tui_core::mention_codec::LinkedMention;
use crate::tui_core::mention_codec::encode_history_mentions;
use crate::tui_core::model_catalog::ModelCatalog;
use crate::tui_core::multi_agents;
use crate::tui_core::multi_agents::AgentMetadata;
use crate::tui_core::session_state::SessionNetworkProxyRuntime;
use crate::tui_core::session_state::ThreadSessionState;
use crate::tui_core::status::RateLimitWindowDisplay;
use crate::tui_core::status::StatusAccountDisplay;
use crate::tui_core::status::StatusHistoryHandle;
use crate::tui_core::status::format_directory_display;
use crate::tui_core::status::format_tokens_compact;
use crate::tui_core::status::rate_limit_snapshot_display_for_limit;
use crate::tui_core::terminal_hyperlinks::HyperlinkLine;
use crate::tui_core::terminal_title::SetTerminalTitleResult;
use crate::tui_core::terminal_title::clear_terminal_title;
use crate::tui_core::terminal_title::set_terminal_title;
use crate::tui_core::text_formatting::proper_join;
use crate::tui_core::token_usage::TokenUsage;
use crate::tui_core::token_usage::TokenUsageInfo;
use crate::tui_core::version::REFLECT_CLI_VERSION;
use crate::utils_absolute_path::{AbsolutePathBuf, ToInferredAbsPath};
use crate::utils_cli::resume_hint;
use crate::utils_path_uri::PathUri;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Text;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;
use tokio::sync::mpsc::UnboundedSender;
use tracing::debug;
use tracing::warn;

const DEFAULT_MODEL_DISPLAY_NAME: &str = "loading";
const MULTI_AGENT_ENABLE_TITLE: &str = "Enable subagents?";
const MULTI_AGENT_ENABLE_YES: &str = "Yes, enable";
const MULTI_AGENT_ENABLE_NO: &str = "Not now";
const MULTI_AGENT_ENABLE_NOTICE: &str = "Subagents will be enabled in the next session.";
const TRUSTED_ACCESS_FOR_CYBER_VERIFICATION_WARNING: &str = "Your conversations have multiple flags for possible cybersecurity risk. Responses may take longer because extra safety checks are on.";
const MEMORIES_ENABLE_TITLE: &str = "Enable memories?";
const MEMORIES_ENABLE_YES: &str = "Yes, enable";
const MEMORIES_ENABLE_NO: &str = "Not now";
const MEMORIES_ENABLE_NOTICE: &str = "Memories will be enabled in the next session.";
const PLAN_MODE_REASONING_SCOPE_TITLE: &str = "Apply reasoning change";
const PLAN_MODE_REASONING_SCOPE_PLAN_ONLY: &str = "Apply to Plan mode override";
const PLAN_MODE_REASONING_SCOPE_ALL_MODES: &str = "Apply to global default and Plan mode override";
const CONNECTORS_SELECTION_VIEW_ID: &str = "connectors-selection";
const PET_SELECTION_LOADING_VIEW_ID: &str = "pet-selection-loading";
const AMBIENT_PET_WRAP_GAP_COLUMNS: u16 = 2;
const TUI_STUB_MESSAGE: &str = "Not available in TUI yet.";
const PARENT_OWNED_INPUT_MESSAGE: &str =
    "This sub-agent is controlled by its parent. Direct input is disabled.";

use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event::ExitMode;
use crate::tui_core::app_event::PermissionProfileSelection;
use crate::tui_core::app_event::RateLimitRefreshOrigin;
#[cfg(target_os = "windows")]
use crate::tui_core::app_event::WindowsSandboxEnableMode;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::auto_review_denials;
use crate::tui_core::auto_review_denials::RecentAutoReviewDenials;
use crate::tui_core::bottom_pane::ApplyPatchApprovalRequest;
use crate::tui_core::bottom_pane::ApprovalRequest;
use crate::tui_core::bottom_pane::BottomPane;
use crate::tui_core::bottom_pane::BottomPaneParams;
use crate::tui_core::bottom_pane::CancellationEvent;
use crate::tui_core::bottom_pane::CollaborationModeIndicator;
use crate::tui_core::bottom_pane::ColumnWidthMode;
use crate::tui_core::bottom_pane::DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED;
use crate::tui_core::bottom_pane::ExecApprovalRequest;
use crate::tui_core::bottom_pane::ExperimentalFeatureItem;
use crate::tui_core::bottom_pane::ExperimentalFeaturesView;
use crate::tui_core::bottom_pane::GoalStatusIndicator;
use crate::tui_core::bottom_pane::HistoryEntry;
use crate::tui_core::bottom_pane::InputResult;
use crate::tui_core::bottom_pane::LocalImageAttachment;
use crate::tui_core::bottom_pane::McpElicitationApprovalRequest;
use crate::tui_core::bottom_pane::McpServerElicitationFormRequest;
use crate::tui_core::bottom_pane::MemoriesSettingsView;
use crate::tui_core::bottom_pane::MentionBinding;
use crate::tui_core::bottom_pane::PermissionsApprovalRequest;
use crate::tui_core::bottom_pane::QUIT_SHORTCUT_TIMEOUT;
use crate::tui_core::bottom_pane::QueuedInputAction;
use crate::tui_core::bottom_pane::SelectionAction;
use crate::tui_core::bottom_pane::SelectionItem;
use crate::tui_core::bottom_pane::SelectionViewParams;
use crate::tui_core::bottom_pane::custom_prompt_view::CustomPromptView;
use crate::tui_core::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::tui_core::clipboard_paste::paste_image_to_temp_png;
use crate::tui_core::collaboration_modes;
use crate::tui_core::diff_render::display_path_for;
use crate::tui_core::exec_cell::CommandOutput;
use crate::tui_core::exec_cell::ExecCell;
use crate::tui_core::exec_cell::new_active_exec_command;
use crate::tui_core::exec_command::split_command_string;
use crate::tui_core::exec_command::strip_bash_lc_and_escape;
use crate::tui_core::get_git_diff::get_git_diff;
use crate::tui_core::history_cell;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::history_cell::HistoryRenderMode;
use crate::tui_core::history_cell::HookCell;
use crate::tui_core::history_cell::McpInvocation;
use crate::tui_core::history_cell::McpToolCallCell;
use crate::tui_core::history_cell::PlainHistoryCell;
use crate::tui_core::history_cell::WebSearchCell;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::keymap::ChatKeymap;
use crate::tui_core::keymap::RuntimeKeymap;
use crate::tui_core::render::Insets;
use crate::tui_core::render::renderable::ColumnRenderable;
use crate::tui_core::render::renderable::FlexRenderable;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::render::renderable::RenderableExt;
use crate::tui_core::render::renderable::RenderableItem;
use crate::tui_core::slash_command::SlashCommand;
use crate::tui_core::status::RateLimitSnapshotDisplay;
use crate::tui_core::status::remote_connection::RemoteConnectionStatus;
use crate::tui_core::status_indicator_widget::STATUS_DETAILS_DEFAULT_MAX_LINES;
use crate::tui_core::status_indicator_widget::StatusDetailsCapitalization;
use crate::tui_core::text_formatting::truncate_text;
use crate::tui_core::tui::FrameRequester;
mod command_lifecycle;
mod connectors;
mod constructor;
use self::connectors::ConnectorsState;
mod exec_state;
use self::exec_state::RunningCommand;
use self::exec_state::UnifiedExecProcessSummary;
use self::exec_state::UnifiedExecWaitState;
use self::exec_state::UnifiedExecWaitStreak;
use self::exec_state::command_execution_command_and_parsed;
use self::exec_state::is_standard_tool_call;
use self::exec_state::is_unified_exec_source;
mod goal_status;
use self::goal_status::GoalStatusState;
#[cfg(test)]
use self::goal_status::goal_status_indicator_from_app_goal;
mod goal_menu;
mod ide_context;
use self::ide_context::IdeContextState;
mod input_queue;
use self::input_queue::InputQueueState;
mod input_flow;
mod input_restore;
mod input_submission;
mod interrupts;
use self::interrupts::InterruptManager;
mod keymap_picker;
mod mcp_startup;
use self::mcp_startup::McpStartupStatus;
mod pets;
mod session_flow;
mod session_header;
use self::session_header::SessionHeader;
mod hook_lifecycle;
mod hooks;
mod interaction;
mod skills;
mod slash_dispatch;
use self::skills::collect_tool_mentions;
use self::skills::find_app_mentions;
use self::skills::find_skill_mentions_with_tool_mentions;
use self::skills::is_app_mentionable;
mod plugin_catalog;
mod plugins;
use self::plugins::PluginInstallAuthFlowState;
use self::plugins::PluginListFetchState;
use self::plugins::PluginsCacheState;
mod plan_implementation;
use self::plan_implementation::PLAN_IMPLEMENTATION_TITLE;
mod model_popups;
mod notifications;
use self::notifications::Notification;
mod permission_popups;
mod permissions_menu;
mod protocol;
mod protocol_requests;
mod rate_limits;
use self::rate_limits::RateLimitErrorKind;
use self::rate_limits::RateLimitSwitchPromptState;
use self::rate_limits::RateLimitWarningState;
use self::rate_limits::app_server_rate_limit_error_kind;
pub(crate) use self::rate_limits::fallback_limit_label;
use self::rate_limits::is_app_server_cyber_policy_error;
mod reset_credits;
pub(crate) use self::rate_limits::limit_label_for_window;
mod reasoning_shortcuts;
mod rendering;
mod replay;
mod review;
mod review_popups;
use self::review::ReviewState;
#[cfg(test)]
pub(crate) use self::review_popups::show_review_commit_picker_with_entries;
mod safety_buffering;
mod service_tiers;
mod settings;
mod settings_popups;
mod side;
use self::safety_buffering::SafetyBufferingState;
mod status_state;
mod windows_sandbox_prompts;
use self::status_state::StatusIndicatorState;
use self::status_state::StatusState;
use self::status_state::TerminalTitleStatusKind;
mod status_controls;
mod status_surfaces;
mod streaming;
use self::status_surfaces::CachedProjectRootName;
mod tokens;
mod tool_lifecycle;
mod tool_requests;
mod transcript;
use self::transcript::TranscriptState;
mod turn_lifecycle;
mod turn_runtime;
use self::turn_lifecycle::TurnLifecycleState;
mod usage;
mod user_messages;
use self::user_messages::PendingSteer;
use self::user_messages::PendingSteerCompareKey;
use self::user_messages::QueueDrain;
use self::user_messages::QueuedUserMessage;
use self::user_messages::ShellEscapePolicy;
use self::user_messages::ThreadComposerState;
pub(crate) use self::user_messages::ThreadInputState;
pub(crate) use self::user_messages::ThreadInputStateRestoreMode;
pub(crate) use self::user_messages::UserMessage;
use self::user_messages::UserMessageDisplay;
#[cfg(test)]
use self::user_messages::UserMessageHistoryOverride;
use self::user_messages::UserMessageHistoryRecord;
use self::user_messages::app_server_text_elements;
pub(crate) use self::user_messages::mention_bindings_from_user_inputs;
use self::user_messages::merge_user_messages;
use self::user_messages::merge_user_messages_with_history_record;
#[cfg(test)]
use self::user_messages::remap_placeholders_for_message;
use self::user_messages::user_message_display_for_history;
use self::user_messages::user_message_for_restore;
use self::user_messages::user_message_preview_text;
mod warnings;
use self::warnings::WarningDisplayState;
pub(crate) use crate::tui_core::branch_summary::StatusLineGitSummary;
use crate::tui_core::streaming::chunking::AdaptiveChunkingPolicy;
use crate::tui_core::streaming::commit_tick::CommitTickScope;
use crate::tui_core::streaming::commit_tick::run_commit_tick;
use crate::tui_core::streaming::controller::PlanStreamController;
use crate::tui_core::streaming::controller::StreamController;
use crate::tui_core::workspace_command::WorkspaceCommandRunner;

use crate::app_server_protocol::AskForApproval;
use crate::file_search::FileMatch;
use crate::protocol_compat::models::ActivePermissionProfile;
use crate::protocol_compat::models::PermissionProfile;
use crate::protocol_compat::openai_models::InputModality;
use crate::protocol_compat::openai_models::ModelPreset;
use crate::protocol_compat::openai_models::ReasoningEffort as ReasoningEffortConfig;
use crate::protocol_compat::plan_tool::StepStatus;
use crate::protocol_compat::plan_tool::UpdatePlanArgs;
use crate::utils_approval_presets::ApprovalPreset;
use crate::utils_approval_presets::builtin_approval_presets;
use chrono::Local;
use strum::IntoEnumIterator;
use unicode_segmentation::UnicodeSegmentation;

const USER_SHELL_COMMAND_HELP_TITLE: &str = "Prefix a command with ! to run it locally";
const USER_SHELL_COMMAND_HELP_HINT: &str = "Example: !ls";
const ASK_FOR_APPROVAL_LABEL: &str = "Ask for approval";
const APPROVE_FOR_ME_LABEL: &str = "Approve for me";
const AUTO_REVIEW_DESCRIPTION: &str = "Only ask for actions detected as potentially unsafe.";
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_STATUS_LINE_ITEMS: [&str; 2] = ["model-with-reasoning", "current-dir"];

/// 所有 `ChatWidget` 构造函数共享的通用初始化参数。
pub(crate) struct ChatWidgetInit {
    pub(crate) config: Config,
    pub(crate) frame_requester: FrameRequester,
    pub(crate) app_event_tx: AppEventSender,
    /// 由 app-server 支撑的运行器，供状态面板执行工作区元数据探测。
    ///
    /// 不涉及 git 状态行刷新的测试可以不设置此字段。生产环境中的 TUI
    /// 构造会为当前 app-server 会话提供一个运行器。
    pub(crate) workspace_command_runner: Option<WorkspaceCommandRunner>,
    pub(crate) initial_user_message: Option<UserMessage>,
    pub(crate) enhanced_keys_supported: bool,
    pub(crate) has_chatgpt_account: bool,
    pub(crate) has_reflect_backend_auth: bool,
    pub(crate) model_catalog: Arc<ModelCatalog>,
    pub(crate) feedback: crate::feedback::ReflectFeedback,
    pub(crate) is_first_run: bool,
    pub(crate) status_account_display: Option<StatusAccountDisplay>,
    pub(crate) runtime_model_provider_base_url: Option<String>,
    pub(crate) initial_plan_type: Option<PlanType>,
    pub(crate) model: Option<String>,
    pub(crate) startup_tooltip_override: Option<String>,
    // 共享闩锁：对于无效的状态行条目 ID 只警告一次。
    pub(crate) status_line_invalid_items_warned: Arc<AtomicBool>,
    // 共享闩锁：对于无效的终端标题条目 ID 只警告一次。
    pub(crate) terminal_title_invalid_items_warned: Arc<AtomicBool>,
    pub(crate) session_telemetry: SessionTelemetry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExternalEditorState {
    #[default]
    Closed,
    Requested,
    Active,
}

/// 维护聊天界面的按会话 UI 状态与交互状态机。
///
/// `ChatWidget` 拥有从协议事件流派生的状态（历史单元格、流式缓冲、底部面板覆盖层以及
/// 瞬态状态文本），并将按键转换为用户意图（`Op` 提交与 `AppEvent` 请求）。
///
/// 它不负责运行 agent 本身；它通过更新 UI 状态以及向 agent 运行时发送请求来反映进度。
///
/// 退出/中断行为有意跨层分布：底部面板负责本地输入路由（哪个视图接收 Ctrl+C），而
/// `ChatWidget` 负责进程级决策，例如中断正在执行的工作、启用双击退出快捷键，以及请求
/// 先关停再退出的退出流程。
pub(crate) struct ChatWidget {
    app_event_tx: AppEventSender,
    reflect_op_target: ReflectOpTarget,
    bottom_pane: BottomPane,
    transcript: TranscriptState,
    config: Config,
    raw_output_mode: bool,
    /// 由 core 解析得到的运行时值。`config.service_tier` 仍是用户的显式选择。
    effective_service_tier: Option<String>,
    /// 未加掩码的协作模式设置（始终为 Default 模式）。
    ///
    /// 掩码应用于此基础模式之上，从而推导出有效模式。
    current_collaboration_mode: CollaborationMode,
    /// 当前生效的协作模式掩码（如果有）。
    active_collaboration_mask: Option<CollaborationModeMask>,
    has_chatgpt_account: bool,
    has_reflect_backend_auth: bool,
    model_catalog: Arc<ModelCatalog>,
    session_telemetry: SessionTelemetry,
    session_header: SessionHeader,
    initial_user_message: Option<UserMessage>,
    status_account_display: Option<StatusAccountDisplay>,
    runtime_model_provider_base_url: Option<String>,
    pub(crate) remote_connection: Option<RemoteConnectionStatus>,
    token_info: Option<TokenUsageInfo>,
    rate_limit_snapshots_by_limit_id: BTreeMap<String, RateLimitSnapshotDisplay>,
    refreshing_status_outputs: Vec<(u64, StatusHistoryHandle)>,
    next_status_refresh_request_id: u64,
    refreshing_token_activity_output: Option<tokens::PendingTokenActivityOutput>,
    completed_token_activity_output: Option<history_cell::CompositeHistoryCell>,
    next_token_activity_request_id: u64,
    pending_rate_limit_reset_request_id: Option<u64>,
    pending_rate_limit_reset_idempotency_key: Option<String>,
    rate_limit_reset_picker_request_id: Option<u64>,
    pending_rate_limit_reset_hint_request_id: Option<u64>,
    pending_usage_menu_rate_limit_request_id: Option<u64>,
    pending_rate_limit_reset_hint: Option<PlainHistoryCell>,
    available_rate_limit_reset_credits: Option<i64>,
    next_rate_limit_reset_request_id: u64,
    plan_type: Option<PlanType>,
    reflect_rate_limit_reached_type: Option<RateLimitReachedType>,
    reflect_spend_control_reached: Option<bool>,
    rate_limit_warnings: RateLimitWarningState,
    warning_display_state: WarningDisplayState,
    rate_limit_switch_prompt: RateLimitSwitchPromptState,
    add_credits_nudge_email_in_flight: Option<AddCreditsNudgeCreditType>,
    adaptive_chunking: AdaptiveChunkingPolicy,
    // 流生命周期控制器
    stream_controller: Option<StreamController>,
    // 用于探索计划（proposed plan）输出的流生命周期控制器。
    plan_stream_controller: Option<PlanStreamController>,
    pending_stream_consolidations: usize,
    /// 持有平台剪贴板租约，使复制后的文本在受支持期间保持可用。
    clipboard_lease: Option<crate::tui_core::clipboard_copy::ClipboardLease>,
    copy_last_response_binding: Vec<KeyBinding>,
    running_commands: HashMap<String, RunningCommand>,
    collab_agent_metadata: HashMap<ThreadId, AgentMetadata>,
    pending_collab_spawn_requests: HashMap<String, multi_agents::SpawnRequestSummary>,
    suppressed_exec_calls: HashSet<String>,
    skills_all: Vec<SkillMetadata>,
    skills_initial_state: Option<HashMap<AbsolutePathBuf, bool>>,
    last_unified_wait: Option<UnifiedExecWaitState>,
    unified_exec_wait_streak: Option<UnifiedExecWaitStreak>,
    turn_lifecycle: TurnLifecycleState,
    safety_buffering: SafetyBufferingState,
    task_complete_pending: bool,
    unified_exec_processes: Vec<UnifiedExecProcessSummary>,
    /// 在启动进行中追踪每个服务器的 MCP 启动状态。
    ///
    /// 从收到第一条启动状态更新开始，到 app-server 支撑的启动轮次稳定为止，
    /// 该映射为 `Some(_)`；在此期间即使当前没有 agent 回合在执行，
    /// 底部面板也会被视为「运行中」。
    mcp_startup_status: Option<HashMap<String, McpStartupStatus>>,
    /// 当前启动轮次预期的 MCP 服务器，由已启用的本地配置填充。
    mcp_startup_expected_servers: Option<HashSet<String>>,
    /// 启动稳定后，忽略过期的更新，直到足够多的通知确认新一轮启动。
    mcp_startup_ignore_updates_until_next_start: bool,
    /// 下一轮的滞后信号表明，仅凭终端更新即可使该轮稳定。
    mcp_startup_allow_terminal_only_next_round: bool,
    /// 缓冲稳定后的 MCP 启动更新，直到它们覆盖一个完整的新轮次。
    mcp_startup_pending_next_round: HashMap<String, McpStartupStatus>,
    /// 追踪缓冲的下一轮是否已看到任何 `Starting` 更新。
    mcp_startup_pending_next_round_saw_starting: bool,
    connectors: ConnectorsState,
    ide_context: IdeContextState,
    plugins_cache: PluginsCacheState,
    plugins_fetch_state: PluginListFetchState,
    plugin_remote_sections_loading: bool,
    plugin_remote_sections_loaded: bool,
    plugin_remote_section_errors: Vec<crate::tui_core::app_event::PluginRemoteSectionError>,
    plugin_install_apps_needing_auth: Vec<AppSummary>,
    plugin_install_auth_flow: Option<PluginInstallAuthFlowState>,
    plugins_active_tab_id: Option<String>,
    newly_installed_marketplace_tab_id: Option<String>,
    // 在主动写入周期期间被推迟的中断式 UI 事件队列
    interrupts: InterruptManager,
    // 累积当前推理（reasoning）块文本，用于提取标题
    reasoning_buffer: String,
    // 缓存第一个已完成的加粗标题，使后续增量不必重新扫描整个块。
    reasoning_header: Option<String>,
    // 保留推理摘要各部分的边界，供仅对话记录（transcript-only）录制使用。
    reasoning_summary_parts: Vec<String>,
    status_state: StatusState,
    review: ReviewState,
    // 活跃的 hook 运行在专用的实时单元格中渲染，以便与工具并行执行。
    active_hook_cell: Option<HookCell>,
    // 沉浸式同伴（ambient pet）渲染在对话记录区域之上，绝不会绘制在底部行内。
    ambient_pet: Option<crate::tui_core::pets::AmbientPet>,
    pet_picker_preview_state: crate::tui_core::pets::PetPickerPreviewState,
    pet_picker_preview_pet: Option<crate::tui_core::pets::AmbientPet>,
    pet_picker_preview_request_id: u64,
    pet_picker_preview_image_visible: std::cell::Cell<bool>,
    pet_selection_load_request_id: u64,
    #[cfg(test)]
    pet_image_support_override: Option<crate::tui_core::pets::PetImageSupport>,
    thread_id: Option<ThreadId>,
    /// 在当前线程作用域内应跨草稿编辑而保留的 nudge 关闭记录。
    ///
    /// nudge 只是一种发现辅助，因此一旦用户将其关闭或进入 Plan 模式，我们就在该线程中
    /// 保持隐藏，而不会在每次匹配的草稿上重新弹出。
    dismissed_plan_mode_nudge_scopes: HashSet<PlanModeNudgeScope>,
    thread_name: Option<String>,
    thread_rename_block_message: Option<String>,
    active_side_conversation: bool,
    blocks_direct_input: bool,
    normal_placeholder_text: String,
    side_placeholder_text: String,
    forked_from: Option<ThreadId>,
    interrupted_turn_notice_mode: InterruptedTurnNoticeMode,
    frame_requester: FrameRequester,
    // 会话配置完成后是否包含初始欢迎横幅
    show_welcome_banner: bool,
    // 主启动会话的一次性提示条（tooltip）覆盖。
    startup_tooltip_override: Option<String>,
    // 恢复现有会话（通过恢复选择器选中）时，避免在 SessionConfigured
    // 时立即重绘，以防止无意义的界面闪烁。
    suppress_session_configured_redraw: bool,
    // 在快照恢复期间，将启动提示的提交推迟到重放的历史渲染完成之后，
    // 使恢复/分叉的提示保持时间顺序。
    suppress_initial_user_message_submit: bool,
    input_queue: InputQueueState,
    safety_buffering_prompt: Option<UserMessage>,
    /// 从 `tui.keymap.chat` 解析出的主聊天界面按键绑定。
    chat_keymap: ChatKeymap,
    /// 用于将最近排队的消息弹出回输入框的按键绑定的提示显示。当默认
    /// 绑定集合包含终端特定的回退绑定时，该绑定可能与配置的第一个绑定不同。
    queued_message_edit_hint_binding: Option<KeyBinding>,
    // 在未聚焦时，于下一次 Draw 显示的待处理通知
    pending_notification: Option<Notification>,
    /// 当为 `Some` 时，用户已按下退出快捷键，第二次按下必须在
    /// `quit_shortcut_expires_at` 之前发生。
    quit_shortcut_expires_at: Option<Instant>,
    /// 追踪首先按下的是哪个退出快捷键。
    ///
    /// 我们要求第二次按下与这个键一致，这样 `Ctrl+C` 后跟 `Ctrl+D`
    /// （或反之）不会意外退出。
    quit_shortcut_key: Option<KeyBinding>,
    // 当前回合跨增量快照累积的运行时指标。
    turn_runtime_metrics: RuntimeMetricsSummary,
    last_rendered_width: std::cell::Cell<Option<usize>>,
    // Feedback 接收器，供 /feedback 使用
    feedback: crate::feedback::ReflectFeedback,
    // 当前会话的 rollout 路径（如果已知）
    current_rollout_path: Option<PathBuf>,
    // 当前工作目录（如果已知）
    current_cwd: Option<PathBuf>,
    // 由 app-server 支撑的命令运行器，用于状态行工作区元数据查询。
    workspace_command_runner: Option<WorkspaceCommandRunner>,
    // 当前会话加载的说明（instruction）源文件，由 app-server 提供。
    instruction_source_paths: Vec<PathUri>,
    // SessionConfigured 提供的运行时网络代理绑定地址。
    session_network_proxy: Option<SessionNetworkProxyRuntime>,
    // 共享闩锁：对于无效的状态行条目 ID 只警告一次。
    status_line_invalid_items_warned: Arc<AtomicBool>,
    // 共享闩锁：对于无效的终端标题条目 ID 只警告一次。
    terminal_title_invalid_items_warned: Arc<AtomicBool>,
    // 最近一次发出的终端标题，用于避免写入重复的 OSC 更新。
    pub(crate) last_terminal_title: Option<String>,
    // 终端标题渲染器观察到的最近一次「需要操作」可见状态。
    last_terminal_title_requires_action: bool,
    // 设置 UI 打开时捕获的原始终端标题配置。
    //
    // 外层 `Option` 追踪设置会话是否处于活动状态（`Some` 表示活动，
    // `None` 表示不活动）。内层 `Option<Vec<String>>` 与
    // `config.tui_terminal_title` 的形状一致（使用默认值时内层为 `None`）。
    // 取消或持久化失败时，内层值恢复到 config；确认时外层被设为 `None` 以结束会话。
    terminal_title_setup_original_items: Option<Option<Vec<String>>>,
    // 用于带动画旋转（spinner）前缀标题状态的基础时刻。
    terminal_title_animation_origin: Instant,
    // 按 cwd 键控、用于状态/标题渲染的缓存项目根目录显示名。
    status_line_project_root_name_cache: Option<CachedProjectRootName>,
    // 状态行缓存的 git 分支名（未知则为 None）。
    status_line_branch: Option<String>,
    // 用于解析缓存分支的 CWD；变更时重置分支状态。
    status_line_branch_cwd: Option<PathBuf>,
    // 异步分支查询进行中时为 true。
    status_line_branch_pending: bool,
    // 已尝试为当前 CWD 发起分支查询后为 true。
    status_line_branch_lookup_complete: bool,
    // 当前状态行 CWD 缓存的 PR 与分支变更摘要。
    status_line_git_summary: Option<StatusLineGitSummary>,
    // 用于解析缓存 Git 摘要的 CWD；变更时重置摘要状态。
    status_line_git_summary_cwd: Option<PathBuf>,
    // 异步 Git 摘要查询进行中时为 true。
    status_line_git_summary_pending: bool,
    // 已尝试为当前 CWD 发起 Git 摘要查询后为 true。
    status_line_git_summary_lookup_complete: bool,
    // 状态行缓存的工作区通知头条（headline）。
    status_line_workspace_headline: Option<String>,
    // 当前进行中的异步工作区头条获取的请求 ID。
    status_line_workspace_headline_pending_request_id: Option<u64>,
    // 分配给下一次工作区头条获取的请求 ID。
    next_status_line_workspace_headline_request_id: u64,
    // 上次请求工作区头条获取的时间。
    status_line_workspace_headline_last_requested_at: Option<Instant>,
    // 在后端报告工作区消息特性开关已禁用后置位。
    status_line_workspace_messages_disabled: bool,
    // Plan 模式未激活时，状态行中显示的当前线程目标状态。
    current_goal_status_indicator: Option<GoalStatusIndicator>,
    current_goal_status: Option<GoalStatusState>,
    external_editor_state: ExternalEditorState,
    last_rendered_user_message_display: Option<UserMessageDisplay>,
    last_non_retry_error: Option<(String, String)>,
}

#[cfg_attr(not(test), allow(dead_code))]
enum ReflectOpTarget {
    Direct(UnboundedSender<AppCommand>),
    AppEvent,
}

/// 影响对话记录覆盖层渲染的活跃单元格状态快照。
///
/// 覆盖层为进行中的单元格维护一个缓存的「实时尾部」；此键让它可以廉价地决定
/// 何时在活跃单元格演化时重新计算该尾部。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveCellTranscriptKey {
    /// 用于原地更新的缓存失效（cache-busting）修订号。
    ///
    /// 许多活跃单元格在流式输出过程中被增量更新（例如当 exec 组追加输出或更改状态时），
    /// 而对话记录覆盖层会缓存其实时尾部，因此这个修订号提供了一种廉价的方式来表达
    /// 「同一个活跃单元格，但其对话记录输出现在不同了」。调用方在任何可能影响
    /// `HistoryCell::transcript_lines` 的变更时都应递增它。
    pub(crate) revision: u64,
    /// 活跃单元格是否延续之前的流，这会影响对话记录块之间的间距。
    pub(crate) is_stream_continuation: bool,
    /// 对时间敏感的对话记录输出的可选动画节拍（tick）。
    ///
    /// 当它变化时，即使修订号与宽度不变，覆盖层也会重新计算缓存的尾部；这正是
    /// 覆盖层中 shimmer/旋转动画无需任何底层数据变化即可动起来的方式。
    pub(crate) animation_tick: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum InterruptedTurnNoticeMode {
    #[default]
    Default,
    Suppress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplayKind {
    ResumeInitialMessages,
    ThreadSnapshot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionConfiguredDisplay {
    Normal,
    PromptEdit,
    /// 应用会话状态，但不生成会话信息单元格。
    Quiet,
    SideConversation,
}

/// 用于将 Plan 模式 nudge 的关闭范围限定在单个会话上下文中的作用域。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PlanModeNudgeScope {
    /// 在服务器分配线程 id 之前输入的草稿。
    NewThread,
    /// 与某个已配置线程关联的草稿。
    Thread(ThreadId),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum TurnAbortReason {
    Interrupted,
    BudgetLimited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThreadItemRenderSource {
    Live,
    Replay(ReplayKind),
}

impl ThreadItemRenderSource {
    fn is_replay(self) -> bool {
        matches!(self, Self::Replay(_))
    }

    fn replay_kind(self) -> Option<ReplayKind> {
        match self {
            Self::Live => None,
            Self::Replay(replay_kind) => Some(replay_kind),
        }
    }
}

// ── 审批/权限/指标提取辅助（外移子模块） ──
mod helpers;
use helpers::*;

impl ChatWidget {
    /// 存储或覆盖某个协作（collab）agent 线程的缓存昵称与角色。
    ///
    /// 由 `App::upsert_agent_picker_thread` 和 `App::replace_chat_widget` 调用，
    /// 以保持渲染元数据与导航缓存同步。必须在任何引用该线程的通知被处理之前调用，
    /// 否则渲染的条目将回退为显示原始线程 id。
    pub(crate) fn set_collab_agent_metadata(
        &mut self,
        thread_id: ThreadId,
        agent_nickname: Option<String>,
        agent_role: Option<String>,
    ) {
        self.collab_agent_metadata.insert(
            thread_id,
            AgentMetadata {
                agent_nickname,
                agent_role,
            },
        );
    }

    /// 返回某个线程的缓存元数据；若尚未注册则默认为空。
    fn collab_agent_metadata(&self, thread_id: ThreadId) -> AgentMetadata {
        self.collab_agent_metadata
            .get(&thread_id)
            .cloned()
            .unwrap_or_default()
    }

    fn restore_retry_status_header_if_present(&mut self) {
        if let Some(header) = self.status_state.take_retry_status_header() {
            self.set_status_header(header);
        }
    }

    /// 记录或更新当前 agent 回合的原始 markdown。
    fn record_agent_markdown(&mut self, message: &str) {
        if !message.is_empty() {
            self.transcript.record_agent_markdown(message.to_string());
        }
    }

    pub(crate) fn open_feedback_note(
        &mut self,
        category: crate::tui_core::app_event::FeedbackCategory,
        include_logs: bool,
    ) {
        self.show_feedback_note(category, include_logs);
    }

    fn show_feedback_note(
        &mut self,
        category: crate::tui_core::app_event::FeedbackCategory,
        include_logs: bool,
    ) {
        let view = crate::tui_core::bottom_pane::FeedbackNoteView::new(
            category,
            self.turn_lifecycle.last_turn_id.clone(),
            self.app_event_tx.clone(),
            include_logs,
        );
        self.bottom_pane.show_view(Box::new(view));
        self.request_redraw();
    }

    pub(crate) fn open_app_link_view(
        &mut self,
        params: crate::tui_core::bottom_pane::AppLinkViewParams,
    ) {
        let view = crate::tui_core::bottom_pane::AppLinkView::new_with_keymap(
            params,
            self.app_event_tx.clone(),
            self.bottom_pane.list_keymap(),
        );
        self.bottom_pane.show_view(Box::new(view));
        self.request_redraw();
    }

    pub(crate) fn dismiss_app_server_request(&mut self, request: &ResolvedAppServerRequest) {
        // 已在远端解决的请求不得保持为用户可操作状态。它可能已在底部面板物化，
        // 也可能仍被推迟在活跃的流式输出之后。
        let removed_deferred = self.interrupts.remove_resolved_prompt(request);
        let removed_visible = self.bottom_pane.dismiss_app_server_request(request);
        if removed_deferred || removed_visible {
            self.request_redraw();
        }
    }

    pub(crate) fn open_feedback_consent(
        &mut self,
        category: crate::tui_core::app_event::FeedbackCategory,
    ) {
        let snapshot = self.feedback.snapshot();
        #[cfg(target_os = "windows")]
        let include_windows_sandbox_log =
            crate::tui_core::windows_sandbox::current_log_file_path_for_reflect_home(
                &self.config.reflect_home,
            )
            .is_file();
        #[cfg(not(target_os = "windows"))]
        let include_windows_sandbox_log = false;
        let params = crate::tui_core::bottom_pane::feedback_upload_consent_params(
            self.app_event_tx.clone(),
            category,
            self.current_rollout_path.clone(),
            self.thread_id
                .map(|thread_id| format!("auto-review-rollout-{thread_id}.jsonl")),
            include_windows_sandbox_log,
            &snapshot,
        );
        self.bottom_pane.show_selection_view(params);
        self.request_redraw();
    }

    pub(crate) fn open_multi_agent_enable_prompt(&mut self) {
        let items = vec![
            SelectionItem {
                name: MULTI_AGENT_ENABLE_YES.to_string(),
                description: Some(
                    "Save the setting now. You will need a new session to use it.".to_string(),
                ),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::UpdateFeatureFlags {
                        updates: vec![(Feature::Collab, true)],
                    });
                    tx.send(AppEvent::InsertHistoryCell(Box::new(
                        history_cell::new_warning_event(MULTI_AGENT_ENABLE_NOTICE.to_string()),
                    )));
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: MULTI_AGENT_ENABLE_NO.to_string(),
                description: Some("Keep subagents disabled.".to_string()),
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(MULTI_AGENT_ENABLE_TITLE.to_string()),
            subtitle: Some("Subagents are currently disabled in your config.".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            ..Default::default()
        });
    }

    pub(crate) fn open_memories_popup(&mut self) {
        if !self.config.features.enabled(Feature::MemoryTool) {
            self.open_memories_enable_prompt();
            return;
        }

        let view = MemoriesSettingsView::new(
            self.config.memories.use_memories,
            self.config.memories.generate_memories,
            self.app_event_tx.clone(),
            self.bottom_pane.list_keymap(),
        );
        self.bottom_pane.show_view(Box::new(view));
    }

    pub(crate) fn open_memories_enable_prompt(&mut self) {
        let items = vec![
            SelectionItem {
                name: MEMORIES_ENABLE_YES.to_string(),
                description: Some(
                    "Save the setting now. You will need a new session to use it.".to_string(),
                ),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::UpdateFeatureFlags {
                        updates: vec![(Feature::MemoryTool, true)],
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: MEMORIES_ENABLE_NO.to_string(),
                description: Some("Keep memories disabled.".to_string()),
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(MEMORIES_ENABLE_TITLE.to_string()),
            subtitle: Some("Memories are currently disabled in your config.".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            ..Default::default()
        });
    }

    pub(crate) fn set_memory_settings(&mut self, use_memories: bool, generate_memories: bool) {
        self.config.memories.use_memories = use_memories;
        self.config.memories.generate_memories = generate_memories;
    }

    pub(crate) fn set_token_info(&mut self, info: Option<TokenUsageInfo>) {
        match info {
            Some(info) => self.apply_token_info(info),
            None => {
                self.bottom_pane
                    .set_context_window(/*percent*/ None, /*used_tokens*/ None);
                self.token_info = None;
            }
        }
    }

    fn apply_token_info(&mut self, info: TokenUsageInfo) {
        let percent = self.context_remaining_percent(&info);
        let used_tokens = self.context_used_tokens(&info, percent.is_some());
        self.bottom_pane.set_context_window(percent, used_tokens);
        self.token_info = Some(info);
    }

    fn context_remaining_percent(&self, info: &TokenUsageInfo) -> Option<i64> {
        info.model_context_window.map(|window| {
            info.last_token_usage
                .percent_of_context_window_remaining(window)
        })
    }

    fn context_used_tokens(&self, info: &TokenUsageInfo, percent_known: bool) -> Option<i64> {
        if percent_known {
            return None;
        }

        Some(info.total_token_usage.tokens_in_context_window())
    }

    fn restore_pre_review_token_info(&mut self) {
        if let Some(saved) = self.review.pre_review_token_info.take() {
            match saved {
                Some(info) => self.apply_token_info(info),
                None => {
                    self.bottom_pane
                        .set_context_window(/*percent*/ None, /*used_tokens*/ None);
                    self.token_info = None;
                }
            }
        }
    }

    pub(crate) fn handle_history_entry_response(&mut self, event: HistoryLookupResponse) {
        self.bottom_pane.on_history_lookup_response(event);
    }

    pub(crate) fn pre_draw_tick(&mut self) {
        self.update_due_hook_visibility();
        self.schedule_hook_timer_if_needed();
        self.bottom_pane.pre_draw_tick();
        if let Some(pet) = self.ambient_pet.as_ref() {
            pet.schedule_next_frame();
        }
        self.refresh_plan_mode_nudge();
        self.refresh_goal_status_indicator_for_time_tick();
        if self.terminal_title_shows_action_required() != self.last_terminal_title_requires_action {
            self.refresh_terminal_title();
        }
        if self.should_animate_terminal_title_spinner()
            || self.should_animate_terminal_title_action_required()
        {
            self.refresh_terminal_title();
        }
        self.refresh_status_line_if_workspace_headline_due();
    }

    fn flush_active_cell(&mut self) {
        if let Some(active) = self.transcript.active_cell.take() {
            self.transcript.needs_final_message_separator = true;
            self.app_event_tx.send(AppEvent::InsertHistoryCell(active));
            self.request_pending_usage_output_insertion();
        }
    }

    pub(crate) fn add_to_history(&mut self, cell: impl HistoryCell + 'static) {
        self.add_boxed_history(Box::new(cell));
    }

    fn add_boxed_history(&mut self, cell: Box<dyn HistoryCell>) {
        // 在真实会话信息到达之前，将占位会话头保留为活跃单元格，
        // 这样我们可以合并会话头，而不是向历史提交一个重复的框。
        let keep_placeholder_header_active = !self.is_session_configured()
            && self
                .transcript
                .active_cell
                .as_ref()
                .is_some_and(|c| c.as_any().is::<history_cell::SessionHeaderHistoryCell>());

        if !keep_placeholder_header_active && !cell.display_lines(u16::MAX).is_empty() {
            // 仅当单元格渲染出可见行时才中断 exec 分组。
            if !self.has_active_stream_tail() {
                self.flush_active_cell();
            }
            self.transcript.needs_final_message_separator = true;
        }
        self.app_event_tx.send(AppEvent::InsertHistoryCell(cell));
    }

    fn enter_review_mode_with_hint(&mut self, hint: String, from_replay: bool) {
        if self.review.pre_review_token_info.is_none() {
            self.review.pre_review_token_info = Some(self.token_info.clone());
        }
        if !from_replay && !self.bottom_pane.is_task_running() {
            self.bottom_pane.set_task_running(/*running*/ true);
        }
        self.review.is_review_mode = true;
        let banner = format!(">> Code review started: {hint} <<");
        self.add_to_history(history_cell::new_review_status_line(banner));
        self.request_redraw();
    }

    fn exit_review_mode_after_item(&mut self) {
        self.flush_answer_stream_with_separator();
        self.flush_interrupt_queue();
        self.flush_active_cell();
        self.review.is_review_mode = false;
        self.restore_pre_review_token_info();
        self.add_to_history(history_cell::new_review_status_line(
            "<< Code review finished >>".to_string(),
        ));
        self.request_redraw();
    }

    fn on_committed_user_message(&mut self, items: &[UserInput], from_replay: bool) {
        let display = Self::user_message_display_from_inputs(items);
        if from_replay {
            if self.review.is_review_mode {
                return;
            }
            self.bottom_pane
                .record_replayed_user_message_history(HistoryEntry {
                    text: display.message.clone(),
                    text_elements: display.text_elements.clone(),
                    local_image_paths: display.local_images.clone(),
                    remote_image_urls: display.remote_image_urls.clone(),
                    mention_bindings: mention_bindings_from_user_inputs(items, &display.message),
                    pending_pastes: Vec::new(),
                });
            self.on_user_message_display(display);
            return;
        }

        let compare_key = Self::pending_steer_compare_key_from_items(items);
        if self
            .input_queue
            .pending_steers
            .front()
            .is_some_and(|pending| pending.compare_key == compare_key)
        {
            if let Some(pending) = self.input_queue.pending_steers.pop_front() {
                self.refresh_pending_input_preview();
                let pending_display =
                    user_message_display_for_history(pending.user_message, &pending.history_record);
                self.on_user_message_display(pending_display);
            } else if self.last_rendered_user_message_display.as_ref() != Some(&display) {
                tracing::warn!(
                    "pending steer matched compare key but queue was empty when rendering committed user message"
                );
                self.on_user_message_display(display);
            }
        } else if !self.review.is_review_mode
            && self.last_rendered_user_message_display.as_ref() != Some(&display)
        {
            self.on_user_message_display(display);
        }
    }

    fn on_user_message_display(&mut self, display: UserMessageDisplay) {
        self.last_rendered_user_message_display = Some(display.clone());
        if !display.message.trim().is_empty()
            || !display.text_elements.is_empty()
            || !display.local_images.is_empty()
            || !display.remote_image_urls.is_empty()
        {
            self.add_to_history(history_cell::new_user_prompt(
                display.message,
                display.text_elements,
                display.local_images,
                display.remote_image_urls,
            ));
        }

        // 用户消息会重置分隔符状态，使下一条 agent 响应不会添加多余的分隔行。
        self.transcript.needs_final_message_separator = false;
    }

    /// 立即退出 UI，无需等待关停。
    ///
    /// 对于用户主动触发的退出，应优先使用 [`Self::request_quit_without_confirmation`]；
    /// 此方法主要用于关停完成或紧急退出的兜底。
    fn request_immediate_exit(&self) {
        self.app_event_tx.send(AppEvent::Exit(ExitMode::Immediate));
    }

    /// 请求先关停再退出的退出流程。
    ///
    /// 用于显式退出命令（`/quit`、`/exit`、`/logout`）以及
    /// 双击 Ctrl+C/Ctrl+D 的退出快捷键。
    fn request_quit_without_confirmation(&self) {
        self.app_event_tx
            .send(AppEvent::Exit(ExitMode::ShutdownFirst));
    }

    pub(crate) fn show_shutdown_in_progress(&mut self) {
        self.bottom_pane.show_shutdown_in_progress();
    }

    fn request_redraw(&mut self) {
        self.frame_requester.schedule_frame();
    }

    fn bump_active_cell_revision(&mut self) {
        self.transcript.bump_active_cell_revision();
    }

    /// 将活跃单元格标记为失败（✗）并将其刷入历史。
    fn finalize_active_cell_as_failed(&mut self) {
        if let Some(mut cell) = self.transcript.active_cell.take() {
            // 将定型后的单元格插入历史，并保持分组一致。
            if let Some(exec) = cell.as_any_mut().downcast_mut::<ExecCell>() {
                exec.mark_failed();
            } else if let Some(tool) = cell.as_any_mut().downcast_mut::<McpToolCallCell>() {
                tool.mark_failed();
            }
            self.add_boxed_history(cell);
            self.request_pending_usage_output_insertion();
        }
    }

    pub(crate) fn set_pending_thread_approvals(&mut self, threads: Vec<String>) {
        self.bottom_pane.set_pending_thread_approvals(threads);
    }

    pub(crate) fn clear_thread_rename_block(&mut self) {
        self.thread_rename_block_message = None;
    }

    pub(crate) fn set_thread_rename_block_message(&mut self, message: impl Into<String>) {
        self.thread_rename_block_message = Some(message.into());
    }

    pub(crate) fn set_interrupted_turn_notice_mode(&mut self, mode: InterruptedTurnNoticeMode) {
        self.interrupted_turn_notice_mode = mode;
    }

    pub(crate) fn add_diff_in_progress(&mut self) {
        self.request_redraw();
    }

    pub(crate) fn on_diff_complete(&mut self) {
        self.request_redraw();
    }

    pub(crate) fn add_debug_config_output(&mut self) {
        self.add_to_history(crate::tui_core::debug_config::new_debug_config_output(
            &self.config,
            self.session_network_proxy.as_ref(),
        ));
    }

    pub(crate) fn add_ps_output(&mut self) {
        let processes = self
            .unified_exec_processes
            .iter()
            .map(|process| history_cell::UnifiedExecProcessDetails {
                command_display: process.command_display.clone(),
                recent_chunks: process.recent_chunks.clone(),
            })
            .collect();
        self.add_to_history(history_cell::new_unified_exec_processes_output(processes));
    }

    fn clean_background_terminals(&mut self) {
        self.submit_op(AppCommand::clean_background_terminals());
        self.unified_exec_processes.clear();
        self.sync_unified_exec_footer();
        self.add_info_message(
            "Stopping all background terminals.".to_string(),
            /*hint*/ None,
        );
    }

    fn plugins_for_mentions(&self) -> Option<&[PluginCapabilitySummary]> {
        if !self.config.features.enabled(Feature::Plugins) {
            return None;
        }

        self.bottom_pane.plugins().map(Vec::as_slice)
    }

    /// 在会话配置期间构建一个占位头单元格。
    fn placeholder_session_header_cell(config: &Config) -> Box<dyn HistoryCell> {
        let placeholder_style = Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC);
        Box::new(
            history_cell::SessionHeaderHistoryCell::new_with_style(
                DEFAULT_MODEL_DISPLAY_NAME.to_string(),
                placeholder_style,
                /*reasoning_effort*/ None,
                /*show_fast_status*/ false,
                config.cwd.to_path_buf(),
                REFLECT_CLI_VERSION,
            )
            .with_yolo_mode(history_cell::is_yolo_mode(config)),
        )
    }

    /// 将真实会话信息单元格与占位头合并，以避免出现两个框。
    fn apply_session_info_cell(&mut self, cell: history_cell::SessionInfoCell) {
        let mut session_info_cell = Some(Box::new(cell) as Box<dyn HistoryCell>);
        let merged_header = if let Some(active) = self.transcript.active_cell.take() {
            if active
                .as_any()
                .is::<history_cell::SessionHeaderHistoryCell>()
            {
                // 复用现有的占位头，以避免渲染两个框。
                if let Some(cell) = session_info_cell.take() {
                    self.transcript.active_cell = Some(cell);
                }
                true
            } else {
                self.transcript.active_cell = Some(active);
                false
            }
        } else {
            false
        };

        self.flush_active_cell();

        if !merged_header && let Some(cell) = session_info_cell {
            self.add_boxed_history(cell);
        }
    }

    pub(crate) fn add_info_message(&mut self, message: String, hint: Option<String>) {
        self.add_to_history(history_cell::new_info_event(message, hint));
        self.request_redraw();
    }

    pub(crate) fn add_memories_enable_notice(&mut self) {
        self.add_to_history(history_cell::new_warning_event(
            MEMORIES_ENABLE_NOTICE.to_string(),
        ));
        self.request_redraw();
    }

    pub(crate) fn add_plain_history_lines(&mut self, lines: Vec<Line<'static>>) {
        self.add_boxed_history(Box::new(PlainHistoryCell::new(lines)));
        self.request_redraw();
    }

    pub(crate) fn add_warning_message(&mut self, message: String) {
        self.add_to_history(history_cell::new_warning_event(message));
        self.request_redraw();
    }

    pub(crate) fn add_error_message(&mut self, message: String) {
        self.add_to_history(history_cell::new_error_event(message));
        self.request_redraw();
    }

    fn add_app_server_stub_message(&mut self, feature: &str) {
        warn!(feature, "stubbed unsupported TUI feature");
        self.add_error_message(format!("{feature}: {TUI_STUB_MESSAGE}"));
    }

    fn rename_confirmation_cell(name: &str, _thread_id: Option<ThreadId>) -> PlainHistoryCell {
        let mut line = vec![
            "• ".into(),
            "Session renamed to ".into(),
            name.to_string().cyan(),
        ];
        if let Some(hint) = resume_hint() {
            line.extend([". To resume this session run ".into(), hint.cyan()]);
        }
        PlainHistoryCell::new(vec![line.into()])
    }

    /// 开始异步的 MCP 清单（inventory）流程：显示加载 spinner，并通过
    /// `AppEvent::FetchMcpInventory` 请求 app-server 获取。
    ///
    /// spinner 位于 `active_cell` 中，结果到达后由
    /// [`clear_mcp_inventory_loading`] 清除。
    pub(crate) fn add_mcp_output(&mut self, detail: McpServerStatusDetail) {
        self.flush_answer_stream_with_separator();
        self.flush_active_cell();
        self.transcript.active_cell = Some(Box::new(history_cell::new_mcp_inventory_loading(
            self.config.animations,
        )));
        self.bump_active_cell_revision();
        self.request_redraw();
        self.app_event_tx.send(AppEvent::FetchMcpInventory {
            detail,
            thread_id: self.thread_id(),
        });
    }

    /// 如果 MCP 加载 spinner 仍是活跃单元格，则将其移除。
    ///
    /// 使用基于 `Any` 的类型检查，使迟到的清单结果不会意外清除
    /// 期间设置的无关单元格。
    pub(crate) fn clear_mcp_inventory_loading(&mut self) {
        let Some(active) = self.transcript.active_cell.as_ref() else {
            return;
        };
        if !active
            .as_any()
            .is::<history_cell::McpInventoryLoadingCell>()
        {
            return;
        }
        self.transcript.active_cell = None;
        self.bump_active_cell_revision();
        self.request_redraw();
    }

    /// 将文件搜索结果转发到底部面板。
    pub(crate) fn apply_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        self.bottom_pane.on_file_search_result(query, matches);
    }

    /// 返回活跃流可用的 markdown 正文宽度。
    ///
    /// 流式控制器只渲染消息正文，而历史单元格会在正文周围添加项目符号、
    /// 槽位（gutter）或计划内边距。调用方传入该包装层预留的列数，
    /// 使实时输出使用与定型单元格在重排（reflow）时相同的宽度。
    fn current_stream_width(&self, reserved_cols: usize) -> Option<usize> {
        self.last_rendered_width.get().and_then(|width| {
            if width == 0 {
                None
            } else {
                let width = u16::try_from(width).unwrap_or(u16::MAX);
                let width = usize::from(self.history_wrap_width(width));
                Some(
                    crate::tui_core::width::usable_content_width(width, reserved_cols).unwrap_or(1),
                )
            }
        })
    }

    pub(crate) fn raw_output_mode(&self) -> bool {
        self.raw_output_mode
    }

    pub(crate) fn history_render_mode(&self) -> HistoryRenderMode {
        if self.raw_output_mode {
            HistoryRenderMode::Raw
        } else {
            HistoryRenderMode::Rich
        }
    }

    pub(crate) fn set_raw_output_mode(&mut self, enabled: bool) {
        self.raw_output_mode = enabled;
        self.config.tui_raw_output_mode = enabled;
        let render_mode = self.history_render_mode();
        if let Some(controller) = self.stream_controller.as_mut() {
            controller.set_render_mode(render_mode);
        }
        if let Some(controller) = self.plan_stream_controller.as_mut() {
            controller.set_render_mode(render_mode);
        }
        self.refresh_status_surfaces();
    }

    pub(crate) fn raw_output_mode_notice(enabled: bool) -> &'static str {
        if enabled {
            "Raw output mode on: transcript text is shown for clean terminal selection."
        } else {
            "Raw output mode off: rich transcript rendering restored."
        }
    }

    pub(crate) fn set_raw_output_mode_and_notify(&mut self, enabled: bool) {
        self.set_raw_output_mode(enabled);
        self.add_info_message(
            Self::raw_output_mode_notice(enabled).to_string(),
            /*hint*/ None,
        );
    }

    pub(crate) fn toggle_raw_output_mode_and_notify(&mut self) -> bool {
        let enabled = !self.raw_output_mode;
        self.set_raw_output_mode_and_notify(enabled);
        enabled
    }

    /// 在终端宽度变化后更新对尺寸敏感的聊天组件状态。
    ///
    /// 实时流的换行与当前视口保持一致，而定型对话记录的重建则通过
    /// 应用级的尺寸重排（resize reflow）完成。
    pub(crate) fn on_terminal_resize(&mut self, width: u16) {
        let had_rendered_width = self.last_rendered_width.get().is_some();
        self.last_rendered_width.set(Some(width as usize));
        let stream_width = self.current_stream_width(/*reserved_cols*/ 2);
        let plan_stream_width = self.current_stream_width(/*reserved_cols*/ 4);
        if let Some(controller) = self.stream_controller.as_mut() {
            controller.set_width(stream_width);
        }
        if let Some(controller) = self.plan_stream_controller.as_mut() {
            controller.set_width(plan_stream_width);
        }
        self.sync_active_stream_tail();
        if !had_rendered_width {
            self.request_redraw();
        }
    }

    /// 是否有 agent 消息流处于活动状态（不是计划流）。
    pub(crate) fn has_active_agent_stream(&self) -> bool {
        self.stream_controller.is_some()
    }

    /// 是否有探索计划（proposed-plan）流处于活动状态。
    pub(crate) fn has_active_plan_stream(&self) -> bool {
        self.plan_stream_controller.is_some()
    }

    fn is_plan_streaming_in_tui(&self) -> bool {
        self.plan_stream_controller.is_some()
    }

    pub(crate) fn composer_is_empty(&self) -> bool {
        self.bottom_pane.composer_is_empty()
    }

    #[cfg(test)]
    pub(crate) fn is_task_running_for_test(&self) -> bool {
        self.bottom_pane.is_task_running()
    }

    pub(crate) fn toggle_vim_mode_and_notify(&mut self) {
        let enabled = self.bottom_pane.toggle_vim_enabled();
        let message = if enabled {
            "Vim mode enabled."
        } else {
            "Vim mode disabled."
        };
        self.add_info_message(message.to_string(), /*hint*/ None);
    }

    /// 当 UI 处于常规输入框状态、没有运行中的任务、
    /// 没有模态覆盖层（例如审批或状态指示器）、也没有输入框弹窗时为 true。
    /// 在此状态下启用 Esc-Esc 回退（backtracking）。
    pub(crate) fn is_normal_backtrack_mode(&self) -> bool {
        self.bottom_pane.is_normal_backtrack_mode()
    }

    pub(crate) fn should_handle_vim_insert_escape(&self, key_event: KeyEvent) -> bool {
        self.bottom_pane
            .composer_should_handle_vim_insert_escape(key_event)
    }

    pub(crate) fn insert_str(&mut self, text: &str) {
        self.bottom_pane.insert_str(text);
    }

    pub(crate) fn set_remote_image_urls(&mut self, remote_image_urls: Vec<String>) {
        self.bottom_pane.set_remote_image_urls(remote_image_urls);
    }

    fn take_remote_image_urls(&mut self) -> Vec<String> {
        self.bottom_pane.take_remote_image_urls()
    }

    #[cfg(test)]
    pub(crate) fn remote_image_urls(&self) -> Vec<String> {
        self.bottom_pane.remote_image_urls()
    }

    #[cfg(test)]
    pub(crate) fn pending_thread_approvals(&self) -> &[String] {
        self.bottom_pane.pending_thread_approvals()
    }

    #[cfg(test)]
    pub(crate) fn has_active_view(&self) -> bool {
        self.bottom_pane.has_active_view()
    }

    pub(crate) fn show_esc_backtrack_hint(&mut self) {
        self.bottom_pane.show_esc_backtrack_hint();
    }

    pub(crate) fn clear_esc_backtrack_hint(&mut self) {
        self.bottom_pane.clear_esc_backtrack_hint();
    }

    fn refresh_skills_for_current_cwd(&mut self, force_reload: bool) {
        self.submit_op(AppCommand::list_skills(
            vec![self.config.cwd.to_path_buf()],
            force_reload,
        ));
    }

    /// 将命令直接转发给 agent。
    pub(crate) fn submit_op<T>(&mut self, op: T) -> bool
    where
        T: Into<AppCommand>,
    {
        let op: AppCommand = op.into();
        if self.blocks_direct_input
            && matches!(
                &op,
                AppCommand::UserTurn { .. } | AppCommand::Review { .. } | AppCommand::Compact
            )
        {
            self.add_error_message(PARENT_OWNED_INPUT_MESSAGE.to_string());
            return false;
        }
        self.prepare_local_op_submission(&op);
        if op.is_review() && !self.bottom_pane.is_task_running() {
            self.bottom_pane.set_task_running(/*running*/ true);
        }
        match &self.reflect_op_target {
            ReflectOpTarget::Direct(reflect_op_tx) => {
                crate::tui_core::session_log::log_outbound_op(&op);
                if let Err(e) = reflect_op_tx.send(op) {
                    tracing::error!("failed to submit op: {e}");
                    return false;
                }
            }
            ReflectOpTarget::AppEvent => {
                self.app_event_tx.send(AppEvent::ReflectOp(op));
            }
        }
        true
    }

    fn append_message_history_entry(&self, text: String) {
        let Some(thread_id) = self.thread_id else {
            tracing::warn!("failed to append to message history: no active thread id");
            return;
        };
        self.app_event_tx
            .send(AppEvent::AppendMessageHistoryEntry { thread_id, text });
    }

    pub(crate) fn prepare_local_op_submission(&mut self, op: &AppCommand) {
        if matches!(op, AppCommand::Interrupt) && self.turn_lifecycle.agent_turn_running {
            if let Some(controller) = self.stream_controller.as_mut() {
                controller.clear_queue();
            }
            if let Some(controller) = self.plan_stream_controller.as_mut() {
                controller.clear_queue();
            }
            self.clear_active_stream_tail();
            self.request_redraw();
        }
    }

    fn on_list_skills(&mut self, ev: SkillsListResponse) {
        self.set_skills_from_response(&ev);
        self.refresh_plugin_mentions();
    }

    pub(crate) fn refresh_plugin_mentions(&mut self) {
        if !self.config.features.enabled(Feature::Plugins) {
            self.bottom_pane.set_plugin_mentions(/*plugins*/ None);
            return;
        }

        self.app_event_tx.send(AppEvent::RefreshPluginMentions);
    }

    pub(crate) fn on_plugin_mentions_loaded(
        &mut self,
        plugins: Option<Vec<PluginCapabilitySummary>>,
    ) {
        if self.bottom_pane.plugins() == plugins.as_ref() {
            return;
        }
        self.bottom_pane.set_plugin_mentions(plugins);
    }

    pub(crate) fn sync_plugin_mentions_config(&mut self, config: &Config) {
        self.config.features = config.features.clone();
        self.config.config_layer_stack = config.config_layer_stack.clone();
        self.config.memories = config.memories.clone();
        self.config.terminal_resize_reflow = config.terminal_resize_reflow.clone();
        self.sync_mentions_v2_enabled();
    }

    pub(crate) fn token_usage(&self) -> TokenUsage {
        self.token_info
            .as_ref()
            .map(|ti| ti.total_token_usage.clone())
            .unwrap_or_default()
    }

    pub(crate) fn thread_id(&self) -> Option<ThreadId> {
        self.thread_id
    }

    pub(crate) fn thread_name(&self) -> Option<String> {
        self.thread_name.clone()
    }

    /// 返回当前线程预计算的 rollout 路径。
    ///
    /// 对于全新的非临时线程，该路径可能在文件物化之前就已存在；
    /// rollout 的持久化会推迟到第一条用户消息被记录之后。
    pub(crate) fn rollout_path(&self) -> Option<PathBuf> {
        self.current_rollout_path.clone()
    }

    /// 返回描述当前进行中的单元格的缓存键，供对话记录覆盖层使用。
    ///
    /// `Ctrl+T` 渲染已提交的对话记录单元格，外加从当前活跃、hook 以及异步用量单元格
    /// 派生出的仅供渲染的实时尾部；覆盖层缓存该尾部，而这个键正是它用来判断是否需要
    /// 重新计算的依据。当没有任何实时单元格时，此方法返回 `None`，使覆盖层可以完全
    /// 丢弃尾部。
    ///
    /// 如果调用方在未递增修订号（或未提供合适的动画节拍）的情况下变更了活跃单元格的
    /// 对话记录输出，覆盖层将在主视口更新的同时继续显示过期的尾部。
    pub(crate) fn active_cell_transcript_key(&self) -> Option<ActiveCellTranscriptKey> {
        let cell = self.transcript.active_cell.as_ref();
        let hook_cell = self.active_hook_cell.as_ref();
        let token_activity_cell = self.pending_token_activity_output();
        let rate_limit_reset_hint = self.pending_rate_limit_reset_hint();
        if cell.is_none()
            && hook_cell.is_none()
            && token_activity_cell.is_none()
            && rate_limit_reset_hint.is_none()
        {
            return None;
        }
        Some(ActiveCellTranscriptKey {
            revision: self.transcript.active_cell_revision,
            is_stream_continuation: cell
                .map(|cell| cell.is_stream_continuation())
                .unwrap_or(false),
            animation_tick: cell
                .and_then(|cell| cell.transcript_animation_tick())
                .or_else(|| {
                    hook_cell.and_then(super::history_cell::HistoryCell::transcript_animation_tick)
                }),
        })
    }

    /// 返回给定终端宽度下活跃单元格的带注释对话记录行。
    ///
    /// 这是为对话记录覆盖层实时尾部路径提供的便利方法，并且它有意
    /// 过滤掉空结果，使覆盖层能把「没有可渲染内容」当作「无尾部」处理。调用方
    /// 应传入覆盖层使用的相同宽度；使用不同宽度将导致主视口与对话记录覆盖层
    /// 之间的换行不一致。
    pub(crate) fn active_cell_transcript_hyperlink_lines(
        &self,
        width: u16,
    ) -> Option<Vec<HyperlinkLine>> {
        let mut lines = Vec::new();
        if let Some(cell) = self.transcript.active_cell.as_ref() {
            lines.extend(cell.transcript_hyperlink_lines(width));
        }
        if let Some(hook_cell) = self.active_hook_cell.as_ref() {
            // 先计算 hook 行，使隐藏的 hook 不会添加分隔符。
            let hook_lines = hook_cell.transcript_hyperlink_lines(width);
            if !hook_lines.is_empty() && !lines.is_empty() {
                lines.push(HyperlinkLine::from(""));
            }
            lines.extend(hook_lines);
        }
        if let Some(token_activity_cell) = self.pending_token_activity_output() {
            let token_activity_lines = token_activity_cell.transcript_hyperlink_lines(width);
            if !token_activity_lines.is_empty() && !lines.is_empty() {
                lines.push(HyperlinkLine::from(""));
            }
            lines.extend(token_activity_lines);
        }
        if let Some(rate_limit_reset_hint) = self.pending_rate_limit_reset_hint() {
            let hint_lines = rate_limit_reset_hint.transcript_hyperlink_lines(width);
            if !hint_lines.is_empty() && !lines.is_empty() {
                lines.push(HyperlinkLine::from(""));
            }
            lines.extend(hint_lines);
        }
        (!lines.is_empty()).then_some(lines)
    }

    #[cfg(test)]
    pub(crate) fn active_cell_transcript_lines(&self, width: u16) -> Option<Vec<Line<'static>>> {
        self.active_cell_transcript_hyperlink_lines(width)
            .map(crate::tui_core::terminal_hyperlinks::visible_lines)
    }

    /// 返回组件当前配置的引用（包含通过 TUI 应用的任何
    /// 运行时覆盖，例如模型或审批策略）。
    pub(crate) fn config_ref(&self) -> &Config {
        &self.config
    }

    #[cfg(test)]
    pub(crate) fn status_line_text(&self) -> Option<String> {
        self.bottom_pane.status_line_text()
    }

    pub(crate) fn clear_token_usage(&mut self) {
        self.token_info = None;
    }
}

impl Drop for ChatWidget {
    fn drop(&mut self) {
        self.stop_rate_limit_poller();
    }
}

const PLACEHOLDERS: [&str; 8] = [
    "Explain this codebase",
    "Summarize recent commits",
    "Implement {feature}",
    "Find and fix a bug in @filename",
    "Write tests for @filename",
    "Improve documentation in @filename",
    "Run /review on my current changes",
    "Use /skills to list available skills",
];

const SIDE_PLACEHOLDERS: [&str; 3] = [
    "Check recently modified functions for compatibility",
    "How many files have been modified?",
    "Will this algorithm scale well?",
];

// tests.rs 尚未创建；当它落地时，门控（gate）已就绪。
// #[cfg(all(test, feature = "tui-upstream-tests"))]
// pub(crate) mod tests;
