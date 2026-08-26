//! 状态面渲染。
//!
//! 注：本文件超过 800 行红线，但 `impl ChatWidget` 状态面方法簇为内聚的完整
//! 状态机（36 个方法），与上游逐行对应，按 AGENTS.md「内聚完整状态机例外」保留。

//! `ChatWidget` 的状态栏与终端标题渲染辅助函数。
//!
//! 将这些逻辑放在一个专注的子模块中，可以更方便地审查增量式的标题/状态
//! 行为，而无需翻遍 `chatwidget.rs` 的其余部分。

use super::*;
use crate::app_server_protocol::AskForApproval;
use crate::config_compat::ConfigLayerSource;
use crate::protocol_compat::config_types::ServiceTier;
use crate::protocol_compat::models::PermissionProfile;
use crate::tui_core::bottom_pane::status_line_from_segments;
use crate::tui_core::branch_summary;
use crate::tui_core::chatwidget::limit_label_for_window;
use crate::tui_core::chatwidget::rate_limits::get_limits_duration;
use crate::tui_core::legacy_core::config::Config;
use crate::tui_core::status::format_tokens_compact;
use crate::utils_sandbox_summary::summarize_permission_profile;

use super::status_state::TerminalTitleStatusKind;

/// 当用户未配置自定义选项时，终端标题中显示的项目。
/// 有意保持最小化：仅包含活动指示器 + 项目名称。
pub(super) const DEFAULT_TERMINAL_TITLE_ITEMS: [&str; 2] = ["activity", "project-name"];

/// 终端标题动画使用的盲文点状旋转指示器帧。
pub(super) const TERMINAL_TITLE_SPINNER_FRAMES: [&str; 10] =
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 终端标题中旋转指示器帧前进的时间间隔。
pub(super) const TERMINAL_TITLE_SPINNER_INTERVAL: Duration = Duration::from_millis(100);

/// 终端标题中「需要操作」状态闪烁相位之间的时间间隔。
const TERMINAL_TITLE_ACTION_REQUIRED_INTERVAL: Duration = Duration::from_secs(1);

/// 当代理（agent）阻塞在等待用户输入时，终端标题中显示的前缀。
const TERMINAL_TITLE_ACTION_REQUIRED_PREFIX: &str = "[ ! ] Action Required";
const TERMINAL_TITLE_ACTION_REQUIRED_PREFIX_HIDDEN: &str = "[ . ] Action Required";

#[derive(Debug)]
/// 单次刷新过程所解析得到的状态面配置。
///
/// 状态栏与终端标题共享一些代价较高或有状态的输入
/// （尤其是 git 分支查询和无效项目警告）。该快照使一次刷新过程能够
/// 将共享内容只计算一次，然后基于同一选择集合渲染两个状态面。
struct StatusSurfaceSelections {
    status_line_items: Vec<StatusLineItem>,
    invalid_status_line_items: Vec<String>,
    terminal_title_items: Vec<TerminalTitleItem>,
    invalid_terminal_title_items: Vec<String>,
}

impl StatusSurfaceSelections {
    fn uses_git_branch(&self) -> bool {
        self.status_line_items.contains(&StatusLineItem::GitBranch)
            || self
                .terminal_title_items
                .contains(&TerminalTitleItem::GitBranch)
    }

    fn uses_git_summary(&self) -> bool {
        self.status_line_items
            .contains(&StatusLineItem::PullRequestNumber)
            || self
                .status_line_items
                .contains(&StatusLineItem::BranchChanges)
    }

    fn uses_workspace_headline(&self) -> bool {
        self.status_line_items
            .contains(&StatusLineItem::WorkspaceHeadline)
    }
}

/// 以最近一次查询所用的 cwd 为键缓存的「项目根目录显示名称」。
///
/// 终端标题的刷新可能非常频繁，因此在工作目录未发生变化时，标题路径
/// 会避免反复向上遍历文件系统，以重新发现相同的项目根目录名称。
#[derive(Clone, Debug)]
pub(super) struct CachedProjectRootName {
    pub(super) cwd: PathBuf,
    pub(super) root_name: Option<String>,
}

impl ChatWidget {
    fn status_surface_selections(&self) -> StatusSurfaceSelections {
        let (status_line_items, invalid_status_line_items) = self.status_line_items_with_invalids();
        let (terminal_title_items, invalid_terminal_title_items) =
            self.terminal_title_items_with_invalids();
        StatusSurfaceSelections {
            status_line_items,
            invalid_status_line_items,
            terminal_title_items,
            invalid_terminal_title_items,
        }
    }

