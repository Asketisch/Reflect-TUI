//! 用于协调 UI 操作的应用级事件。
//!
//! `AppEvent` 是 UI 组件与顶层 `App` 循环之间的内部消息总线。
//! widget 发出事件以请求必须在应用层处理的动作（例如打开
//! 选择器、持久化配置或关闭 agent），而无需直接访问
//! `App` 内部。
//!
//! 退出通过 `AppEvent::Exit(ExitMode)` 显式建模，使调用方可以请求先关机的
//! 退出，而无需深入应用循环或耦合到关机/退出时序。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::app_server_protocol::AddCreditsNudgeCreditType;
use crate::app_server_protocol::AddCreditsNudgeEmailStatus;
use crate::app_server_protocol::ConsumeAccountRateLimitResetCreditResponse;
use crate::app_server_protocol::GetAccountRateLimitsResponse;
use crate::app_server_protocol::GetAccountTokenUsageResponse;
use crate::app_server_protocol::MarketplaceAddResponse;
use crate::app_server_protocol::MarketplaceRemoveResponse;
use crate::app_server_protocol::MarketplaceUpgradeResponse;
use crate::app_server_protocol::McpServerStatus;
use crate::app_server_protocol::McpServerStatusDetail;
use crate::app_server_protocol::PluginInstallResponse;
use crate::app_server_protocol::PluginListResponse;
use crate::app_server_protocol::PluginMarketplaceEntry;
use crate::app_server_protocol::PluginReadParams;
use crate::app_server_protocol::PluginReadResponse;
use crate::app_server_protocol::PluginUninstallResponse;
use crate::app_server_protocol::SkillsListResponse;
use crate::app_server_protocol::ThreadGoalStatus;
use crate::connectors::AppInfo;
use crate::file_search::FileMatch;
use crate::message_history::HistoryBatchCursor;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::openai_models::ModelPreset;
use crate::tui_core::inline_visualization::InlineVisualizationContext;
use crate::utils_absolute_path::AbsolutePathBuf;
use crate::utils_approval_presets::ApprovalPreset;

use crate::app_server_protocol::AskForApproval;
use crate::config_compat::types::ApprovalsReviewer;
use crate::features::Feature;
use crate::plugin::PluginCapabilitySummary;
use crate::protocol_compat::config_types::CollaborationModeMask;
use crate::protocol_compat::config_types::Personality;
use crate::protocol_compat::models::ActivePermissionProfile;
use crate::protocol_compat::openai_models::ReasoningEffort;
use crate::tui_core::app_command::AppCommand;
use crate::tui_core::app_server_session::AppServerStartedThread;
use crate::tui_core::bottom_pane::ApprovalRequest;
use crate::tui_core::bottom_pane::StatusLineItem;
use crate::tui_core::bottom_pane::TerminalTitleItem;
use crate::tui_core::chatwidget::UserMessage;
use crate::tui_core::goal_files::GoalDraft;

use crate::tui_core::history_cell::HistoryCell;

// ── 辅助类型（外移子模块） ──
pub(crate) mod types;
pub(crate) use types::*;

// ── AppEvent 核心枚举（外移子模块） ──
pub(crate) mod event;
pub(crate) use event::*;

/// 在任何必需的 UI 护栏完成后应用的具名配置文件选择。
#[derive(Debug, Clone)]
pub(crate) struct PermissionProfileSelection {
    pub profile_id: String,
    pub approval_policy: Option<AskForApproval>,
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    pub display_label: String,
}

/// UI 层请求的退出策略。
///
/// 大多数用户发起的退出应使用 `ShutdownFirst`，使核心清理得以运行，
/// 且 UI 仅在核心确认完成后退出。`Immediate` 是逃生舱，
/// 用于关机已完成（或被绕过）且 UI 循环应立即终止的情形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitMode {
    /// 关闭核心并在完成后退出。
    ShutdownFirst,
    /// 立即退出 UI 循环，不等待关机。
    ///
    /// 这会跳过 `Op::Shutdown`，因此任何进行中的工作都可能被丢弃，
    /// 且通常在 `ShutdownComplete` 之前运行的清理可能被错过。
    Immediate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeedbackCategory {
    BadResult,
    GoodResult,
    Bug,
    SafetyCheck,
    Other,
}
