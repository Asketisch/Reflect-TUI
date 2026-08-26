//! AppEvent 辅助类型：线程目标、历史查找、连接器快照、插件位置等。从 app_event.rs 抽出。

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadGoalSetMode {
    ConfirmIfExists,
    ReplaceExisting,
    UpdateExisting {
        status: ThreadGoalStatus,
        token_budget: Option<i64>,
    },
}

/// 批量查找返回的一个绝对历史偏移。
///
/// 格式错误的行保留其偏移，`entry` 设为 `None`，使 composer 能缓存缺口
/// 而不会让每条更旧的记录发生位移。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistoryBatchEntryResponse {
    pub(crate) offset: usize,
    pub(crate) entry: Option<String>,
}

/// 路由回发起线程的持久历史数据。
///
/// 批量响应保留绝对偏移和格式错误行的缺口，使 composer 能
/// 独立于响应到达时激活的搜索查询来缓存数据。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HistoryLookupResponse {
    Entry {
        offset: usize,
        log_id: u64,
        entry: Option<String>,
    },
    Batch {
        cursor: HistoryBatchCursor,
        log_id: u64,
        entries: Vec<HistoryBatchEntryResponse>,
        next_older_cursor: Option<HistoryBatchCursor>,
    },
    BatchError {
        cursor: HistoryBatchCursor,
        log_id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsolidationScrollbackReflow {
    IfResizeReflowRan,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) enum WindowsSandboxEnableMode {
    Elevated,
    Legacy,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct ConnectorsSnapshot {
    pub(crate) connectors: Vec<AppInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginLocation {
    Local { marketplace_path: AbsolutePathBuf },
    Remote { marketplace_name: String },
}

impl PluginLocation {
    pub(crate) fn into_request_params(self) -> (Option<AbsolutePathBuf>, Option<String>) {
        match self {
            PluginLocation::Local { marketplace_path } => (Some(marketplace_path), None),
            PluginLocation::Remote { marketplace_name } => (None, Some(marketplace_name)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginRemoteSectionError {
    pub(crate) section_id: String,
    pub(crate) label: String,
    pub(crate) message: String,
}

/// 区分请求速率限制刷新的原因，使完成
/// 处理器能正确路由结果。
///
/// `StartupPrefetch` 在引导后与其余 TUI 初始化并发触发一次，
/// 更新缓存的快照和任何可用的重置额度提示（无需结束状态卡）。
/// `StatusCommand` 绑定到某次具体的 `/status` 调用，完成后必须调用
/// `finish_status_rate_limit_refresh`，使卡片停止显示“刷新中”状态。
/// `UsageMenu` 刷新缓存的零重置计数，使被禁用的菜单项无需重启即可用。
/// `ResetPicker` 在展示兑换选择前刷新速率限制和详细的重置额度行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RateLimitRefreshOrigin {
    /// 引导后急切获取，用于 `/status` 数据和重置可用性。
    StartupPrefetch { reset_hint_request_id: u64 },
    /// 用户通过 `/status` 发起；`request_id` 与获取完成时
    /// 应更新的状态卡相关联。
    StatusCommand { request_id: u64 },
    /// 缓存的重置额度计数为零时，用户重新打开了 `/usage`。
    UsageMenu { request_id: u64 },
    /// 用户打开了重置额度选择器。
    ResetPicker { request_id: u64 },
    /// 重置额度成功消耗后请求的刷新。
    ResetConsume { request_id: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeymapEditIntent {
    ReplaceAll,
    AddAlternate,
    ReplaceOne { old_key: String },
}