    fn warn_invalid_status_line_items_once(&mut self, invalid_items: &[String]) {
        if self.thread_id.is_some()
            && !invalid_items.is_empty()
            && self
                .status_line_invalid_items_warned
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let label = if invalid_items.len() == 1 {
                "item"
            } else {
                "items"
            };
            let message = format!(
                "Ignored invalid status line {label}: {}.",
                proper_join(invalid_items)
            );
            self.on_warning(message);
        }
    }

    fn warn_invalid_terminal_title_items_once(&mut self, invalid_items: &[String]) {
        if self.thread_id.is_some()
            && !invalid_items.is_empty()
            && self
                .terminal_title_invalid_items_warned
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            let label = if invalid_items.len() == 1 {
                "item"
            } else {
                "items"
            };
            let message = format!(
                "Ignored invalid terminal title {label}: {}.",
                proper_join(invalid_items)
            );
            self.on_warning(message);
        }
    }

    fn sync_status_surface_shared_state(&mut self, selections: &StatusSurfaceSelections) {
        if !selections.uses_git_branch() {
            self.status_line_branch = None;
            self.status_line_branch_pending = false;
            self.status_line_branch_lookup_complete = false;
        } else {
            let cwd = self.status_line_cwd().to_path_buf();
            self.sync_status_line_branch_state(&cwd);
            if !self.status_line_branch_lookup_complete {
                self.request_status_line_branch(cwd);
            }
        }

        if !selections.uses_git_summary() {
            self.status_line_git_summary = None;
            self.status_line_git_summary_pending = false;
            self.status_line_git_summary_lookup_complete = false;
        } else {
            let cwd = self.status_line_cwd().to_path_buf();
            self.sync_status_line_git_summary_state(&cwd);
            if !self.status_line_git_summary_lookup_complete {
                self.request_status_line_git_summary(cwd);
            }
        }

        if !selections.uses_workspace_headline() {
            self.status_line_workspace_headline = None;
            self.status_line_workspace_headline_pending_request_id = None;
            self.status_line_workspace_headline_last_requested_at = None;
            self.status_line_workspace_messages_disabled = false;
        } else {
            self.request_status_line_workspace_headline_if_due(Instant::now());
        }
    }

    fn refresh_status_line_from_selections(&mut self, selections: &StatusSurfaceSelections) {
        let enabled = !selections.status_line_items.is_empty();
        self.bottom_pane.set_status_line_enabled(enabled);
        if !enabled {
            self.set_status_line(/*status_line*/ None);
            self.set_status_line_hyperlink(/*url*/ None);
            return;
        }

        let mut segments = Vec::new();
        for item in &selections.status_line_items {
            if let Some(value) = self.status_line_value_for_item(*item) {
                segments.push((*item, value));
            }
        }

        self.set_status_line(status_line_from_segments(
            segments,
            self.config.tui_status_line_use_colors,
        ));
        let hyperlink_url = selections
            .status_line_items
            .contains(&StatusLineItem::PullRequestNumber)
            .then(|| self.status_line_pull_request_url())
            .flatten();
        self.set_status_line_hyperlink(hyperlink_url);
    }

/// 清除 Reflect 最近写入的终端标题（如果有）。
///
/// 该方法不会尝试恢复 shell 或终端此前的标题；它只清除受管理的标题，
/// 并在 OSC 写入成功后更新缓存。
    pub(crate) fn clear_managed_terminal_title(&mut self) -> std::io::Result<()> {
        if self.last_terminal_title.is_some() {
            clear_terminal_title()?;
            self.last_terminal_title = None;
        }

        Ok(())
    }

/// 针对一份已解析的选择快照，渲染并应用终端标题。
///
/// 空选择会清除受管理的标题。非空选择按配置顺序渲染当前值，
/// 跳过不可用的段，并缓存最后一次成功写入的标题，以避免冗余的 OSC 写入。
/// 当处于动画运行状态且包含 `activity` 项目时，还会调度下一帧，
/// 使标题动画持续推进。
    fn refresh_terminal_title_from_selections(&mut self, selections: &StatusSurfaceSelections) {
        self.last_terminal_title_requires_action =
            self.terminal_title_shows_action_required_with_selections(selections);
        if selections.terminal_title_items.is_empty() {
            if let Err(err) = self.clear_managed_terminal_title() {
                tracing::debug!(error = %err, "failed to clear terminal title");
            }
            return;
        }

        let now = Instant::now();
        let title = self.terminal_title_text_for_selections(selections, now);
        let animation_interval = self.terminal_title_animation_interval_with_selections(selections);
        if self.last_terminal_title == title {
            if let Some(interval) = animation_interval {
                self.frame_requester.schedule_frame_in(interval);
            }
            return;
        }
        match title {
            Some(title) => match set_terminal_title(&title) {
                Ok(SetTerminalTitleResult::Applied) => {
                    self.last_terminal_title = Some(title);
                }
                Ok(SetTerminalTitleResult::NoVisibleContent) => {
                    if let Err(err) = self.clear_managed_terminal_title() {
                        tracing::debug!(error = %err, "failed to clear terminal title");
                    }
                }
                Err(err) => {
                    tracing::debug!(error = %err, "failed to set terminal title");
                }
            },
            None => {
                if let Err(err) = self.clear_managed_terminal_title() {
                    tracing::debug!(error = %err, "failed to clear terminal title");
                }
            }
        }

        if let Some(interval) = animation_interval {
            self.frame_requester.schedule_frame_in(interval);
        }
    }

