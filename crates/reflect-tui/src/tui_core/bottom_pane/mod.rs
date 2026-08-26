//! BottomPane 主模块（核心状态机）。
//!
//! 注：本文件超过 800 行红线，但 `impl BottomPane`（约 1550 行，142 个方法）为内聚的完整
//! 状态机，与上游逐行对应，按 AGENTS.md「内聚完整状态机例外」保留。

//! 底栏（bottom pane）是聊天 UI 的交互式页脚。
//!
//! 该面板拥有 [`ChatComposer`]（可编辑的提示词输入框）以及一组临时的
//! [`BottomPaneView`]（弹窗/模态框），后者会临时替换输入框，用于
//! 诸如选择列表之类的聚焦交互。
//!
//! 输入路由是分层的：`BottomPane` 决定哪个本地界面接收按键（视图 vs 输入框），
//! 而诸如“中断”或“退出”之类的高层意图则由父级组件（`ChatWidget`）决定。
//! 这一分工对 Ctrl+C/Ctrl+D 很重要：底栏先让活动视图有机会消费 Ctrl+C
//! （通常是让其自行关闭），然后让活动输入框的历史搜索把 Ctrl+C 当作取消来处理，
//! 而 `ChatWidget` 可以把未处理的 Ctrl+C 视为中断，或视为双击退出快捷键的第一次按下。
//!
//! 有些 UI 是基于时间而非输入的，例如短暂的“再次按下即可退出”提示。
//! 面板会调度重绘，以便即使 UI 处于空闲状态，这些提示也能按时过期。
use std::collections::VecDeque;
use std::path::PathBuf;

use crate::app_server_protocol::SkillMetadata;
use crate::app_server_protocol::ToolRequestUserInputParams;
use crate::features::Features;
use crate::file_search::FileMatch;
use crate::plugin::PluginCapabilitySummary;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::openai_models::ReasoningEffort;
use crate::protocol_compat::user_input::TextElement;
use crate::tui_core::app::app_server_requests::ResolvedAppServerRequest;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event::ConnectorsSnapshot;
use crate::tui_core::app_event::HistoryLookupResponse;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::bottom_pane::pending_input_preview::PendingInputPreview;
use crate::tui_core::bottom_pane::pending_thread_approvals::PendingThreadApprovals;
use crate::tui_core::bottom_pane::unified_exec_footer::UnifiedExecFooter;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::keymap::RuntimeKeymap;
use crate::tui_core::keymap::primary_binding;
use crate::tui_core::render::renderable::FlexRenderable;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::render::renderable::RenderableItem;
use crate::tui_core::terminal_palette::effective_stdout_color_level;
use crate::tui_core::tui::FrameRequester;
pub(crate) use bottom_pane_view::BottomPaneView;
pub(crate) use bottom_pane_view::ViewCompletion;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use std::time::Duration;
use std::time::Instant;

mod action_required_title;
mod app_link_view;
mod approval_overlay;
mod mcp_server_elicitation;
mod multi_select_picker;
mod request_user_input;
mod status_line_setup;
mod status_line_style;
mod status_surface_preview;
mod title_setup;
pub(crate) use action_required_title::ACTION_REQUIRED_PREVIEW_PREFIX;
pub(crate) use action_required_title::build_action_required_title_text;
pub(crate) use app_link_view::AppLinkElicitationTarget;
pub(crate) use app_link_view::AppLinkSuggestionType;
pub(crate) use app_link_view::AppLinkView;
pub(crate) use app_link_view::AppLinkViewParams;
pub(crate) use approval_overlay::ApplyPatchApprovalRequest;
pub(crate) use approval_overlay::ApprovalOverlay;
pub(crate) use approval_overlay::ApprovalRequest;
pub(crate) use approval_overlay::ExecApprovalRequest;
pub(crate) use approval_overlay::McpElicitationApprovalRequest;
pub(crate) use approval_overlay::PermissionsApprovalRequest;
pub(crate) use mcp_server_elicitation::McpServerElicitationFormRequest;
pub(crate) use mcp_server_elicitation::McpServerElicitationOverlay;
pub(crate) use request_user_input::RequestUserInputOverlay;
pub(crate) use status_line_style::status_line_from_segments;
mod bottom_pane_view;
mod effort_ignition;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalImageAttachment {
    pub(crate) placeholder: String,
    pub(crate) path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MentionBinding {
    /// 可见的提及符号（`$` 或 `@`）。
    pub(crate) sigil: char,
    /// 不带前导符号（`$` 或 `@`）的提及文本。
    pub(crate) mention: String,
    /// 规范的提及目标（例如 `app://...` 或 SKILL.md 的绝对路径）。
    pub(crate) path: String,
}
mod chat_composer;
mod chat_composer_history;
mod command_popup;
pub(crate) mod custom_prompt_view;
mod effort_status_line;
mod experimental_features_view;
mod file_search_popup;
mod footer;
mod list_selection_view;
mod memories_settings_view;
mod mentions_v2;
pub(crate) mod prompt_args;
mod skill_popup;
mod skills_toggle_view;
pub(crate) mod slash_commands;
pub(crate) use footer::CollaborationModeIndicator;
pub(crate) use footer::GoalStatusIndicator;
#[cfg(test)]
pub(crate) use footer::goal_status_indicator_line;
pub(crate) use list_selection_view::ColumnWidthMode;
#[cfg(test)]
pub(crate) use list_selection_view::ListSelectionView;
pub(crate) use list_selection_view::OnSelectionChangedCallback;
pub(crate) use list_selection_view::SelectionRowDisplay;
pub(crate) use list_selection_view::SelectionToggle;
pub(crate) use list_selection_view::SelectionViewParams;
pub(crate) use list_selection_view::SideContentWidth;
pub(crate) use list_selection_view::popup_content_width;
pub(crate) use list_selection_view::side_by_side_layout_widths;
pub(crate) use memories_settings_view::MemoriesSettingsView;
use slash_commands::ServiceTierCommand;
mod feedback_view;
mod hooks_browser_view;
pub(crate) use feedback_view::feedback_disabled_params;
pub(crate) use feedback_view::feedback_selection_params;
pub(crate) use feedback_view::feedback_upload_consent_params;
pub(crate) use skills_toggle_view::SkillsToggleItem;
pub(crate) use skills_toggle_view::SkillsToggleView;
pub(crate) use status_line_setup::StatusLineItem;
pub(crate) use status_line_setup::StatusLineSetupView;
pub(crate) use status_surface_preview::StatusSurfacePreviewData;
pub(crate) use status_surface_preview::StatusSurfacePreviewItem;
pub(crate) use title_setup::TerminalTitleItem;
pub(crate) use title_setup::TerminalTitleSetupView;
#[cfg(test)]
pub(crate) use title_setup::preview_line_for_title_items;
mod paste_burst;
mod pending_input_preview;
mod pending_thread_approvals;
pub(crate) mod popup_consts;
mod scroll_state;
mod selection_popup_common;
mod selection_tabs;
mod textarea;
mod unified_exec_footer;
pub(crate) use feedback_view::FeedbackNoteView;
pub(crate) use hooks_browser_view::HooksBrowserView;
pub(crate) use selection_tabs::SelectionTab;

/// “再次按下即可退出”提示保持可见的时长。
///
/// 该值与以下两者共享：
/// - `ChatWidget`：启用双击退出快捷键。
/// - `BottomPane`/`ChatComposer`：渲染并使页脚提示过期。
///
/// 使用单一值可确保 Ctrl+C 与 Ctrl+D 行为一致。
pub(crate) const QUIT_SHORTCUT_TIMEOUT: Duration = Duration::from_secs(1);

const APPROVAL_PROMPT_TYPING_IDLE_DELAY: Duration = Duration::from_secs(1);

/// 是否要求按两次 Ctrl+C/Ctrl+D 才能退出。
///
/// 这个 UX 实验默认是开启的，但实际使用中要求按两次才能退出会显得卡顿
/// （尤其是对习惯 shell 和其他 TUI 的用户而言）。在我们重新思考更好的
/// 退出/中断设计之前，暂时将其禁用。
pub(crate) const DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED: bool = false;

/// 将取消键提供给底栏界面后的处理结果。
///
/// 这主要用于 Ctrl+C 路由：活动视图可以消费该键以关闭自身，
/// 而调用方可以决定当该键未被本地处理时应采取什么（若有）更高级别的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancellationEvent {
    Handled,
    NotHandled,
}

