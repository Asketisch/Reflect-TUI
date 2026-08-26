//! 在聊天组件中协调异步的 `/usage` 卡片。
//!
//! 斜杠命令会立即构建一个复合历史单元格，但在账户请求运行期间，组件会让
//! 该单元格保持暂态。暂态卡片通过 [`ChatWidget::pending_token_activity_output`]
//! 渲染在输入框上方，因此加载过程无需清空或重写会话历史。当匹配的响应到达时，
//! [`TokenActivityHandle`] 会更新共享的卡片状态，而
//! [`ChatWidget::finish_token_activity_refresh`] 会将该单元格移入已完成槽位。
//! 事件分发只会在活动输出与流合并不再阻塞插入之后，才把已完成的单元格提交
//! 到历史中。
//!
//! 纯图表渲染与日期分桶位于 [`chart`] 中。本模块负责请求关联、暂态/已完成
//! 卡片状态，以及与 `ChatWidget` 历史插入的集成。

mod chart;

use std::sync::Arc;
use std::sync::RwLock;

use crate::app_server_protocol::GetAccountTokenUsageResponse;
use chrono::NaiveDate;
use chrono::Utc;
use ratatui::style::Stylize;
use ratatui::text::Line;

use super::ChatWidget;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::history_cell::CompositeHistoryCell;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::history_cell::PlainHistoryCell;
use crate::tui_core::history_cell::plain_lines;

pub(crate) use chart::TokenActivityView;

/// 跟踪单个令牌活动历史单元格的可渲染生命周期。
#[derive(Debug)]
enum TokenActivityState {
    Loading,
    Loaded {
        response: GetAccountTokenUsageResponse,
        today: NaiveDate,
    },
    Error,
}

/// 完成一个异步渲染的令牌活动历史单元格。
///
/// 克隆体共享同一份卡片状态，使后台请求路径能够更新仍由组件暂态输出状态
/// 持有的单元格。组件仍负责请求 ID 匹配、重绘以及历史插入。
#[derive(Clone, Debug)]
pub(super) struct TokenActivityHandle {
    state: Arc<RwLock<TokenActivityState>>,
}

/// 持有等待后台响应的那一张暂态令牌活动卡片。
///
/// 请求 ID 可防止迟到的结果修改更新的 `/usage` 卡片。在匹配的响应完成且
/// 组件确认活动输出不再阻塞插入之前，该单元格不会进入会话历史。
pub(super) struct PendingTokenActivityOutput {
    request_id: u64,
    cell: CompositeHistoryCell,
    handle: TokenActivityHandle,
}

impl TokenActivityHandle {
    /// 用已获取的活动数据或不可用状态替换加载中状态。
    ///
    /// 本方法有意丢弃错误字符串，因为 TUI 只展示一条稳定的不可用消息。
    /// 多次调用会覆盖先前的终态，因此应在完成之前先做请求 ID 匹配。
    pub(super) fn finish(&self, result: Result<GetAccountTokenUsageResponse, String>) {
        self.finish_with_today(result, Utc::now().date_naive());
    }

    fn finish_with_today(
        &self,
        result: Result<GetAccountTokenUsageResponse, String>,
        today: NaiveDate,
    ) {
        let state = match result {
            Ok(response) => TokenActivityState::Loaded { response, today },
            Err(_) => TokenActivityState::Error,
        };
        #[expect(clippy::expect_used)]
        let mut current = self.state.write().expect("token activity state poisoned");
        *current = state;
    }
}

/// 根据共享的异步状态渲染一张 `/usage` 卡片。
#[derive(Debug)]
struct TokenActivityHistoryCell {
    view: TokenActivityView,
    state: Arc<RwLock<TokenActivityState>>,
}

/// 为一次 `/usage` 调用创建卡片内容和完成句柄。
///
/// 复合单元格从一开始就包含回显的斜杠命令和一张加载中的卡片。调用方必须
/// 保留返回的句柄，并在匹配的后台响应到达时完成它；否则暂态卡片会一直
/// 停留在加载中状态。
pub(super) fn new_token_activity_output(
    view: TokenActivityView,
) -> (CompositeHistoryCell, TokenActivityHandle) {
    let command = PlainHistoryCell::new(vec![
        format!("/usage {}", view.label().to_lowercase())
            .magenta()
            .into(),
    ]);
    let state = Arc::new(RwLock::new(TokenActivityState::Loading));
    let handle = TokenActivityHandle {
        state: Arc::clone(&state),
    };
    let card = TokenActivityHistoryCell { view, state };
    (
        CompositeHistoryCell::new(vec![Box::new(command), Box::new(card)]),
        handle,
    )
}

impl HistoryCell for TokenActivityHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        #[expect(clippy::expect_used)]
        let state = self.state.read().expect("token activity state poisoned");
        match &*state {
            TokenActivityState::Loading => {
                vec![
                    " Token activity".bold().into(),
                    "   Loading...".dim().into(),
                ]
            }
            TokenActivityState::Error => vec![
                " Token activity".bold().into(),
                "   Token activity unavailable".dim().into(),
            ],
            TokenActivityState::Loaded { response, today } => {
                chart::loaded_lines(self.view, response, *today, width)
            }
        }
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.display_lines(u16::MAX))
    }
}