/// 基于一份共享的配置快照，重新计算两个状态面。
///
/// 这是底部状态栏与终端标题共用的刷新入口。它一次性解析两个配置，
/// 一次性发出无效项目警告，同步共享的缓存状态（例如 git 分支查询），
/// 然后基于该共享快照渲染每个状态面。
    pub(crate) fn refresh_status_surfaces(&mut self) {
        let selections = self.status_surface_selections();
        self.warn_invalid_status_line_items_once(&selections.invalid_status_line_items);
        self.warn_invalid_terminal_title_items_once(&selections.invalid_terminal_title_items);
        self.sync_status_surface_shared_state(&selections);
        self.refresh_status_line_from_selections(&selections);
        self.refresh_terminal_title_from_selections(&selections);
    }

    /// 根据配置与运行时状态，重新计算并输出终端标题。
    pub(crate) fn refresh_terminal_title(&mut self) {
        let selections = self.status_surface_selections();
        self.warn_invalid_terminal_title_items_once(&selections.invalid_terminal_title_items);
        self.sync_status_surface_shared_state(&selections);
        self.refresh_terminal_title_from_selections(&selections);
    }

    fn terminal_title_requires_action(&self) -> bool {
        self.bottom_pane.terminal_title_requires_action()
    }

    pub(super) fn terminal_title_shows_action_required(&self) -> bool {
        self.terminal_title_requires_action() && self.terminal_title_uses_activity()
    }

    fn terminal_title_text_for_selections(
        &mut self,
        selections: &StatusSurfaceSelections,
        now: Instant,
    ) -> Option<String> {
        if self.terminal_title_shows_action_required_with_selections(selections) {
            return Some(self.action_required_terminal_title_text(selections, now));
        }

        let mut previous = None;
        let title = selections
            .terminal_title_items
            .iter()
            .copied()
            .filter_map(|item| {
                self.terminal_title_value_for_item(item, now)
                    .map(|value| (item, value))
            })
            .fold(String::new(), |mut title, (item, value)| {
                title.push_str(item.separator_from_previous(previous));
                title.push_str(&value);
                previous = Some(item);
                title
            });
        (!title.is_empty()).then_some(title)
    }

    fn action_required_terminal_title_text(
        &mut self,
        selections: &StatusSurfaceSelections,
        now: Instant,
    ) -> String {
        crate::tui_core::bottom_pane::build_action_required_title_text(
            self.action_required_terminal_title_prefix_at(now),
            selections.terminal_title_items.iter().copied(),
            &[TerminalTitleItem::Status],
            |item| self.terminal_title_value_for_item(item, now),
        )
    }

    fn action_required_terminal_title_prefix_at(&self, now: Instant) -> &'static str {
        if !self.config.animations {
            return TERMINAL_TITLE_ACTION_REQUIRED_PREFIX;
        }

        let elapsed = now.saturating_duration_since(self.terminal_title_animation_origin);
        let phase = (elapsed.as_millis() / TERMINAL_TITLE_ACTION_REQUIRED_INTERVAL.as_millis()) % 2;
        if phase == 0 {
            TERMINAL_TITLE_ACTION_REQUIRED_PREFIX
        } else {
            TERMINAL_TITLE_ACTION_REQUIRED_PREFIX_HIDDEN
        }
    }

    fn terminal_title_shows_action_required_with_selections(
        &self,
        selections: &StatusSurfaceSelections,
    ) -> bool {
        self.terminal_title_requires_action()
            && selections
                .terminal_title_items
                .contains(&TerminalTitleItem::Spinner)
    }

    fn terminal_title_animation_interval_with_selections(
        &self,
        selections: &StatusSurfaceSelections,
    ) -> Option<Duration> {
        if self.config.animations
            && self.terminal_title_shows_action_required_with_selections(selections)
        {
            return Some(TERMINAL_TITLE_ACTION_REQUIRED_INTERVAL);
        }

        self.should_animate_terminal_title_spinner_with_selections(selections)
            .then_some(TERMINAL_TITLE_SPINNER_INTERVAL)
    }

    pub(super) fn request_status_line_branch_refresh(&mut self) {
        let selections = self.status_surface_selections();
        if !selections.uses_git_branch() {
            return;
        }
        let cwd = self.status_line_cwd().to_path_buf();
        self.sync_status_line_branch_state(&cwd);
        self.request_status_line_branch(cwd);
    }

    pub(super) fn request_status_line_git_summary_refresh(&mut self) {
        let selections = self.status_surface_selections();
        if !selections.uses_git_summary() {
            return;
        }
        let cwd = self.status_line_cwd().to_path_buf();
        self.sync_status_line_git_summary_state(&cwd);
        self.request_status_line_git_summary(cwd);
    }