use crate::tui_core::bottom_pane::prompt_args::parse_slash_name;
pub(crate) use chat_composer::ChatComposer;
pub(crate) use chat_composer::ChatComposerConfig;
pub(crate) use chat_composer::InputResult;
pub(crate) use chat_composer::QueuedInputAction;
pub(crate) use chat_composer_history::HistoryEntry;

use crate::tui_core::status_indicator_widget::StatusDetailsCapitalization;
use crate::tui_core::status_indicator_widget::StatusIndicatorWidget;
pub(crate) use experimental_features_view::ExperimentalFeatureItem;
pub(crate) use experimental_features_view::ExperimentalFeaturesView;
pub(crate) use list_selection_view::SELECTION_TOGGLE_BLOCKED_PREFIX;
pub(crate) use list_selection_view::SELECTION_TOGGLE_UNAVAILABLE_PREFIX;
pub(crate) use list_selection_view::SelectionAction;
pub(crate) use list_selection_view::SelectionItem;

struct DelayedApprovalRequest {
    request: ApprovalRequest,
    features: Features,
}

/// 显示在聊天 UI 下半部分的面板。
///
/// 它是提示词输入框（`ChatComposer`）和视图栈（`BottomPaneView`）的所属容器。
/// 它执行本地输入路由并渲染基于时间的提示，而进程级决策（退出、中断、关闭）
/// 则交由 `ChatWidget` 处理。
pub(crate) struct BottomPane {
    /// 即使显示 BottomPaneView 也会保留输入框，以便视图关闭时输入状态得以保留。
    composer: ChatComposer,

    /// 用于替代输入框显示的视图栈（例如弹窗/模态框）。
    view_stack: Vec<Box<dyn BottomPaneView>>,
    delayed_approval_requests: VecDeque<DelayedApprovalRequest>,
    last_composer_activity_at: Option<Instant>,

    app_event_tx: AppEventSender,
    frame_requester: FrameRequester,
    thread_id: Option<ThreadId>,

    has_input_focus: bool,
    enhanced_keys_supported: bool,
    disable_paste_burst: bool,
    is_task_running: bool,
    esc_backtrack_hint: bool,
    animations_enabled: bool,

    /// 任务运行期间显示在输入框上方的内联状态指示器。
    status: Option<StatusIndicatorWidget>,
    /// 统一执行（unified exec）会话摘要的来源。
    ///
    /// 当存在状态行时，该摘要会以内联方式镜像到该行中；
    /// 当不存在状态行时，它会渲染为独立的页脚行。
    unified_exec_footer: UnifiedExecFooter,
    /// 显示在输入框上方的待处理 steers 与排队草稿的预览。
    pending_input_preview: PendingInputPreview,
    /// 带有待处理审批请求的非活动线程。
    pending_thread_approvals: PendingThreadApprovals,
    context_window_percent: Option<i64>,
    context_window_used_tokens: Option<i64>,
    keymap: RuntimeKeymap,
}

pub(crate) struct BottomPaneParams {
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) frame_requester: FrameRequester,
    pub(crate) has_input_focus: bool,
    pub(crate) enhanced_keys_supported: bool,
    pub(crate) placeholder_text: String,
    pub(crate) disable_paste_burst: bool,
    pub(crate) animations_enabled: bool,
    pub(crate) skills: Option<Vec<SkillMetadata>>,
}

impl BottomPane {
    pub fn new(params: BottomPaneParams) -> Self {
        let BottomPaneParams {
            app_event_tx,
            frame_requester,
            has_input_focus,
            enhanced_keys_supported,
            placeholder_text,
            disable_paste_burst,
            animations_enabled,
            skills,
        } = params;
        let mut composer = ChatComposer::new(
            has_input_focus,
            app_event_tx.clone(),
            enhanced_keys_supported,
            placeholder_text,
            disable_paste_burst,
        );
        composer.set_frame_requester(frame_requester.clone());
        let keymap = RuntimeKeymap::defaults();
        composer.set_keymap_bindings(&keymap);
        composer.set_skill_mentions(skills);
        Self {
            composer,
            view_stack: Vec::new(),
            delayed_approval_requests: VecDeque::new(),
            last_composer_activity_at: None,
            app_event_tx,
            frame_requester,
            thread_id: None,
            has_input_focus,
            enhanced_keys_supported,
            disable_paste_burst,
            is_task_running: false,
            status: None,
            unified_exec_footer: UnifiedExecFooter::new(),
            pending_input_preview: PendingInputPreview::new(),
            pending_thread_approvals: PendingThreadApprovals::new(),
            esc_backtrack_hint: false,
            animations_enabled,
            context_window_percent: None,
            context_window_used_tokens: None,
            keymap,
        }
    }

    pub fn set_skills(&mut self, skills: Option<Vec<SkillMetadata>>) {
        self.composer.set_skill_mentions(skills);
        self.request_redraw();
    }

    /// 更新活动输入框的图片粘贴行为并立即重绘。
    ///
    /// 调用方使用此方法使输入框的交互提示与模型能力保持一致。
    pub fn set_image_paste_enabled(&mut self, enabled: bool) {
        self.composer.set_image_paste_enabled(enabled);
        self.request_redraw();
    }

    /// 将生效的推理努力级别镜像到输入框中，以便其下一可见帧可以播放一次性的 Max/Ultra 特效。
    pub(crate) fn set_active_reasoning_effort(&mut self, effort: Option<&ReasoningEffort>) {
        let animations_enabled = effort_ignition::effort_animation_enabled(
            self.animations_enabled,
            effective_stdout_color_level(),
        );
        if self
            .composer
            .set_active_reasoning_effort(effort, animations_enabled)
        {
            self.request_redraw();
        }
    }

    /// 设置已恢复线程的努力级别，但不会重放其一次性动画。
    pub(crate) fn set_active_reasoning_effort_baseline(
        &mut self,
        effort: Option<&ReasoningEffort>,
    ) {
        self.composer.set_active_reasoning_effort_baseline(effort);
    }

    pub fn set_connectors_snapshot(&mut self, snapshot: Option<ConnectorsSnapshot>) {
        self.composer.set_connector_mentions(snapshot);
        self.request_redraw();
    }

    pub fn set_plugin_mentions(&mut self, plugins: Option<Vec<PluginCapabilitySummary>>) {
        self.composer.set_plugin_mentions(plugins);
        self.request_redraw();
    }

    pub fn set_plugins_command_enabled(&mut self, enabled: bool) {
        self.composer.set_plugins_command_enabled(enabled);
        self.request_redraw();
    }

    pub fn set_token_activity_command_enabled(&mut self, enabled: bool) {
        self.composer.set_token_activity_command_enabled(enabled);
        self.request_redraw();
    }

    pub fn set_mentions_v2_enabled(&mut self, enabled: bool) {
        self.composer.set_mentions_v2_enabled(enabled);
        self.request_redraw();
    }

    pub fn take_mention_bindings(&mut self) -> Vec<MentionBinding> {
        self.composer.take_mention_bindings()
    }

    pub fn take_recent_submission_mention_bindings(&mut self) -> Vec<MentionBinding> {
        self.composer.take_recent_submission_mention_bindings()
    }

    /// 将暂存的斜杠命令草稿添加到输入框的本地召回列表中。
    ///
    /// 在 `ChatWidget` 分发已识别的命令后，该方法应恰好调用一次。
    /// 无论命令是否成功，斜杠召回都会记录已提交的命令文本。
    pub(crate) fn record_pending_slash_command_history(&mut self) {
        self.composer.record_pending_slash_command_history();
    }