impl ChatWidget {
    /// 启动一次令牌活动刷新，并替换当前的暂态卡片。
    ///
    /// 每次调用都会获得一个请求 ID，使后台响应只更新各自的卡片。卡片在完成
    /// 之前不会进入会话历史，这样既保持加载状态可见，又不打扰现有的会话内容。
    pub(crate) fn add_token_activity_output(&mut self, view: TokenActivityView) {
        let request_id = self.next_token_activity_request_id;
        self.next_token_activity_request_id =
            self.next_token_activity_request_id.wrapping_add(/*rhs*/ 1);
        let (cell, handle) = new_token_activity_output(view);
        self.completed_token_activity_output = None;
        self.refreshing_token_activity_output = Some(PendingTokenActivityOutput {
            request_id,
            cell,
            handle,
        });
        self.bump_active_cell_revision();
        self.request_redraw();
        self.app_event_tx
            .send(AppEvent::RefreshTokenActivity { request_id });
    }

    /// 返回应渲染在输入框上方的暂态令牌活动卡片。
    ///
    /// 加载中的卡片优先于等待插入历史的已完成卡片。调用方应渲染返回的单元格，
    /// 但把所有权留给组件，以便完成和插入操作能安全地更新它。
    pub(super) fn pending_token_activity_output(&self) -> Option<&dyn HistoryCell> {
        self.refreshing_token_activity_output
            .as_ref()
            .map(|output| &output.cell as &dyn HistoryCell)
            .or_else(|| {
                self.completed_token_activity_output
                    .as_ref()
                    .map(|cell| cell as &dyn HistoryCell)
            })
    }

    /// 将后台令牌活动结果应用到与之匹配的暂态卡片。
    ///
    /// 当挂起的请求匹配成功并移入已完成槽位时返回 `true`。迟到的响应返回
    /// `false`，包括被更新的 `/usage` 调用所替换的卡片，以及在会话变更期间
    /// 被清除的卡片的响应。
    pub(crate) fn finish_token_activity_refresh(
        &mut self,
        request_id: u64,
        result: Result<GetAccountTokenUsageResponse, String>,
    ) -> bool {
        let Some(output) = self.refreshing_token_activity_output.take() else {
            return false;
        };
        if output.request_id != request_id {
            self.refreshing_token_activity_output = Some(output);
            return false;
        }
        output.handle.finish(result);
        self.completed_token_activity_output = Some(output.cell);
        self.bump_active_cell_revision();
        self.request_redraw();
        true
    }

    /// 报告已完成的异步用量输出在插入前是否必须等待。
    ///
    /// 在存在流、排队中的合并或活动会话单元格时插入，可能导致输出相对于
    /// 可见的工作发生乱序，因此调用方应在这些屏障清除后重试。
    pub(crate) fn usage_history_insertion_blocked(&self) -> bool {
        self.stream_controller.is_some()
            || self.plan_stream_controller.is_some()
            || self.pending_stream_consolidations > 0
            || self.transcript.active_cell.is_some()
            || self.active_hook_cell.is_some()
    }

    /// 记录一个会延迟令牌卡片插入的流合并屏障。
    ///
    /// 每个排队的合并最终都应调用
    /// [`ChatWidget::note_stream_consolidation_completed`]。
    pub(crate) fn note_stream_consolidation_queued(&mut self) {
        self.pending_stream_consolidations =
            self.pending_stream_consolidations.saturating_add(/*rhs*/ 1);
    }

    /// 释放一个排队的流合并屏障。
    ///
    /// 计数器在零处饱和，因此未匹配的完成操作不会导致下溢，
    /// 但成对的入队/完成调用仍是预期的契约。
    pub(crate) fn note_stream_consolidation_completed(&mut self) {
        self.pending_stream_consolidations =
            self.pending_stream_consolidations.saturating_sub(/*rhs*/ 1);
    }

    /// 将已完成的令牌活动卡片转移到历史插入路径。
    ///
    /// 调用方应只在 [`ChatWidget::usage_history_insertion_blocked`]
    /// 返回 `false` 之后使用本方法；取走卡片会将其从暂态渲染区域移除。
    pub(crate) fn take_completed_token_activity_output(&mut self) -> Option<CompositeHistoryCell> {
        let output = self.completed_token_activity_output.take()?;
        self.bump_active_cell_revision();
        Some(output)
    }

    /// 当已完成的用量输出仍在等待时，请求再次尝试插入。
    ///
    /// 本方法用于流或历史生命周期事件之后——这些事件可能已经清除了插入
    /// 屏障，但并不直接持有已完成的输出。
    pub(crate) fn request_pending_usage_output_insertion(&self) {
        if self.completed_token_activity_output.is_some()
            || self.pending_rate_limit_reset_hint().is_some()
        {
            self.app_event_tx.send(AppEvent::CommitPendingUsageOutput);
        }
    }

    pub(crate) fn request_pending_usage_output_insertion_after_stream_shutdown(&self) {
        if self.completed_token_activity_output.is_some()
            || self.pending_rate_limit_reset_hint().is_some()
        {
            self.app_event_tx
                .send(AppEvent::CommitPendingUsageOutputAfterStreamShutdown);
        }
    }

    /// 丢弃不应再更新的暂态与已完成令牌卡片。
    ///
    /// 在会话重置、回退或替换流程清除此组件持有的状态之后，
    /// 迟到的后台响应将无法再修改这些卡片。
    pub(crate) fn clear_pending_token_activity_refreshes(&mut self) {
        let cleared_refresh = self.refreshing_token_activity_output.take().is_some();
        let cleared_completed = self.completed_token_activity_output.take().is_some();
        if cleared_refresh || cleared_completed {
            self.bump_active_cell_revision();
            self.request_redraw();
        }
    }
}

#[cfg(test)]
#[path = "tokens_tests.rs"]
mod tests;