/// 将配置的状态栏 id 解析为已知项目，并收集未知 id。
///
/// 未知 id 会按插入顺序去重，用于生成警告消息。
    fn status_line_items_with_invalids(&self) -> (Vec<StatusLineItem>, Vec<String>) {
        parse_items_with_invalids(self.configured_status_line_items())
    }

    pub(super) fn configured_status_line_items(&self) -> Vec<String> {
        self.config.tui_status_line.clone().unwrap_or_else(|| {
            DEFAULT_STATUS_LINE_ITEMS
                .iter()
                .map(ToString::to_string)
                .collect()
        })
    }

/// 将配置的终端标题 id 解析为已知项目，并收集未知 id。
///
/// 未知 id 会按插入顺序去重，用于生成警告消息。
    fn terminal_title_items_with_invalids(&self) -> (Vec<TerminalTitleItem>, Vec<String>) {
        parse_items_with_invalids(self.configured_terminal_title_items())
    }

    /// 返回配置的终端标题 id；未配置时返回默认顺序。
    pub(super) fn configured_terminal_title_items(&self) -> Vec<String> {
        self.config.tui_terminal_title.clone().unwrap_or_else(|| {
            DEFAULT_TERMINAL_TITLE_ITEMS
                .iter()
                .map(ToString::to_string)
                .collect()
        })
    }

    fn status_line_cwd(&self) -> &Path {
        self.current_cwd
            .as_deref()
            .unwrap_or(self.config.cwd.as_path())
    }

/// 解析与 `cwd` 关联的项目根目录。
///
/// 可用时优先采用 Git 仓库根目录。否则回退到最近的
/// 项目配置层，以便非 git 项目仍能呈现稳定的项目标签。
    fn status_line_project_root_for_cwd(&self, cwd: &Path) -> Option<PathBuf> {
        if let Some(repo_root) = get_git_repo_root(cwd) {
            return Some(repo_root);
        }

        self.config
            .config_layer_stack
            .get_layers(
                ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ true,
            )
            .iter()
            .find_map(|layer| match &layer.name {
                ConfigLayerSource::Project { dot_reflect_folder } => {
                    dot_reflect_folder.as_path().parent().map(Path::to_path_buf)
                }
                _ => None,
            })
    }

    fn status_line_project_root_name_for_cwd(&self, cwd: &Path) -> Option<String> {
        self.status_line_project_root_for_cwd(cwd).map(|root| {
            root.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| format_directory_display(&root, /*max_width*/ None))
        })
    }

    /// 返回当前 cwd 对应的、已缓存的「项目根目录显示名称」。
    fn status_line_project_root_name(&mut self) -> Option<String> {
        let cwd = self.status_line_cwd().to_path_buf();
        if let Some(cache) = &self.status_line_project_root_name_cache
            && cache.cwd == cwd
        {
            return cache.root_name.clone();
        }

        let root_name = self.status_line_project_root_name_for_cwd(&cwd);
        self.status_line_project_root_name_cache = Some(CachedProjectRootName {
            cwd,
            root_name: root_name.clone(),
        });
        root_name
    }

/// 生成终端标题的 `project` 值。
///
/// 优先使用已缓存的「项目根目录名称」；当无法推断出项目根目录时，
/// 回退到当前目录名称。
    fn terminal_title_project_name(&mut self) -> Option<String> {
        let project = self.status_line_project_root_name().or_else(|| {
            let cwd = self.status_line_cwd();
            Some(
                cwd.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| format_directory_display(cwd, /*max_width*/ None)),
            )
        })?;
        Some(Self::truncate_terminal_title_part(
            project, /*max_chars*/ 24,
        ))
    }

/// 当状态栏的 cwd 发生变化时，重置 git 分支缓存状态。
///
/// 分支缓存以 cwd 为键，因为分支查询是相对于该路径执行的。
/// 在 cwd 变化后保留过期的分支值，会暴露错误的仓库上下文。
    fn sync_status_line_branch_state(&mut self, cwd: &Path) {
        if self
            .status_line_branch_cwd
            .as_ref()
            .is_some_and(|path| path == cwd)
        {
            return;
        }
        self.status_line_branch_cwd = Some(cwd.to_path_buf());
        self.status_line_branch = None;
        self.status_line_branch_pending = false;
        self.status_line_branch_lookup_complete = false;
    }

    fn sync_status_line_git_summary_state(&mut self, cwd: &Path) {
        if self.status_line_git_summary_cwd.as_deref() == Some(cwd) {
            return;
        }
        self.status_line_git_summary_cwd = Some(cwd.to_path_buf());
        self.status_line_git_summary = None;
        self.status_line_git_summary_pending = false;
        self.status_line_git_summary_lookup_complete = false;
    }