    /// 用一份已解析的运行时键位映射替换所有底栏键位缓存。
    ///
    /// 底栏拥有多个输入界面：输入框、覆盖层和选择视图。通过此方法应用同一份快照，
    /// 可在配置重载或交互式重映射后使这些界面保持同步。调用方不应直接更新输入框，
    /// 除非他们有意让覆盖层和选择视图继续使用旧绑定。
    pub fn set_keymap_bindings(&mut self, keymap: &RuntimeKeymap) {
        self.keymap = keymap.clone();
        self.composer.set_keymap_bindings(keymap);
        let interrupt_binding = primary_binding(&keymap.chat.interrupt_turn);
        self.pending_input_preview
            .set_interrupt_binding(interrupt_binding);
        if let Some(status) = self.status.as_mut() {
            status.set_interrupt_binding(interrupt_binding);
        }
        self.request_redraw();
    }

    /// 清除待处理的附件和提及绑定，例如当斜杠命令未提交文本时。
    pub(crate) fn drain_pending_submission_state(&mut self) {
        let _ = self.take_recent_submission_images_with_placeholders();
        let _ = self.take_remote_image_urls();
        let _ = self.take_recent_submission_mention_bindings();
        let _ = self.take_mention_bindings();
    }

    pub fn set_collaboration_modes_enabled(&mut self, enabled: bool) {
        self.composer.set_collaboration_modes_enabled(enabled);
        self.request_redraw();
    }

    pub fn set_connectors_enabled(&mut self, enabled: bool) {
        self.composer.set_connectors_enabled(enabled);
    }

    #[cfg(target_os = "windows")]
    pub fn set_windows_degraded_sandbox_active(&mut self, enabled: bool) {
        self.composer.set_windows_degraded_sandbox_active(enabled);
        self.request_redraw();
    }

    pub fn set_collaboration_mode_indicator(
        &mut self,
        indicator: Option<CollaborationModeIndicator>,
    ) {
        self.composer.set_collaboration_mode_indicator(indicator);
        self.request_redraw();
    }

    pub fn set_goal_status_indicator(&mut self, indicator: Option<GoalStatusIndicator>) {
        self.composer.set_goal_status_indicator(indicator);
        self.request_redraw();
    }

    pub fn set_ide_context_active(&mut self, active: bool) {
        self.composer.set_ide_context_active(active);
        self.request_redraw();
    }

    pub fn set_personality_command_enabled(&mut self, enabled: bool) {
        self.composer.set_personality_command_enabled(enabled);
        self.request_redraw();
    }

    pub fn set_service_tier_commands_enabled(&mut self, enabled: bool) {
        self.composer.set_service_tier_commands_enabled(enabled);
        self.request_redraw();
    }

    pub fn set_service_tier_commands(&mut self, commands: Vec<ServiceTierCommand>) {
        self.composer.set_service_tier_commands(commands);
        self.request_redraw();
    }

    pub fn set_goal_command_enabled(&mut self, enabled: bool) {
        self.composer.set_goal_command_enabled(enabled);
        self.request_redraw();
    }

    pub(crate) fn set_side_conversation_active(&mut self, active: bool) {
        self.composer.set_side_conversation_active(active);
        self.request_redraw();
    }

    pub(crate) fn set_placeholder_text(&mut self, placeholder: String) {
        self.composer.set_placeholder_text(placeholder);
        self.request_redraw();
    }

    pub(crate) fn set_parent_owned_thread(&mut self) {
        self.composer.set_parent_owned_thread();
        self.request_redraw();
    }

    /// 更新排队消息旁显示的按键提示，使其与 `ChatWidget` 实际监听的绑定一致。
    pub(crate) fn set_queued_message_edit_binding(&mut self, binding: Option<KeyBinding>) {
        self.pending_input_preview.set_edit_binding(binding);
        self.request_redraw();
    }

    pub(crate) fn set_vim_enabled(&mut self, enabled: bool) {
        self.composer.set_vim_enabled(enabled);
        self.request_redraw();
    }

    pub(crate) fn toggle_vim_enabled(&mut self) -> bool {
        let enabled = self.composer.toggle_vim_enabled();
        self.request_redraw();
        enabled
    }

    pub fn status_widget(&self) -> Option<&StatusIndicatorWidget> {
        self.status.as_ref()
    }

    pub fn skills(&self) -> Option<&Vec<SkillMetadata>> {
        self.composer.skills()
    }

    pub fn plugins(&self) -> Option<&Vec<PluginCapabilitySummary>> {
        self.composer.plugins()
    }

    #[cfg(test)]
    pub(crate) fn context_window_percent(&self) -> Option<i64> {
        self.context_window_percent
    }

    #[cfg(test)]
    pub(crate) fn context_window_used_tokens(&self) -> Option<i64> {
        self.context_window_used_tokens
    }

    fn active_view(&self) -> Option<&dyn BottomPaneView> {
        self.view_stack.last().map(std::convert::AsRef::as_ref)
    }

    fn push_view(&mut self, view: Box<dyn BottomPaneView>) {
        self.view_stack.push(view);
        self.schedule_active_view_frame();
        self.request_redraw();
    }

    fn pop_active_view_with_completion(&mut self, completion: Option<ViewCompletion>) {
        if self.view_stack.pop().is_some() {
            match completion {
                Some(ViewCompletion::Accepted) => {
                    while self
                        .view_stack
                        .last()
                        .is_some_and(|view| view.dismiss_after_child_accept())
                    {
                        self.view_stack.pop();
                    }
                }
                Some(ViewCompletion::Cancelled) => {
                    if let Some(view) = self.view_stack.last_mut() {
                        view.clear_dismiss_after_child_accept();
                    }
                }
                None => {}
            }
            self.on_view_stack_depth_decreased();
        }
    }

    fn on_view_stack_depth_decreased(&mut self) {
        if self.view_stack.is_empty() {
            self.on_active_view_complete();
        }
    }

    fn approval_prompt_delay_remaining(&self, now: Instant) -> Option<Duration> {
        self.last_composer_activity_at.and_then(|last_activity_at| {
            last_activity_at
                .checked_add(APPROVAL_PROMPT_TYPING_IDLE_DELAY)
                .and_then(|show_at| show_at.checked_duration_since(now))
                .filter(|delay| !delay.is_zero())
        })
    }

    fn record_composer_activity_at(&mut self, now: Instant) {
        self.last_composer_activity_at = Some(now);
        if !self.delayed_approval_requests.is_empty()
            && let Some(delay) = self.approval_prompt_delay_remaining(now)
        {
            self.request_redraw_in(delay);
        }
    }

    fn maybe_show_delayed_approval_requests_at(&mut self, now: Instant) {
        if self.delayed_approval_requests.is_empty() || !self.view_stack.is_empty() {
            return;
        }
        if let Some(delay) = self.approval_prompt_delay_remaining(now) {
            self.request_redraw_in(delay);
            return;
        }

        // 一旦输入空闲足够久，就提升（显示）最旧的延迟审批。
        // `ApprovalOverlay` 通过 `pop()` 推进其内部队列，因此从队尾开始清空
        // 剩余的延迟审批，以保持 FIFO（先进先出）的显示顺序。
        let Some(first) = self.delayed_approval_requests.pop_front() else {
            return;
        };
        let mut modal = ApprovalOverlay::new(
            first.request,
            self.app_event_tx.clone(),
            first.features,
            self.keymap.approval.clone(),
            self.keymap.list.clone(),
        );
        while let Some(delayed) = self.delayed_approval_requests.pop_back() {
            modal.enqueue_request(delayed.request);
        }
        self.pause_status_timer_for_modal();
        self.push_view(Box::new(modal));
    }

