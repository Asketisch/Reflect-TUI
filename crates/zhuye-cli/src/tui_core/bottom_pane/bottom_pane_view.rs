use crate::app_server_protocol::ToolRequestUserInputParams;
use crate::tui_core::app::app_server_requests::ResolvedAppServerRequest;
use crate::tui_core::bottom_pane::ApprovalRequest;
use crate::tui_core::bottom_pane::McpServerElicitationFormRequest;
use crate::tui_core::render::renderable::Renderable;
use crossterm::event::KeyEvent;
use std::time::Instant;

use super::CancellationEvent;

/// 活动的底部面板视图结束的原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewCompletion {
    Accepted,
    Cancelled,
}

/// 每个可显示在底部面板中的视图都要实现的 trait。
pub(crate) trait BottomPaneView: Renderable {
    /// 在视图处于活动状态时处理按键事件。此调用之后
    /// 总会调度一次重绘。
    fn handle_key_event(&mut self, _key_event: KeyEvent) {}

    /// 若视图已结束且应被移除，则返回 `true`。
    fn is_complete(&self) -> bool {
        false
    }

    /// 视图结束后返回结束原因。
    fn completion(&self) -> Option<ViewCompletion> {
        None
    }

    /// 当子视图被接受后应移除此视图时返回 true。
    fn dismiss_after_child_accept(&self) -> bool {
        false
    }

    /// 在子视图被取消后清除任何待处理的子流程清理标记。
    fn clear_dismiss_after_child_accept(&mut self) {}

    /// 为打开期间需要外部刷新的视图提供的稳定标识符。
    fn view_id(&self) -> Option<&'static str> {
        None
    }

    /// 基于列表的视图的实际条目索引，用于在外部刷新时
    /// 保持选中项。
    fn selected_index(&self) -> Option<usize> {
        None
    }

    /// 带标签页的列表视图的当前活动标签页 id。
    #[allow(dead_code)]
    fn active_tab_id(&self) -> Option<&str> {
        None
    }

    /// 在此视图处于活动状态时处理 Ctrl-C。
    fn on_ctrl_c(&mut self) -> CancellationEvent {
        CancellationEvent::NotHandled
    }

    /// 若 Esc 应经由 `handle_key_event` 处理，而非走
    /// `on_ctrl_c` 取消路径，则返回 true。
    fn prefer_esc_to_handle_key_event(&self) -> bool {
        false
    }

    /// 当此按键事件会中断当前 agent 回合时返回 true。
    fn will_interrupt_turn_on_key_event(&self, _key_event: KeyEvent) -> bool {
        false
    }

    /// 可选的粘贴处理器。若视图修改了自身状态并
    /// 需要重绘，则返回 true。
    fn handle_paste(&mut self, _pasted: String) -> bool {
        false
    }

    /// 刷新任何待处理的粘贴突发状态。若状态发生变化则返回 true。
    ///
    /// 这让复用 `ChatComposer` 的模态视图能够参与与主编写器
    /// 相同的基于时间的粘贴突发刷新。
    fn flush_paste_burst_if_due(&mut self) -> bool {
        false
    }

    /// 视图当前是否持有粘贴突发瞬态状态。
    ///
    /// 当为 `true` 时，底部面板将调度一次短暂的延迟重绘，
    /// 给突发时间窗口留出刷新的机会。
    fn is_in_paste_burst(&self) -> bool {
        false
    }

    /// 在渲染前立即处理基于时间的状态。
    ///
    /// 当状态发生变化且底部面板应重绘或结束
    /// 活动视图时返回 true。
    fn pre_draw_tick(&mut self, _now: Instant) -> bool {
        false
    }

    /// 尝试处理审批请求；若未被消费则返回原始值。
    fn try_consume_approval_request(
        &mut self,
        request: ApprovalRequest,
    ) -> Option<ApprovalRequest> {
        Some(request)
    }

    /// 尝试处理 request_user_input；若未被消费则返回原始值。
    fn try_consume_user_input_request(
        &mut self,
        request: ToolRequestUserInputParams,
    ) -> Option<ToolRequestUserInputParams> {
        Some(request)
    }

    /// 尝试处理受支持的 MCP 服务器表单征询请求；若未被
    /// 消费则返回原始值。
    fn try_consume_mcp_server_elicitation_request(
        &mut self,
        request: McpServerElicitationFormRequest,
    ) -> Option<McpServerElicitationFormRequest> {
        Some(request)
    }

    /// 关闭已由其他客户端解决的请求。
    ///
    /// 当视图状态发生变化时返回 `true`。
    fn dismiss_app_server_request(&mut self, _request: &ResolvedAppServerRequest) -> bool {
        false
    }

    /// 此视图是否意味着会话正阻塞等待用户操作。
    ///
    /// 返回 `true` 的视图会显示 "Action Required" 终端标题，
    /// 而非通常的工作中转圈动画，使终端标签页清晰表明
    /// Reflect 需要用户输入。
    fn terminal_title_requires_action(&self) -> bool {
        false
    }

    /// 返回此视图在活动期间所需的下一次基于时间的重绘。
    fn next_frame_delay(&self) -> Option<std::time::Duration> {
        None
    }
}