/// 启动一次异步 git 分支查询；若已有查询正在进行则跳过。
///
/// 产生的 `StatusLineBranchUpdated` 事件携带查询所用的 cwd，
/// 以便调用方在目录变化后能够拒绝过期的完成结果。
    fn request_status_line_branch(&mut self, cwd: PathBuf) {
        if self.status_line_branch_pending {
            return;
        }
        let Some(runner) = self.workspace_command_runner.clone() else {
            self.status_line_branch_lookup_complete = true;
            return;
        };
        self.status_line_branch_pending = true;
        let tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            let branch = branch_summary::current_branch_name(runner.as_ref(), &cwd).await;
            tx.send(AppEvent::StatusLineBranchUpdated { cwd, branch });
        });
    }

    fn request_status_line_git_summary(&mut self, cwd: PathBuf) {
        if self.status_line_git_summary_pending {
            return;
        }
        let Some(runner) = self.workspace_command_runner.clone() else {
            self.status_line_git_summary_lookup_complete = true;
            return;
        };
        self.status_line_git_summary_pending = true;
        let tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            let summary = branch_summary::status_line_git_summary(runner.as_ref(), &cwd).await;
            tx.send(AppEvent::StatusLineGitSummaryUpdated { cwd, summary });
        });
    }

    fn request_status_line_workspace_headline_if_due(&mut self, now: Instant) {
        if !self.status_line_workspace_headline_should_fetch(now) {
            return;
        }
        let request_id = self.next_status_line_workspace_headline_request_id;
        self.next_status_line_workspace_headline_request_id = self
            .next_status_line_workspace_headline_request_id
            .wrapping_add(/*rhs*/ 1);
        self.status_line_workspace_headline_pending_request_id = Some(request_id);
        self.status_line_workspace_headline_last_requested_at = Some(now);
        self.app_event_tx
            .send(AppEvent::RefreshStatusLineWorkspaceHeadline { request_id });
    }

    fn status_line_workspace_headline_should_fetch(&self, now: Instant) -> bool {
        if self
            .status_line_workspace_headline_pending_request_id
            .is_some()
            || self.status_line_workspace_messages_disabled
            || !self.has_reflect_backend_auth
        {
            return false;
        }

        self.status_line_workspace_headline_last_requested_at
            .is_none_or(|last_requested_at| {
                now.saturating_duration_since(last_requested_at)
                    >= crate::tui_core::workspace_messages::WORKSPACE_HEADLINE_REFRESH_INTERVAL
            })
    }

    pub(super) fn refresh_status_line_if_workspace_headline_due(&mut self) {
        let now = Instant::now();
        if self.status_line_workspace_headline_should_fetch(now)
            && self
                .status_line_items_with_invalids()
                .0
                .contains(&StatusLineItem::WorkspaceHeadline)
        {
            self.refresh_status_line();
        }
    }

    pub(crate) fn set_status_line_workspace_headline(
        &mut self,
        request_id: u64,
        result: Result<crate::tui_core::workspace_messages::WorkspaceHeadlineFetchResult, String>,
    ) -> bool {
        if self.status_line_workspace_headline_pending_request_id != Some(request_id) {
            return false;
        }
        self.status_line_workspace_headline_pending_request_id = None;
        match result {
            Ok(crate::tui_core::workspace_messages::WorkspaceHeadlineFetchResult::Available(
                headline,
            )) => {
                self.status_line_workspace_messages_disabled = false;
                self.status_line_workspace_headline = headline;
            }
            Ok(
                crate::tui_core::workspace_messages::WorkspaceHeadlineFetchResult::FeatureDisabled,
            ) => {
                self.status_line_workspace_messages_disabled = true;
                self.status_line_workspace_headline = None;
            }
            Err(err) => {
                tracing::debug!(error = %err, "failed to fetch workspace headline");
            }
        }

        if !self.status_line_workspace_messages_disabled
            && self
                .status_line_items_with_invalids()
                .0
                .contains(&StatusLineItem::WorkspaceHeadline)
        {
            self.frame_requester.schedule_frame_in(
                crate::tui_core::workspace_messages::WORKSPACE_HEADLINE_REFRESH_INTERVAL,
            );
        }
        self.refresh_status_line();
        true
    }