    /// 将按键事件转发给活动视图或输入框。
    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> InputResult {
        // 如果存在活动的模态视图，则在此处理；否则转发给输入框。
        if !self.view_stack.is_empty() {
            if key_event.kind == KeyEventKind::Release {
                return InputResult::None;
            }

            // 路由该键后我们需要三方面信息：
            // Esc 是否完成了视图、视图是否因任何原因结束，
            // 以及是否应调度粘贴突发（paste-burst）计时器。
            let (ctrl_c_completed, view_complete, completion, view_in_paste_burst) = {
                let last_index = self.view_stack.len() - 1;
                let view = &mut self.view_stack[last_index];
                let prefer_esc =
                    key_event.code == KeyCode::Esc && view.prefer_esc_to_handle_key_event();
                let ctrl_c_completed = key_event.code == KeyCode::Esc
                    && !prefer_esc
                    && matches!(view.on_ctrl_c(), CancellationEvent::Handled)
                    && view.is_complete();
                if ctrl_c_completed {
                    (true, true, view.completion(), false)
                } else {
                    view.handle_key_event(key_event);
                    (
                        false,
                        view.is_complete(),
                        view.completion(),
                        view.is_in_paste_burst(),
                    )
                }
            };

            if ctrl_c_completed {
                self.pop_active_view_with_completion(completion);
                if let Some(next_view) = self.view_stack.last()
                    && next_view.is_in_paste_burst()
                {
                    self.request_redraw_in(ChatComposer::recommended_paste_flush_delay());
                }
            } else if view_complete {
                self.pop_active_view_with_completion(completion);
            } else if view_in_paste_burst {
                self.request_redraw_in(ChatComposer::recommended_paste_flush_delay());
            }
            self.request_redraw();
            InputResult::None
        } else {
            // 如果任务正在运行且状态行可见，即使输入框持有焦点，
            // 也允许配置的动作进行中断。
            // 当存在活动弹窗时，优先关闭它而不是中断任务。
            if self.should_interrupt_running_task(key_event)
                && let Some(status) = &self.status
            {
                // 发送 Op::Interrupt
                status.interrupt();
                self.request_redraw();
                return InputResult::None;
            }
            let records_composer_activity =
                matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                    && !key_hint::has_ctrl_or_alt(key_event.modifiers)
                    && matches!(
                        key_event.code,
                        KeyCode::Char(_)
                            | KeyCode::Backspace
                            | KeyCode::Delete
                            | KeyCode::Enter
                            | KeyCode::Tab
                    );
            let (input_result, needs_redraw) = self.composer.handle_key_event(key_event);
            if records_composer_activity {
                self.record_composer_activity_at(Instant::now());
            }
            if needs_redraw {
                self.request_redraw();
            }
            if self.composer.is_in_paste_burst() {
                self.request_redraw_in(ChatComposer::recommended_paste_flush_delay());
            }
            input_result
        }
    }

    /// 处理底栏内的 Ctrl+C 按下事件。
    ///
    /// 活动模态视图会优先获得消费该键的机会（通常是让其自行关闭）。
    /// 如果没有活动视图，Ctrl+C 会先取消活动中的历史搜索，然后才回退到清空输入框草稿。
    ///
    /// 该方法可能显示退出快捷键提示，作为用户可见的、已收到 Ctrl+C 的确认，
    /// 但它不会决定进程是否应退出；`ChatWidget` 拥有退出/中断状态机，
    /// 并使用该结果来决定接下来发生什么。
    pub(crate) fn on_ctrl_c(&mut self) -> CancellationEvent {
        if let Some(view) = self.view_stack.last_mut() {
            let event = view.on_ctrl_c();
            let view_complete = view.is_complete();
            let completion = view.completion();
            if matches!(event, CancellationEvent::Handled) {
                if view_complete {
                    self.pop_active_view_with_completion(completion);
                }
                self.show_quit_shortcut_hint(key_hint::ctrl(KeyCode::Char('c')));
                self.request_redraw();
            }
            event
        } else if self.composer.cancel_history_search() {
            self.request_redraw();
            CancellationEvent::Handled
        } else if self.composer_is_empty() {
            CancellationEvent::NotHandled
        } else {
            self.view_stack.pop();
            self.clear_composer_for_ctrl_c();
            self.show_quit_shortcut_hint(key_hint::ctrl(KeyCode::Char('c')));
            self.request_redraw();
            CancellationEvent::Handled
        }
    }

    pub fn handle_paste(&mut self, pasted: String) {
        let has_pasted_text = !pasted.is_empty();
        if let Some(view) = self.view_stack.last_mut() {
            let needs_redraw = view.handle_paste(pasted);
            let view_complete = view.is_complete();
            if view_complete {
                self.view_stack.clear();
                self.on_active_view_complete();
            }
            if needs_redraw || view_complete {
                self.request_redraw();
            }
        } else {
            let needs_redraw = self.composer.handle_paste(pasted);
            if has_pasted_text {
                self.record_composer_activity_at(Instant::now());
            }
            if needs_redraw {
                self.request_redraw();
            }
        }
    }

    pub(crate) fn insert_str(&mut self, text: &str) {
        self.composer.insert_str(text);
        self.request_redraw();
    }

    pub(crate) fn pre_draw_tick(&mut self) {
        self.pre_draw_tick_at(Instant::now());
    }

    fn pre_draw_tick_at(&mut self, now: Instant) {
        self.composer.sync_popups();
        self.maybe_show_delayed_approval_requests_at(now);
        self.tick_active_view(now);
        self.schedule_active_view_frame();
    }

    fn tick_active_view(&mut self, now: Instant) {
        let Some(view) = self.view_stack.last_mut() else {
            return;
        };
        let needs_redraw = view.pre_draw_tick(now);
        let view_complete = view.is_complete();
        if view_complete {
            self.view_stack.clear();
            self.on_active_view_complete();
        }
        if needs_redraw || view_complete {
            self.request_redraw();
        }
    }

    fn schedule_active_view_frame(&self) {
        if let Some(delay) = self
            .active_view()
            .and_then(BottomPaneView::next_frame_delay)
        {
            self.request_redraw_in(delay);
        }
    }

    /// 用 `text` 替换输入框文本。
    ///
    /// 这适用于无需保留提及关联的全新输入；它会路由到
    /// `ChatComposer::set_text_content`，后者会重置提及绑定。
    pub(crate) fn set_composer_text(
        &mut self,
        text: String,
        text_elements: Vec<TextElement>,
        local_image_paths: Vec<PathBuf>,
    ) {
        self.composer
            .set_text_content(text, text_elements, local_image_paths);
        self.composer.move_cursor_to_end();
        self.request_redraw();
    }

    /// 替换输入框文本，同时保留提及链接目标。
    ///
    /// 当本地校验/门控失败（例如提交了不支持的图片）后需要重新填充草稿时使用此方法，
    /// 以便先前选中的提及目标在重试时保持稳定。
    pub(crate) fn set_composer_text_with_mention_bindings(
        &mut self,
        text: String,
        text_elements: Vec<TextElement>,
        local_image_paths: Vec<PathBuf>,
        mention_bindings: Vec<MentionBinding>,
    ) {
        self.composer.set_text_content_with_mention_bindings(
            text,
            text_elements,
            local_image_paths,
            mention_bindings,
        );
        self.composer.move_cursor_to_end();
        self.request_redraw();
    }

    #[allow(dead_code)]
    pub(crate) fn set_composer_input_enabled(
        &mut self,
        enabled: bool,
        placeholder: Option<String>,
    ) {
        self.composer.set_input_enabled(enabled, placeholder);
        self.request_redraw();
    }

    pub(crate) fn show_shutdown_in_progress(&mut self) {
        self.view_stack.clear();
        self.composer.show_shutdown_in_progress();
        self.request_redraw();
    }

    pub(crate) fn clear_composer_for_ctrl_c(&mut self) {
        if let Some(text) = self.composer.clear_for_ctrl_c() {
            if let Some(thread_id) = self.thread_id {
                self.app_event_tx
                    .send(AppEvent::AppendMessageHistoryEntry { thread_id, text });
            } else {
                tracing::warn!(
                    "failed to append Ctrl+C-cleared draft to history: no active thread id"
                );
            }
        }
        self.request_redraw();
    }

