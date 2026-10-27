//! 聊天 widget 的 MCP 启动状态及状态处理。
//!
//! 应用服务器以“每个服务器”的状态更新形式上报 MCP 服务器的启动过程。
//! 本模块保持 TUI 缓冲的启动轮次状态一致，并将这些更新转化为
//! 状态标题、警告以及排队输入的释放节点。

use std::collections::BTreeSet;

use crate::app_server_protocol::McpServerStartupState;
use crate::app_server_protocol::McpServerStatusUpdatedNotification;

use super::ChatWidget;

const MCP_STARTUP_SINGLE_HEADER_PREFIX: &str = "Booting MCP server:";
const MCP_STARTUP_MULTI_HEADER_PREFIX: &str = "Starting MCP servers";

#[derive(Debug, Clone)]
pub(crate) enum McpStartupStatus {
    Starting,
    Ready,
    Failed { error: String },
    Cancelled,
}

impl ChatWidget {
    /// 记录一次 MCP 启动更新，并将其提升为活跃的启动轮次或缓冲的“下一轮”。
    ///
    /// 这条路径必须处理应用服务器“有损”的事件传递。在 `finish_mcp_startup()`
    /// 或 `finish_mcp_startup_after_lag()` 之后，我们会短暂忽略传入的更新，
    /// 以避免刚刚结束的轮次中陈旧的事件重新打开启动流程。在该守卫激活期间，
    /// 我们把更新缓冲到可能的下一轮中，只有当缓冲集合足够一致、可以视为
    /// 一次全新的启动轮次时，才会重新激活。
    fn update_mcp_startup_status(
        &mut self,
        server: String,
        status: McpStartupStatus,
        complete_when_settled: bool,
    ) {
        let mut activated_pending_round = false;
        let startup_status = if self.mcp_startup_ignore_updates_until_next_start {
            // “忽略模式”会缓冲下一次可能的轮次，以避免结束后的陈旧更新立即重新触发启动。
            // 只有在我们尚未看到待处理轮次的 `Starting` 时，新的 `Starting` 更新
            // 才会重置缓冲区；这样可以保留像
            // `alpha: Starting -> alpha: Ready -> beta: Starting` 这种合法的交叉顺序。
            if matches!(status, McpStartupStatus::Starting)
                && !self.mcp_startup_pending_next_round_saw_starting
            {
                self.mcp_startup_pending_next_round.clear();
                self.mcp_startup_allow_terminal_only_next_round = false;
            }
            self.mcp_startup_pending_next_round_saw_starting |=
                matches!(status, McpStartupStatus::Starting);
            self.mcp_startup_pending_next_round.insert(server, status);
            let Some(expected_servers) = &self.mcp_startup_expected_servers else {
                return;
            };
            let saw_full_round = expected_servers.is_empty()
                || expected_servers
                    .iter()
                    .all(|name| self.mcp_startup_pending_next_round.contains_key(name));
            let saw_starting = self
                .mcp_startup_pending_next_round
                .values()
                .any(|state| matches!(state, McpStartupStatus::Starting));
            if !(saw_full_round
                && (saw_starting || self.mcp_startup_allow_terminal_only_next_round))
            {
                return;
            }

            // 缓冲的映射现在看起来像一个完整的下一轮，因此将其提升为活跃轮次，
            // 并恢复正常的完成跟踪。
            self.mcp_startup_ignore_updates_until_next_start = false;
            self.mcp_startup_allow_terminal_only_next_round = false;
            self.mcp_startup_pending_next_round_saw_starting = false;
            activated_pending_round = true;
            std::mem::take(&mut self.mcp_startup_pending_next_round)
        } else {
            // 正常路径：将更新合并到活跃轮次中，并立即呈现每个服务器的失败。
            let mut startup_status = self.mcp_startup_status.take().unwrap_or_default();
            if let McpStartupStatus::Failed { error } = &status {
                let already_reported = matches!(
                    startup_status.get(&server),
                    Some(McpStartupStatus::Failed { error: previous }) if previous == error
                );
                if !already_reported {
                    self.on_warning(error);
                }
            }
            startup_status.insert(server, status);
            startup_status
        };
        if activated_pending_round {
            // 被提升上来的缓冲轮次可能已经包含终态失败。
            for state in startup_status.values() {
                if let McpStartupStatus::Failed { error } = state {
                    self.on_warning(error);
                }
            }
        }
        self.mcp_startup_status = Some(startup_status);
        self.update_task_running_state();

        // 由应用服务器支撑的启动流程在每个预期的服务器都报告了非 Starting 的状态时完成。
        // 滞后处理可以通过 `finish_mcp_startup_after_lag()` 强制更早地收尾。
        if complete_when_settled
            && let Some(current) = &self.mcp_startup_status
            && let Some(expected_servers) = &self.mcp_startup_expected_servers
            && !current.is_empty()
            && expected_servers
                .iter()
                .all(|name| current.contains_key(name))
            && current
                .values()
                .all(|state| !matches!(state, McpStartupStatus::Starting))
        {
            let mut failed = Vec::new();
            let mut cancelled = Vec::new();
            for (name, state) in current {
                match state {
                    McpStartupStatus::Ready => {}
                    McpStartupStatus::Failed { .. } => failed.push(name.clone()),
                    McpStartupStatus::Cancelled => cancelled.push(name.clone()),
                    McpStartupStatus::Starting => {}
                }
            }
            failed.sort();
            cancelled.sort();
            self.finish_mcp_startup(failed, cancelled);
            return;
        }
        if let Some(header) = self.mcp_startup_status_header() {
            self.set_status_header(header);
        }
        self.request_redraw();
    }