/// 为某个已配置的状态栏项目解析显示字符串。
///
/// 返回 `None` 表示「暂时省略该项目」而非「配置错误」。调用方依赖
/// 这一行为，在等待会话、令牌或 git 元数据期间，保持部分可用的
/// 状态栏可读。
    pub(super) fn status_line_value_for_item(&mut self, item: StatusLineItem) -> Option<String> {
        match item {
            StatusLineItem::ModelName => Some(self.model_display_name().to_string()),
            StatusLineItem::ModelWithReasoning => Some(self.model_with_reasoning_display_name()),
            StatusLineItem::Reasoning => Some(self.reasoning_display_name()),
            StatusLineItem::CurrentDir => {
                Some(format_directory_display(
                    self.status_line_cwd(),
                    /*max_width*/ None,
                ))
            }
            StatusLineItem::ProjectRoot => self.status_line_project_root_name(),
            StatusLineItem::GitBranch => self.status_line_branch.clone(),
            StatusLineItem::PullRequestNumber => self
                .status_line_git_summary
                .as_ref()
                .and_then(|summary| summary.pull_request.as_ref())
                .map(|pull_request| format!("PR #{}", pull_request.number)),
            StatusLineItem::BranchChanges => self
                .status_line_git_summary
                .as_ref()
                .and_then(|summary| summary.branch_change_stats.as_ref())
                .map(|stats| {
                    if stats.additions == 0 && stats.deletions == 0 {
                        "No changes".to_string()
                    } else {
                        format!("+{} -{}", stats.additions, stats.deletions)
                    }
                }),
            StatusLineItem::Status => Some(self.run_state_status_text()),
            StatusLineItem::Permissions => Some(permissions_display(&self.config)),
            StatusLineItem::ApprovalMode => Some(approval_mode_display(&self.config)),
            StatusLineItem::UsedTokens => {
                let usage = self.status_line_total_usage();
                let total = usage.blended_total();
                if total <= 0 {
                    None
                } else {
                    Some(format!("{} used", format_tokens_compact(total)))
                }
            }
            StatusLineItem::ContextRemaining => self
                .status_line_context_remaining_percent()
                .map(|remaining| format!("Context {remaining}% left")),
            StatusLineItem::ContextUsed => self
                .status_line_context_used_percent()
                .map(|used| format!("Context {used}% used")),
            StatusLineItem::FiveHourLimit => {
                let (window, is_secondary) = self
                    .rate_limit_snapshots_by_limit_id
                    .get("reflect")
                    .and_then(five_hour_status_window)?;
                let label = limit_label_for_window(window.window_minutes, is_secondary);
                self.status_line_limit_display(Some(window), &label)
            }
            StatusLineItem::WeeklyLimit => {
                let (window, is_secondary) = self
                    .rate_limit_snapshots_by_limit_id
                    .get("reflect")
                    .and_then(weekly_status_window)?;
                let label = limit_label_for_window(window.window_minutes, is_secondary);
                self.status_line_limit_display(Some(window), &label)
            }
            StatusLineItem::ReflectVersion => Some(REFLECT_CLI_VERSION.to_string()),
            StatusLineItem::ContextWindowSize => self
                .status_line_context_window_size()
                .map(|cws| format!("{} window", format_tokens_compact(cws))),
            StatusLineItem::TotalInputTokens => Some(format!(
                "{} in",
                format_tokens_compact(self.status_line_total_usage().input_tokens)
            )),
            StatusLineItem::TotalOutputTokens => Some(format!(
                "{} out",
                format_tokens_compact(self.status_line_total_usage().output_tokens)
            )),
            StatusLineItem::SessionId => self.thread_id.map(|id| id.to_string()),
            StatusLineItem::FastMode => Some(
                if self.current_service_tier() == Some(ServiceTier::Fast.request_value()) {
                    "Fast on".to_string()
                } else {
                    "Fast off".to_string()
                },
            ),
            StatusLineItem::RawOutput => self.raw_output_mode().then(|| "raw output".to_string()),
            StatusLineItem::ThreadTitle => self.thread_name.as_ref().map_or_else(
                || self.thread_id.map(|id| id.to_string()),
                |name| {
                    let trimmed = name.trim();
                    if trimmed.is_empty() {
                        self.thread_id.map(|id| id.to_string())
                    } else {
                        Some(trimmed.to_string())
                    }
                },
            ),
            StatusLineItem::WorkspaceHeadline => self.status_line_workspace_headline.clone(),
            StatusLineItem::TaskProgress => self.terminal_title_task_progress(),
        }
    }

    fn status_line_pull_request_url(&self) -> Option<String> {
        self.status_line_git_summary
            .as_ref()
            .and_then(|summary| summary.pull_request.as_ref())
            .map(|pull_request| pull_request.url.clone())
    }

    pub(super) fn status_surface_preview_value_for_item(
        &mut self,
        item: StatusSurfacePreviewItem,
    ) -> Option<String> {
        let status_line_item = match item {
            StatusSurfacePreviewItem::AppName => return Some("reflect".to_string()),
            StatusSurfacePreviewItem::ProjectName => return self.terminal_title_project_name(),
            StatusSurfacePreviewItem::ProjectRoot => StatusLineItem::ProjectRoot,
            StatusSurfacePreviewItem::Status => return Some(self.run_state_status_text()),
            StatusSurfacePreviewItem::TaskProgress => return self.terminal_title_task_progress(),
            StatusSurfacePreviewItem::CurrentDir => StatusLineItem::CurrentDir,
            StatusSurfacePreviewItem::ThreadTitle => StatusLineItem::ThreadTitle,
            StatusSurfacePreviewItem::GitBranch => StatusLineItem::GitBranch,
            StatusSurfacePreviewItem::PullRequestNumber => StatusLineItem::PullRequestNumber,
            StatusSurfacePreviewItem::BranchChanges => StatusLineItem::BranchChanges,
            StatusSurfacePreviewItem::Permissions => StatusLineItem::Permissions,
            StatusSurfacePreviewItem::ApprovalMode => StatusLineItem::ApprovalMode,
            StatusSurfacePreviewItem::ContextRemaining => StatusLineItem::ContextRemaining,
            StatusSurfacePreviewItem::ContextUsed => StatusLineItem::ContextUsed,
            StatusSurfacePreviewItem::FiveHourLimit => StatusLineItem::FiveHourLimit,
            StatusSurfacePreviewItem::WeeklyLimit => StatusLineItem::WeeklyLimit,
            StatusSurfacePreviewItem::ReflectVersion => StatusLineItem::ReflectVersion,
            StatusSurfacePreviewItem::ContextWindowSize => StatusLineItem::ContextWindowSize,
            StatusSurfacePreviewItem::UsedTokens => StatusLineItem::UsedTokens,
            StatusSurfacePreviewItem::TotalInputTokens => StatusLineItem::TotalInputTokens,
            StatusSurfacePreviewItem::TotalOutputTokens => StatusLineItem::TotalOutputTokens,
            StatusSurfacePreviewItem::SessionId => StatusLineItem::SessionId,
            StatusSurfacePreviewItem::FastMode => StatusLineItem::FastMode,
            StatusSurfacePreviewItem::RawOutput => StatusLineItem::RawOutput,
            StatusSurfacePreviewItem::WorkspaceHeadline => StatusLineItem::WorkspaceHeadline,
            StatusSurfacePreviewItem::Model => StatusLineItem::ModelName,
            StatusSurfacePreviewItem::ModelWithReasoning => StatusLineItem::ModelWithReasoning,
            StatusSurfacePreviewItem::Reasoning => StatusLineItem::Reasoning,
        };
        self.status_line_value_for_item(status_line_item)
    }