    /// 获取当前输入框文本（用于测试和程序化检查）。
    pub(crate) fn composer_text(&self) -> String {
        self.composer.current_text()
    }

    #[cfg(test)]
    pub(crate) fn composer_cursor(&self) -> usize {
        self.composer.cursor()
    }

    pub(crate) fn composer_draft_snapshot(&self) -> chat_composer::ComposerDraftSnapshot {
        self.composer.draft_snapshot()
    }

    #[cfg(test)]
    pub(crate) fn composer_text_elements(&self) -> Vec<TextElement> {
        self.composer.text_elements()
    }

    pub(crate) fn composer_local_images(&self) -> Vec<LocalImageAttachment> {
        self.composer.local_images()
    }

    #[cfg(test)]
    pub(crate) fn composer_local_image_paths(&self) -> Vec<PathBuf> {
        self.composer.local_image_paths()
    }

    pub(crate) fn composer_text_with_pending(&self) -> String {
        self.composer.current_text_with_pending()
    }

    /// 返回输入框当前是否接受交互式草稿编辑。
    pub(crate) fn composer_input_enabled(&self) -> bool {
        self.composer.input_enabled()
    }

    pub(crate) fn composer_pending_pastes(&self) -> Vec<(String, String)> {
        self.composer.pending_pastes()
    }

    pub(crate) fn apply_external_edit(&mut self, text: String) {
        self.composer.apply_external_edit(text);
        self.request_redraw();
    }

    pub(crate) fn set_footer_hint_override(&mut self, items: Option<Vec<(String, String)>>) {
        self.composer.set_footer_hint_override(items);
        self.request_redraw();
    }

    /// 将外部决定的 Plan 模式提示可见性应用到页脚展示上。
    pub(crate) fn set_plan_mode_nudge_visible(&mut self, visible: bool) {
        if self.composer.set_plan_mode_nudge_visible(visible) {
            self.request_redraw();
        }
    }

    #[cfg(test)]
    pub(crate) fn plan_mode_nudge_visible(&self) -> bool {
        self.composer.plan_mode_nudge_visible()
    }

    pub(crate) fn set_remote_image_urls(&mut self, urls: Vec<String>) {
        self.composer.set_remote_image_urls(urls);
        self.request_redraw();
    }

    pub(crate) fn remote_image_urls(&self) -> Vec<String> {
        self.composer.remote_image_urls()
    }

    pub(crate) fn take_remote_image_urls(&mut self) -> Vec<String> {
        let urls = self.composer.take_remote_image_urls();
        self.request_redraw();
        urls
    }

    pub(crate) fn set_composer_pending_pastes(&mut self, pending_pastes: Vec<(String, String)>) {
        self.composer.set_pending_pastes(pending_pastes);
        self.request_redraw();
    }

    /// 更新状态指示器的标题（默认为 “Working”）及其下方的详情。
    ///
    /// 传入 `None` 会清除已有的任何详情。返回活动状态指示器
    /// 是否已更新并请求了重绘。
    pub(crate) fn update_status(
        &mut self,
        header: String,
        details: Option<String>,
        details_capitalization: StatusDetailsCapitalization,
        details_max_lines: usize,
    ) -> bool {
        if let Some(status) = self.status.as_mut() {
            status.update_header(header);
            status.update_details(details, details_capitalization, details_max_lines.max(1));
            self.request_redraw();
            return true;
        }
        false
    }