    pub(super) fn mcp_startup_status_header(&self) -> Option<String> {
        let current = self.mcp_startup_status.as_ref()?;
        let total = current.len();
        let mut starting: Vec<_> = current
            .iter()
            .filter_map(|(name, state)| {
                if matches!(state, McpStartupStatus::Starting) {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        starting.sort();
        let first = starting.first()?;
        let completed = total.saturating_sub(starting.len());
        let max_to_show = 3;
        let mut to_show: Vec<String> = starting
            .iter()
            .take(max_to_show)
            .map(ToString::to_string)
            .collect();
        if starting.len() > max_to_show {
            to_show.push("…".to_string());
        }
        Some(if total > 1 {
            format!(
                "{MCP_STARTUP_MULTI_HEADER_PREFIX} ({completed}/{total}): {}",
                to_show.join(", ")
            )
        } else {
            format!("{MCP_STARTUP_SINGLE_HEADER_PREFIX} {first}")
        })
    }

    pub(crate) fn set_mcp_startup_expected_servers<I>(&mut self, server_names: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.mcp_startup_expected_servers = Some(server_names.into_iter().collect());
    }

    pub(super) fn finish_mcp_startup(&mut self, failed: Vec<String>, cancelled: Vec<String>) {
        if !cancelled.is_empty() {
            self.on_warning(format!(
                "MCP startup interrupted. The following servers were not initialized: {}",
                cancelled.join(", ")
            ));
        }
        let mut parts = Vec::new();
        if !failed.is_empty() {
            parts.push(format!("failed: {}", failed.join(", ")));
        }
        if !parts.is_empty() {
            self.on_warning(format!("MCP startup incomplete ({})", parts.join("; ")));
        }

        let mcp_startup_owned_status = self.status_header_is_mcp_startup_owned();
        self.mcp_startup_status = None;
        self.mcp_startup_ignore_updates_until_next_start = true;
        self.mcp_startup_allow_terminal_only_next_round = false;
        self.mcp_startup_pending_next_round.clear();
        self.mcp_startup_pending_next_round_saw_starting = false;
        self.update_task_running_state();
        if self.bottom_pane.is_task_running() && mcp_startup_owned_status {
            self.restore_reasoning_status_header();
        }
        self.maybe_send_next_queued_input();
        self.request_redraw();
    }

    pub(crate) fn finish_mcp_startup_after_lag(&mut self) {
        if self.mcp_startup_ignore_updates_until_next_start {
            if self.mcp_startup_pending_next_round.is_empty() {
                self.mcp_startup_pending_next_round_saw_starting = false;
            }
            self.mcp_startup_allow_terminal_only_next_round = true;
        }

        let Some(current) = &self.mcp_startup_status else {
            return;
        };

        let mut failed = Vec::new();
        let mut cancelled = Vec::new();

        let mut server_names: BTreeSet<String> = current.keys().cloned().collect();
        if let Some(expected_servers) = &self.mcp_startup_expected_servers {
            server_names.extend(expected_servers.iter().cloned());
        }

        for name in server_names {
            match current.get(&name) {
                Some(McpStartupStatus::Ready) => {}
                Some(McpStartupStatus::Failed { .. }) => failed.push(name),
                Some(McpStartupStatus::Cancelled | McpStartupStatus::Starting) | None => {
                    cancelled.push(name);
                }
            }
        }

        failed.sort();
        failed.dedup();
        cancelled.sort();
        cancelled.dedup();
        self.finish_mcp_startup(failed, cancelled);
    }

    pub(super) fn status_header_is_mcp_startup_owned(&self) -> bool {
        self.status_state
            .current_status
            .header
            .starts_with(MCP_STARTUP_SINGLE_HEADER_PREFIX)
            || self
                .status_state
                .current_status
                .header
                .starts_with(MCP_STARTUP_MULTI_HEADER_PREFIX)
    }

    pub(super) fn on_mcp_server_status_updated(
        &mut self,
        notification: McpServerStatusUpdatedNotification,
    ) {
        let status = match notification.status {
            McpServerStartupState::Starting => McpStartupStatus::Starting,
            McpServerStartupState::Ready => McpStartupStatus::Ready,
            McpServerStartupState::Failed => McpStartupStatus::Failed {
                error: notification.error.unwrap_or_else(|| {
                    format!("MCP client for `{}` failed to start", notification.name)
                }),
            },
            McpServerStartupState::Cancelled => McpStartupStatus::Cancelled,
        };
        self.update_mcp_startup_status(
            notification.name,
            status,
            /*complete_when_settled*/ true,
        );
    }
}