/// 将某个已配置的终端标题项目解析为可显示的段。
///
/// 返回 `None` 表示「暂时省略该段」，这样调用方可以在隐藏尚不可用的
/// 值的同时，保持配置的顺序不变。
    pub(super) fn terminal_title_value_for_item(
        &mut self,
        item: TerminalTitleItem,
        now: Instant,
    ) -> Option<String> {
        match item {
            TerminalTitleItem::AppName => Some("reflect".to_string()),
            TerminalTitleItem::Project => self.terminal_title_project_name(),
            TerminalTitleItem::CurrentDir => Some(Self::truncate_terminal_title_part(
                format_directory_display(self.status_line_cwd(), /*max_width*/ None),
                /*max_chars*/ 32,
            )),
            TerminalTitleItem::Spinner => self.terminal_title_spinner_text_at(now),
            TerminalTitleItem::Status => Some(self.run_state_status_text()),
            TerminalTitleItem::Thread => self
                .status_line_value_for_item(StatusLineItem::ThreadTitle)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 48)),
            TerminalTitleItem::GitBranch => self.status_line_branch.as_ref().map(|branch| {
                Self::truncate_terminal_title_part(branch.clone(), /*max_chars*/ 32)
            }),
            TerminalTitleItem::ContextRemaining => self
                .status_line_value_for_item(StatusLineItem::ContextRemaining)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::ContextUsed => self
                .status_line_value_for_item(StatusLineItem::ContextUsed)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::FiveHourLimit => self
                .status_line_value_for_item(StatusLineItem::FiveHourLimit)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::WeeklyLimit => self
                .status_line_value_for_item(StatusLineItem::WeeklyLimit)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::ReflectVersion => self
                .status_line_value_for_item(StatusLineItem::ReflectVersion)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::UsedTokens => self
                .status_line_value_for_item(StatusLineItem::UsedTokens)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::TotalInputTokens => self
                .status_line_value_for_item(StatusLineItem::TotalInputTokens)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::TotalOutputTokens => self
                .status_line_value_for_item(StatusLineItem::TotalOutputTokens)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::SessionId => self
                .status_line_value_for_item(StatusLineItem::SessionId)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::FastMode => self
                .status_line_value_for_item(StatusLineItem::FastMode)
                .map(|value| Self::truncate_terminal_title_part(value, /*max_chars*/ 32)),
            TerminalTitleItem::Model => Some(Self::truncate_terminal_title_part(
                self.model_display_name().to_string(),
                /*max_chars*/ 32,
            )),
            TerminalTitleItem::ModelWithReasoning => Some(Self::truncate_terminal_title_part(
                self.model_with_reasoning_display_name(),
                /*max_chars*/ 32,
            )),
            TerminalTitleItem::Reasoning => Some(Self::truncate_terminal_title_part(
                self.reasoning_display_name(),
                /*max_chars*/ 32,
            )),
            TerminalTitleItem::TaskProgress => self.terminal_title_task_progress(),
        }
    }

    fn reasoning_display_name(&self) -> String {
        let effort = self.effective_reasoning_effort();
        Self::status_line_reasoning_effort_label(effort.as_ref())
    }

    fn model_with_reasoning_display_name(&self) -> String {
        let label = self.reasoning_display_name();
        let service_tier_label = self
            .current_service_tier()
            .and_then(|service_tier| {
                self.current_model_service_tier_commands()
                    .into_iter()
                    .find(|tier| tier.id == service_tier)
                    .map(|tier| tier.name)
            })
            .filter(|_| self.has_chatgpt_account)
            .map(|tier| format!(" {tier}"))
            .unwrap_or_default();
        format!("{} {label}{service_tier_label}", self.model_display_name())
    }