    /// 为 `key` 显示短暂的“再次按下即可退出”提示。
    ///
    /// `ChatWidget` 拥有退出快捷键状态机（决定何时允许退出），而底栏负责渲染。
    /// 我们还会在 [`QUIT_SHORTCUT_TIMEOUT`] 之后调度一次重绘，
    /// 这样即使停止输入且没有其他事件触发绘制，提示也会消失。
    pub(crate) fn show_quit_shortcut_hint(&mut self, key: KeyBinding) {
        if !DOUBLE_PRESS_QUIT_SHORTCUT_ENABLED {
            return;
        }

        self.composer
            .show_quit_shortcut_hint(key, self.has_input_focus);
        let frame_requester = self.frame_requester.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                tokio::time::sleep(QUIT_SHORTCUT_TIMEOUT).await;
                frame_requester.schedule_frame();
            });
        } else {
            // 在测试（以及其他非 Tokio 环境）中，回退到线程，
            // 以便提示仍能在无需显式绘制的情况下过期。
            std::thread::spawn(move || {
                std::thread::sleep(QUIT_SHORTCUT_TIMEOUT);
                frame_requester.schedule_frame();
            });
        }
        self.request_redraw();
    }

    /// 立即清除“再次按下即可退出”提示。
    pub(crate) fn clear_quit_shortcut_hint(&mut self) {
        self.composer.clear_quit_shortcut_hint(self.has_input_focus);
        self.request_redraw();
    }

    #[cfg(test)]
    pub(crate) fn quit_shortcut_hint_visible(&self) -> bool {
        self.composer.quit_shortcut_hint_visible()
    }

    #[cfg(test)]
    pub(crate) fn status_indicator_visible(&self) -> bool {
        self.status.is_some()
    }

    #[cfg(test)]
    pub(crate) fn status_line_text(&self) -> Option<String> {
        self.composer.status_line_text()
    }

    pub(crate) fn show_esc_backtrack_hint(&mut self) {
        self.esc_backtrack_hint = true;
        self.composer.set_esc_backtrack_hint(/*show*/ true);
        self.request_redraw();
    }

    pub(crate) fn clear_esc_backtrack_hint(&mut self) {
        if self.esc_backtrack_hint {
            self.esc_backtrack_hint = false;
            self.composer.set_esc_backtrack_hint(/*show*/ false);
            self.request_redraw();
        }
    }

    // esc_backtrack_hint_visible 已移除；提示改由内部控制。

    pub fn set_task_running(&mut self, running: bool) {
        let was_running = self.is_task_running;
        self.is_task_running = running;
        self.composer.set_task_running(running);

        if running {
            if !was_running {
                if self.status.is_none() {
                    self.status = Some(StatusIndicatorWidget::new(
                        self.app_event_tx.clone(),
                        self.frame_requester.clone(),
                        self.animations_enabled,
                    ));
                }
                if let Some(status) = self.status.as_mut() {
                    status.set_interrupt_hint_visible(/*visible*/ true);
                    status.set_interrupt_binding(primary_binding(&self.keymap.chat.interrupt_turn));
                }
                self.sync_status_inline_message();
                self.request_redraw();
            }
        } else {
            // 任务完成时隐藏状态指示器，但保留其他模态视图。
            self.hide_status_indicator();
        }
    }

    pub(crate) fn set_queue_submissions(&mut self, queue_submissions: bool) {
        self.composer.set_queue_submissions(queue_submissions);
    }

    /// 隐藏状态指示器，同时保持任务运行状态不变。
    pub(crate) fn hide_status_indicator(&mut self) {
        if self.status.take().is_some() {
            self.request_redraw();
        }
    }

    pub(crate) fn ensure_status_indicator(&mut self) {
        if self.status.is_none() {
            self.status = Some(StatusIndicatorWidget::new(
                self.app_event_tx.clone(),
                self.frame_requester.clone(),
                self.animations_enabled,
            ));
            if let Some(status) = self.status.as_mut() {
                status.set_interrupt_binding(primary_binding(&self.keymap.chat.interrupt_turn));
            }
            self.sync_status_inline_message();
            self.request_redraw();
        }
    }

    pub(crate) fn set_interrupt_hint_visible(&mut self, visible: bool) {
        if let Some(status) = self.status.as_mut() {
            status.set_interrupt_hint_visible(visible);
            self.request_redraw();
        }
    }

    pub(crate) fn set_context_window(&mut self, percent: Option<i64>, used_tokens: Option<i64>) {
        if self.context_window_percent == percent && self.context_window_used_tokens == used_tokens
        {
            return;
        }

        self.context_window_percent = percent;
        self.context_window_used_tokens = used_tokens;
        self.composer
            .set_context_window(percent, self.context_window_used_tokens);
        self.request_redraw();
    }

    /// 使用提供的条目显示一个通用的列表选择视图。
    pub(crate) fn show_selection_view(
        &mut self,
        mut params: list_selection_view::SelectionViewParams,
    ) {
        self.apply_standard_popup_hint(&mut params);
        let view = list_selection_view::ListSelectionView::new(
            params,
            self.app_event_tx.clone(),
            self.keymap.list.clone(),
        );
        self.push_view(Box::new(view));
    }

    fn apply_standard_popup_hint(&self, params: &mut list_selection_view::SelectionViewParams) {
        if !params.allow_cancel {
            if params.footer_hint.is_none()
                || params.footer_hint.as_ref() == Some(&popup_consts::standard_popup_hint_line())
            {
                params.footer_hint = None;
            }
            return;
        }
        if params.footer_hint.is_none()
            || params.footer_hint.as_ref() == Some(&popup_consts::standard_popup_hint_line())
        {
            params.footer_hint = Some(self.standard_popup_hint_line());
        }
    }

    /// 当活动选择视图与 `view_id` 匹配时将其替换。
    pub(crate) fn replace_selection_view_if_active(
        &mut self,
        view_id: &'static str,
        mut params: list_selection_view::SelectionViewParams,
    ) -> bool {
        let is_match = self
            .view_stack
            .last()
            .is_some_and(|view| view.view_id() == Some(view_id));
        if !is_match {
            return false;
        }

        self.view_stack.pop();
        self.apply_standard_popup_hint(&mut params);
        let view = list_selection_view::ListSelectionView::new(
            params,
            self.app_event_tx.clone(),
            self.keymap.list.clone(),
        );
        self.push_view(Box::new(view));
        true
    }

    /// 替换最新的匹配选择视图，而不影响堆叠在其上方的视图。
    pub(crate) fn replace_selection_view_if_present(
        &mut self,
        view_id: &'static str,
        mut params: list_selection_view::SelectionViewParams,
    ) -> bool {
        let Some(index) = self
            .view_stack
            .iter()
            .rposition(|view| view.view_id() == Some(view_id))
        else {
            return false;
        };

        let replaces_active_view = index + 1 == self.view_stack.len();
        self.apply_standard_popup_hint(&mut params);
        self.view_stack[index] = Box::new(list_selection_view::ListSelectionView::new(
            params,
            self.app_event_tx.clone(),
            self.keymap.list.clone(),
        ));
        if replaces_active_view {
            self.schedule_active_view_frame();
        }
        self.request_redraw();
        true
    }

    pub(crate) fn standard_popup_hint_line(&self) -> Line<'static> {
        popup_consts::standard_popup_hint_line_for_keymap(&self.keymap.list)
    }

    pub(crate) fn list_keymap(&self) -> crate::tui_core::keymap::ListKeymap {
        self.keymap.list.clone()
    }

    /// 用一个通用的列表选择视图替换 ID 位于 `view_ids` 中的一个或多个活动视图。
    pub(crate) fn replace_active_views_with_selection_view(
        &mut self,
        view_ids: &[&'static str],
        mut params: list_selection_view::SelectionViewParams,
    ) -> bool {
        let is_match = self
            .view_stack
            .last()
            .and_then(|view| view.view_id())
            .is_some_and(|view_id| view_ids.contains(&view_id));
        if !is_match {
            return false;
        }

        while self
            .view_stack
            .last()
            .and_then(|view| view.view_id())
            .is_some_and(|view_id| view_ids.contains(&view_id))
        {
            self.view_stack.pop();
        }
        self.apply_standard_popup_hint(&mut params);
        let view = list_selection_view::ListSelectionView::new(
            params,
            self.app_event_tx.clone(),
            self.keymap.list.clone(),
        );
        self.push_view(Box::new(view));
        true
    }

    pub(crate) fn selected_index_for_active_view(&self, view_id: &'static str) -> Option<usize> {
        self.view_stack
            .last()
            .filter(|view| view.view_id() == Some(view_id))
            .and_then(|view| view.selected_index())
    }

    pub(crate) fn active_tab_id_for_active_view(&self, view_id: &'static str) -> Option<&str> {
        self.view_stack
            .last()
            .filter(|view| view.view_id() == Some(view_id))
            .and_then(|view| view.active_tab_id())
    }

    pub(crate) fn dismiss_active_view_if_id(&mut self, view_id: &'static str) -> bool {
        let is_match = self
            .view_stack
            .last()
            .is_some_and(|view| view.view_id() == Some(view_id));
        if !is_match {
            return false;
        }

        self.view_stack.pop();
        self.request_redraw();
        true
    }

    /// 关闭最新的匹配视图，而不影响堆叠在其上方的视图。
    pub(crate) fn dismiss_view_by_id(&mut self, view_id: &'static str) -> bool {
        let Some(index) = self
            .view_stack
            .iter()
            .rposition(|view| view.view_id() == Some(view_id))
        else {
            return false;
        };

        let removed_active_view = index + 1 == self.view_stack.len();
        self.view_stack.remove(index);
        if removed_active_view {
            self.schedule_active_view_frame();
        }
        self.request_redraw();
        true
    }

    /// 更新显示在输入框上方的待处理输入预览。
    pub(crate) fn set_pending_input_preview(
        &mut self,
        queued: Vec<String>,
        pending_steers: Vec<String>,
        rejected_steers: Vec<String>,
    ) {
        self.pending_input_preview.pending_steers = pending_steers;
        self.pending_input_preview.rejected_steers = rejected_steers;
        self.pending_input_preview.queued_messages = queued;
        self.request_redraw();
    }

    /// 更新显示在输入框上方的非活动线程审批列表。
    pub(crate) fn set_pending_thread_approvals(&mut self, threads: Vec<String>) {
        if self.pending_thread_approvals.set_threads(threads) {
            self.request_redraw();
        }
    }

    #[cfg(test)]
    pub(crate) fn pending_thread_approvals(&self) -> &[String] {
        self.pending_thread_approvals.threads()
    }

    /// 更新统一执行（unified exec）进程集，并刷新当前活动的任一摘要界面。
    ///
    /// 根据当前是否可见状态指示器，摘要可以内联显示在状态行中，
    /// 或作为独立的页脚行显示。
    pub(crate) fn set_unified_exec_processes(&mut self, processes: Vec<String>) {
        if self.unified_exec_footer.set_processes(processes) {
            self.sync_status_inline_message();
            self.request_redraw();
        }
    }

    /// 将统一执行（unified exec）摘要文本复制到活动状态行中（若有）。
    ///
    /// 这使状态行的内联文本保持同步，而无需强制显示独立的 unified exec 页脚行。
    fn sync_status_inline_message(&mut self) {
        if let Some(status) = self.status.as_mut() {
            status.update_inline_message(self.unified_exec_footer.summary_text());
        }
    }

    pub(crate) fn composer_is_empty(&self) -> bool {
        self.composer.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn composer_is_vim_enabled(&self) -> bool {
        self.composer.is_vim_enabled()
    }

    pub(crate) fn composer_should_handle_vim_insert_escape(&self, key_event: KeyEvent) -> bool {
        self.composer.should_handle_vim_insert_escape(key_event)
    }

    pub(crate) fn is_task_running(&self) -> bool {
        self.is_task_running
    }

    pub(crate) fn should_interrupt_running_task(&self, key_event: KeyEvent) -> bool {
        let is_agent_command = self
            .composer_text()
            .lines()
            .next()
            .and_then(parse_slash_name)
            .is_some_and(|(name, _, _)| name == "agent");

        self.keymap.chat.interrupt_turn.is_pressed(key_event)
            && self.is_task_running
            && !(is_agent_command && key_event.code == KeyCode::Esc)
            && self.no_modal_or_popup_active()
            && !self.composer_should_handle_vim_insert_escape(key_event)
            && self.status.is_some()
    }

    pub(crate) fn terminal_title_requires_action(&self) -> bool {
        self.active_view()
            .is_some_and(bottom_pane_view::BottomPaneView::terminal_title_requires_action)
    }

    pub(crate) fn has_active_view(&self) -> bool {
        !self.view_stack.is_empty()
    }

    pub(crate) fn active_view_will_interrupt_turn_on_key_event(&self, key_event: KeyEvent) -> bool {
        self.is_task_running
            && self
                .active_view()
                .is_some_and(|view| view.will_interrupt_turn_on_key_event(key_event))
    }

    #[cfg(test)]
    pub(crate) fn active_view_id(&self) -> Option<&'static str> {
        self.view_stack.last().and_then(|view| view.view_id())
    }

    /// 当面板处于常规输入框状态（没有任何覆盖层或弹窗且未运行任务）时返回 true。
    /// 这是从主视图使用 Esc-Esc 回溯的安全上下文。
    pub(crate) fn is_normal_backtrack_mode(&self) -> bool {
        !self.is_task_running && self.view_stack.is_empty() && !self.composer.popup_active()
    }

    /// 当没有活动的弹窗或模态视图时返回 true，与任务状态无关。
    pub(crate) fn can_launch_external_editor(&self) -> bool {
        self.view_stack.is_empty() && !self.composer.popup_active()
    }

    /// 当底栏没有活动模态视图且没有活动输入框弹窗时返回 true。
    ///
    /// 这是按键路由决策中“无模态/弹窗活动”的 UI 级定义。
    /// 它有意不包含任务状态，因为有些操作在任务运行期间是安全的，有些则不是。
    pub(crate) fn no_modal_or_popup_active(&self) -> bool {
        self.can_launch_external_editor()
    }

    pub(crate) fn show_view(&mut self, view: Box<dyn BottomPaneView>) {
        self.push_view(view);
    }

    /// 当 agent 请求用户审批时调用。
    pub fn push_approval_request(&mut self, request: ApprovalRequest, features: &Features) {
        let request = if let Some(view) = self.view_stack.last_mut() {
            match view.try_consume_approval_request(request) {
                Some(request) => request,
                None => {
                    self.request_redraw();
                    return;
                }
            }
        } else {
            request
        };

        let now = Instant::now();
        if !self.delayed_approval_requests.is_empty()
            || self.approval_prompt_delay_remaining(now).is_some()
        {
            self.delayed_approval_requests
                .push_back(DelayedApprovalRequest {
                    request,
                    features: features.clone(),
                });
            self.maybe_show_delayed_approval_requests_at(now);
        } else {
            // 近期没有输入框活动，因此立即显示审批模态框。
            let modal = ApprovalOverlay::new(
                request,
                self.app_event_tx.clone(),
                features.clone(),
                self.keymap.approval.clone(),
                self.keymap.list.clone(),
            );
            self.pause_status_timer_for_modal();
            self.push_view(Box::new(modal));
        }
    }

    /// 当 agent 请求用户输入时调用。
    pub fn push_user_input_request(&mut self, request: ToolRequestUserInputParams) {
        let request = if let Some(view) = self.view_stack.last_mut() {
            match view.try_consume_user_input_request(request) {
                Some(request) => request,
                None => {
                    self.request_redraw();
                    return;
                }
            }
        } else {
            request
        };

        let modal = RequestUserInputOverlay::new_with_keymap(
            request,
            self.app_event_tx.clone(),
            self.has_input_focus,
            self.enhanced_keys_supported,
            self.disable_paste_burst,
            self.keymap.clone(),
        );
        self.pause_status_timer_for_modal();
        self.set_composer_input_enabled(
            /*enabled*/ false,
            Some("Answer the questions to continue.".to_string()),
        );
        self.push_view(Box::new(modal));
    }

    pub(crate) fn push_mcp_server_elicitation_request(
        &mut self,
        request: McpServerElicitationFormRequest,
    ) {
        let request = if let Some(view) = self.view_stack.last_mut() {
            match view.try_consume_mcp_server_elicitation_request(request) {
                Some(request) => request,
                None => {
                    self.request_redraw();
                    return;
                }
            }
        } else {
            request
        };

        if let Some(tool_suggestion) = request.tool_suggestion()
            && let Some(install_url) = tool_suggestion.install_url.clone()
        {
            let suggestion_type = match tool_suggestion.suggest_type {
                mcp_server_elicitation::ToolSuggestionType::Install => {
                    AppLinkSuggestionType::Install
                }
                mcp_server_elicitation::ToolSuggestionType::Enable => AppLinkSuggestionType::Enable,
            };
            let is_installed = matches!(
                tool_suggestion.suggest_type,
                mcp_server_elicitation::ToolSuggestionType::Enable
            );
            let view = AppLinkView::new_with_keymap(
                AppLinkViewParams {
                    app_id: tool_suggestion.tool_id.clone(),
                    title: tool_suggestion.tool_name.clone(),
                    description: None,
                    instructions: match suggestion_type {
                        AppLinkSuggestionType::Install => {
                            "Install this app in your browser, then return here.".to_string()
                        }
                        AppLinkSuggestionType::Enable => {
                            "Enable this app to use it for the current request.".to_string()
                        }
                        AppLinkSuggestionType::Auth => unreachable!(
                            "auth uses URL mode elicitation, not tool suggestion forms"
                        ),
                        AppLinkSuggestionType::ExternalAction => unreachable!(
                            "external actions use URL mode elicitation, not tool suggestion forms"
                        ),
                    },
                    url: install_url,
                    is_installed,
                    is_enabled: false,
                    suggest_reason: Some(tool_suggestion.suggest_reason.clone()),
                    suggestion_type: Some(suggestion_type),
                    elicitation_target: Some(AppLinkElicitationTarget {
                        thread_id: request.thread_id(),
                        server_name: request.server_name().to_string(),
                        request_id: request.request_id().clone(),
                    }),
                },
                self.app_event_tx.clone(),
                self.keymap.list.clone(),
            );
            self.pause_status_timer_for_modal();
            self.set_composer_input_enabled(
                /*enabled*/ false,
                Some("Respond to the tool suggestion to continue.".to_string()),
            );
            self.push_view(Box::new(view));
            return;
        }

        let modal = McpServerElicitationOverlay::new_with_keymap(
            request,
            self.app_event_tx.clone(),
            self.has_input_focus,
            self.enhanced_keys_supported,
            self.disable_paste_burst,
            self.keymap.list.clone(),
        );
        self.pause_status_timer_for_modal();
        self.set_composer_input_enabled(
            /*enabled*/ false,
            Some("Respond to the MCP server request to continue.".to_string()),
        );
        self.push_view(Box::new(modal));
    }

    pub(crate) fn dismiss_app_server_request(
        &mut self,
        request: &ResolvedAppServerRequest,
    ) -> bool {
        let delayed_len = self.delayed_approval_requests.len();
        self.delayed_approval_requests
            .retain(|delayed| !delayed.request.matches_resolved_request(request));
        let delayed_changed = self.delayed_approval_requests.len() != delayed_len;

        if self.view_stack.is_empty() {
            if delayed_changed {
                self.request_redraw();
            }
            return delayed_changed;
        }

        let mut changed = delayed_changed;
        let mut completed_indices = Vec::new();
        for index in (0..self.view_stack.len()).rev() {
            let view = &mut self.view_stack[index];
            if !view.dismiss_app_server_request(request) {
                continue;
            }
            changed = true;
            if view.is_complete() {
                completed_indices.push(index);
            }
        }
        if !changed {
            return false;
        }
        for index in completed_indices {
            self.view_stack.remove(index);
        }
        self.on_view_stack_depth_decreased();
        self.request_redraw();
        true
    }

    fn on_active_view_complete(&mut self) {
        self.resume_status_timer_after_modal();
        self.set_composer_input_enabled(/*enabled*/ true, /*placeholder*/ None);
    }

    fn pause_status_timer_for_modal(&mut self) {
        if let Some(status) = self.status.as_mut() {
            status.pause_timer();
        }
    }

    fn resume_status_timer_after_modal(&mut self) {
        if let Some(status) = self.status.as_mut() {
            status.resume_timer();
        }
    }

    /// 当前底栏所需的高度（终端行数）。
    pub(crate) fn request_redraw(&self) {
        self.frame_requester.schedule_frame();
    }

    pub(crate) fn request_redraw_in(&self, dur: Duration) {
        self.frame_requester.schedule_frame_in(dur);
    }

    // --- 历史辅助方法 ---

    pub(crate) fn set_history_metadata(
        &mut self,
        thread_id: ThreadId,
        log_id: u64,
        entry_count: usize,
    ) {
        self.thread_id = Some(thread_id);
        self.composer
            .set_history_metadata(thread_id, log_id, entry_count);
    }

    pub(crate) fn flush_paste_burst_if_due(&mut self) -> bool {
        // 让活动视图优先刷新粘贴突发（paste-burst）状态，
        // 以便复用输入框的覆盖层行为保持一致。
        if let Some(view) = self.view_stack.last_mut()
            && view.flush_paste_burst_if_due()
        {
            return true;
        }
        self.composer.flush_paste_burst_if_due()
    }

    pub(crate) fn is_in_paste_burst(&self) -> bool {
        // 视图可以独立于主输入框持有粘贴突发（paste-burst）状态，因此先检查它。
        self.view_stack
            .last()
            .is_some_and(|view| view.is_in_paste_burst())
            || self.composer.is_in_paste_burst()
    }

    pub(crate) fn on_history_lookup_response(&mut self, response: HistoryLookupResponse) {
        let updated = match response {
            HistoryLookupResponse::Entry {
                offset,
                log_id,
                entry,
            } => self
                .composer
                .on_history_entry_response(log_id, offset, entry),
            HistoryLookupResponse::Batch {
                cursor,
                log_id,
                entries,
                next_older_cursor,
            } => {
                self.composer
                    .on_history_batch_response(log_id, cursor, entries, next_older_cursor)
            }
            HistoryLookupResponse::BatchError { cursor, log_id } => {
                self.composer.on_history_batch_error(log_id, cursor)
            }
        };
        if updated {
            self.composer.sync_popups();
            self.request_redraw();
        }
    }

    pub(crate) fn record_replayed_user_message_history(&mut self, entry: HistoryEntry) {
        self.composer.record_replayed_user_message_history(entry);
    }

    pub(crate) fn on_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        self.composer.on_file_search_result(query, matches);
        self.request_redraw();
    }

    pub(crate) fn attach_image(&mut self, path: PathBuf) {
        if self.view_stack.is_empty() {
            self.composer.attach_image(path);
            self.request_redraw();
        }
    }

    #[cfg(test)]
    pub(crate) fn take_recent_submission_images(&mut self) -> Vec<PathBuf> {
        self.composer.take_recent_submission_images()
    }

    pub(crate) fn take_recent_submission_images_with_placeholders(
        &mut self,
    ) -> Vec<LocalImageAttachment> {
        self.composer
            .take_recent_submission_images_with_placeholders()
    }

    pub(crate) fn prepare_inline_args_submission(
        &mut self,
        record_history: bool,
    ) -> Option<(String, Vec<TextElement>)> {
        self.composer.prepare_inline_args_submission(record_history)
    }

    fn as_renderable(&'_ self) -> RenderableItem<'_> {
        self.as_renderable_with_composer_right_reserve(/*composer_right_reserve*/ 0)
    }

    pub(crate) fn as_renderable_with_composer_right_reserve(
        &'_ self,
        composer_right_reserve: u16,
    ) -> RenderableItem<'_> {
        if let Some(view) = self.active_view() {
            RenderableItem::Borrowed(view)
        } else {
            let mut flex = FlexRenderable::new();
            if let Some(status) = &self.status {
                flex.push(/*flex*/ 0, RenderableItem::Borrowed(status));
            }
            // 避免重复展示同一摘要，并避免在状态行已可见时再额外添加一行。
            if self.status.is_none() && !self.unified_exec_footer.is_empty() {
                flex.push(
                    /*flex*/ 0,
                    RenderableItem::Borrowed(&self.unified_exec_footer),
                );
            }
            let has_pending_thread_approvals = !self.pending_thread_approvals.is_empty();
            let has_pending_input = !self.pending_input_preview.queued_messages.is_empty()
                || !self.pending_input_preview.pending_steers.is_empty()
                || !self.pending_input_preview.rejected_steers.is_empty();
            let has_status_or_footer =
                self.status.is_some() || !self.unified_exec_footer.is_empty();
            let has_inline_previews = has_pending_thread_approvals || has_pending_input;
            if has_inline_previews && has_status_or_footer {
                flex.push(/*flex*/ 0, RenderableItem::Owned("".into()));
            }
            flex.push(
                /*flex*/ 1,
                RenderableItem::Borrowed(&self.pending_thread_approvals),
            );
            if has_pending_thread_approvals && has_pending_input {
                flex.push(/*flex*/ 0, RenderableItem::Owned("".into()));
            }
            flex.push(
                /*flex*/ 1,
                RenderableItem::Borrowed(&self.pending_input_preview),
            );
            if !has_inline_previews && has_status_or_footer {
                flex.push(/*flex*/ 0, RenderableItem::Owned("".into()));
            }
            let mut flex2 = FlexRenderable::new();
            flex2.push(/*flex*/ 1, RenderableItem::Owned(flex.into()));
            let composer: RenderableItem<'_> = if composer_right_reserve == 0 {
                RenderableItem::Borrowed(&self.composer)
            } else {
                RenderableItem::Owned(Box::new(ChatComposerRightReserveRenderable {
                    composer: &self.composer,
                    right_reserve: composer_right_reserve,
                }))
            };
            flex2.push(/*flex*/ 0, composer);
            RenderableItem::Owned(Box::new(flex2))
        }
    }

    pub(crate) fn set_status_line(&mut self, status_line: Option<Line<'static>>) {
        if self.composer.set_status_line(status_line) {
            self.request_redraw();
        }
    }

    pub(crate) fn set_status_line_hyperlink(&mut self, url: Option<String>) {
        if self.composer.set_status_line_hyperlink(url) {
            self.request_redraw();
        }
    }

    pub(crate) fn set_status_line_enabled(&mut self, enabled: bool) {
        if self.composer.set_status_line_enabled(enabled) {
            self.request_redraw();
        }
    }

    /// 更新上下文相关的页脚标签，并且仅在发生变化时才请求重绘。
    ///
    /// 在可见线程趋于稳定、`App` 可能多次重新计算标签的线程切换期间，
    /// 这能让页脚的处理开销保持低廉。
    pub(crate) fn set_active_agent_label(&mut self, active_agent_label: Option<String>) {
        if self.composer.set_active_agent_label(active_agent_label) {
            self.request_redraw();
        }
    }

    pub(crate) fn set_side_conversation_context_label(&mut self, label: Option<String>) {
        if self.composer.set_side_conversation_context_label(label) {
            self.request_redraw();
        }
    }
}

struct ChatComposerRightReserveRenderable<'a> {
    composer: &'a chat_composer::ChatComposer,
    right_reserve: u16,
}

impl Renderable for ChatComposerRightReserveRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.composer.render_with_mask_and_textarea_right_reserve(
            area,
            buf,
            /*mask_char*/ None,
            self.right_reserve,
        );
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.composer
            .desired_height_with_textarea_right_reserve(width, self.right_reserve)
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.composer
            .cursor_pos_with_textarea_right_reserve(area, self.right_reserve)
    }

    fn cursor_style(&self, area: Rect) -> crossterm::cursor::SetCursorStyle {
        self.composer.cursor_style(area)
    }
}

impl Renderable for BottomPane {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.as_renderable().render(area, buf);
    }
    fn desired_height(&self, width: u16) -> u16 {
        self.as_renderable().desired_height(width)
    }
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.as_renderable().cursor_pos(area)
    }

    fn cursor_style(&self, area: Rect) -> crossterm::cursor::SetCursorStyle {
        self.as_renderable().cursor_style(area)
    }
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