/// 计算基于文字的状态项目所使用的紧凑运行时状态标签。
///
/// 启动状态优先于普通任务状态；无论最后一次活跃的状态桶是什么，
/// 空闲状态都渲染为 `Ready`。
    pub(super) fn run_state_status_text(&self) -> String {
        if self.mcp_startup_status.is_some() {
            return "Starting".to_string();
        }

        match self.status_state.terminal_title_status_kind {
            TerminalTitleStatusKind::Working if !self.bottom_pane.is_task_running() => {
                "Ready".to_string()
            }
            TerminalTitleStatusKind::WaitingForBackgroundTerminal
                if !self.bottom_pane.is_task_running() =>
            {
                "Ready".to_string()
            }
            TerminalTitleStatusKind::Thinking if !self.bottom_pane.is_task_running() => {
                "Ready".to_string()
            }
            TerminalTitleStatusKind::Working => "Working".to_string(),
            TerminalTitleStatusKind::WaitingForBackgroundTerminal => "Waiting".to_string(),
            TerminalTitleStatusKind::Thinking => "Thinking".to_string(),
        }
    }

    pub(super) fn terminal_title_spinner_text_at(&self, now: Instant) -> Option<String> {
        if !self.config.animations {
            return None;
        }

        if !self.terminal_title_has_active_progress() {
            return None;
        }

        Some(self.terminal_title_spinner_frame_at(now).to_string())
    }

    fn terminal_title_spinner_frame_at(&self, now: Instant) -> &'static str {
        let elapsed = now.saturating_duration_since(self.terminal_title_animation_origin);
        let frame_index =
            (elapsed.as_millis() / TERMINAL_TITLE_SPINNER_INTERVAL.as_millis()) as usize;
        TERMINAL_TITLE_SPINNER_FRAMES[frame_index % TERMINAL_TITLE_SPINNER_FRAMES.len()]
    }

    fn terminal_title_uses_activity(&self) -> bool {
        self.config.tui_terminal_title.as_ref().is_none_or(|items| {
            items
                .iter()
                .any(|item| item == "activity" || item == "spinner")
        })
    }

    fn terminal_title_has_active_progress(&self) -> bool {
        if self.terminal_title_shows_action_required() {
            return false;
        }

        self.mcp_startup_status.is_some() || self.bottom_pane.is_task_running()
    }

    pub(super) fn should_animate_terminal_title_spinner(&self) -> bool {
        self.config.animations
            && self.terminal_title_uses_activity()
            && self.terminal_title_has_active_progress()
    }

    pub(super) fn should_animate_terminal_title_action_required(&self) -> bool {
        self.config.animations && self.terminal_title_shows_action_required()
    }

    fn should_animate_terminal_title_spinner_with_selections(
        &self,
        selections: &StatusSurfaceSelections,
    ) -> bool {
        self.config.animations
            && selections
                .terminal_title_items
                .contains(&TerminalTitleItem::Spinner)
            && self.terminal_title_has_active_progress()
    }

    /// 将最近一次 `update_plan` 的进度快照格式化为终端标题显示内容。
    pub(super) fn terminal_title_task_progress(&self) -> Option<String> {
        let (completed, total) = self.transcript.last_plan_progress?;
        if total == 0 {
            return None;
        }
        Some(format!("Tasks {completed}/{total}"))
    }

    /// 按字素簇截断标题段，并在需要时追加 `...`。
    pub(super) fn truncate_terminal_title_part(value: String, max_chars: usize) -> String {
        if max_chars == 0 {
            return String::new();
        }

        let mut graphemes = value.graphemes(true);
        let head: String = graphemes.by_ref().take(max_chars).collect();
        if graphemes.next().is_none() || max_chars <= 3 {
            return head;
        }

        let mut truncated = head.graphemes(true).take(max_chars - 3).collect::<String>();
        truncated.push_str("...");
        truncated
    }
}

// ── rate-limit 窗口/配置展示辅助（外移子模块） ──
mod helpers;
use helpers::*;
