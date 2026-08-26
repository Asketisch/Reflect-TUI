//! 用于自定义 TUI 状态栏的状态行配置视图。
//!
//! 此模块提供一个交互式选择器，用于选择终端底部状态行中
//! 显示哪些条目。用户可以：
//!
//! - **选择条目**：切换显示哪些信息
//! - **重新排序**：使用左右方向键更改显示顺序
//! - **预览更改**：查看配置的状态行的实时预览
//!
//! # 可用状态行条目
//!
//! - 模型信息（名称、推理级别）
//! - 目录路径（当前目录、项目根目录）
//! - Git 信息（分支名称）
//! - 权限配置
//! - 审批模式
//! - 上下文用量（剩余 %、已用 %、窗口大小）
//! - 用量限额（主限额、次限额）
//! - 会话信息（线程标题、线程 ID、已用 token）
//! - 应用版本

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::collections::HashSet;
use strum::IntoEnumIterator;
use strum_macros::Display;
use strum_macros::EnumIter;
use strum_macros::EnumString;

use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::bottom_pane::CancellationEvent;
use crate::tui_core::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::tui_core::bottom_pane::multi_select_picker::MultiSelectItem;
use crate::tui_core::bottom_pane::multi_select_picker::MultiSelectPicker;
use crate::tui_core::bottom_pane::status_surface_preview::StatusSurfacePreviewData;
use crate::tui_core::bottom_pane::status_surface_preview::StatusSurfacePreviewItem;
use crate::tui_core::keymap::ListKeymap;
use crate::tui_core::render::renderable::Renderable;

const STATUS_LINE_USE_THEME_COLORS_ITEM_ID: &str = "status-line-use-theme-colors";

/// 可以在状态行中显示的可用条目。
///
/// 每个变体表示可以在 TUI 底部显示的一条信息。
/// 条目被序列化为 kebab-case 用于配置
/// 存储（例如 `ModelWithReasoning` 变为 `model-with-reasoning`）。
///
/// 部分条目根据可用性有条件地显示：
/// - Git 相关条目仅在 git 仓库中显示
/// - 上下文/限额条目仅在 API 提供数据时显示
/// - 线程 ID 仅在会话启动后显示
#[derive(EnumIter, EnumString, Display, Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
#[strum(serialize_all = "kebab_case")]
pub(crate) enum StatusLineItem {
    /// 当前模型名称。
    #[strum(to_string = "model", serialize = "model-name")]
    ModelName,

    /// 带推理级别后缀的模型名称。
    ModelWithReasoning,

    /// 当前推理级别。
    Reasoning,

    /// 当前工作目录路径。
    CurrentDir,

    /// 项目根目录（如果检测到）。
    #[strum(
        to_string = "project-name",
        serialize = "project",
        serialize = "project-root"
    )]
    ProjectRoot,

    /// 当前 git 分支名称（如果在仓库中）。
    GitBranch,

    /// 当前分支的打开拉取请求编号。
    PullRequestNumber,

    /// 相对默认分支的已提交分支 diff 统计。
    BranchChanges,

    /// 紧凑的运行时状态文本。
    #[strum(to_string = "run-state", serialize = "status")]
    Status,

    /// 活动权限配置或沙箱摘要。
    Permissions,

    /// 活动命令审批模式。
    #[strum(to_string = "approval-mode", serialize = "approval")]
    ApprovalMode,

    /// 上下文窗口剩余百分比。
    ContextRemaining,

    /// 上下文窗口已用百分比。
    ///
    /// 也接受遗留的 `context-usage` 配置值。
    #[strum(to_string = "context-used", serialize = "context-usage")]
    ContextUsed,

    /// 主速率限额的剩余用量。
    FiveHourLimit,

    /// 次速率限额的剩余用量。
    WeeklyLimit,

    /// Reflect 应用版本。
    ReflectVersion,

    /// 总上下文窗口大小（token 数）。
    ContextWindowSize,

    /// 当前会话使用的总 token 数。
    UsedTokens,

    /// 消耗的总输入 token 数。
    TotalInputTokens,

    /// 生成的总输出 token 数。
    TotalOutputTokens,

    /// 完整线程 UUID。
    #[strum(to_string = "thread-id", serialize = "session-id")]
    SessionId,

    /// Fast 模式当前是否激活。
    FastMode,

    /// 原始滚回模式当前是否激活。
    RawOutput,

    /// 当前线程标题（如果用户设置）。
    ThreadTitle,

    /// 当前工作区通知标题。
    WorkspaceHeadline,

    /// 来自 `update_plan` 的最新清单任务进度（如果可用）。
    TaskProgress,
}

impl StatusLineItem {
    /// 在弹窗中展示的用户可见描述。
    pub(crate) fn description(self) -> &'static str {
        match self {
            StatusLineItem::ModelName => "Current model name",
            StatusLineItem::ModelWithReasoning => "Current model name with reasoning level",
            StatusLineItem::Reasoning => "Current reasoning level",
            StatusLineItem::CurrentDir => "Current working directory",
            StatusLineItem::ProjectRoot => "Project name (omitted when unavailable)",
            StatusLineItem::GitBranch => "Current Git branch (omitted when unavailable)",
            StatusLineItem::PullRequestNumber => {
                "Open pull request number for the current branch (omitted when unavailable)"
            }
            StatusLineItem::BranchChanges => {
                "Committed branch changes against the default branch (omitted when unavailable)"
            }
            StatusLineItem::Status => "Compact session run-state text (Ready, Working, Thinking)",
            StatusLineItem::Permissions => "Active permission profile or sandbox mode",
            StatusLineItem::ApprovalMode => "Active command approval mode",
            StatusLineItem::ContextRemaining => {
                "Percentage of context window remaining (omitted when unknown)"
            }
            StatusLineItem::ContextUsed => {
                "Percentage of context window used (omitted when unknown)"
            }
            StatusLineItem::FiveHourLimit => {
                "Remaining usage on the primary usage limit (omitted when unavailable)"
            }
            StatusLineItem::WeeklyLimit => {
                "Remaining usage on the secondary usage limit (omitted when unavailable)"
            }
            StatusLineItem::ReflectVersion => "Reflect application version",
            StatusLineItem::ContextWindowSize => {
                "Total context window size in tokens (omitted when unknown)"
            }
            StatusLineItem::UsedTokens => "Total tokens used in session (omitted when zero)",
            StatusLineItem::TotalInputTokens => "Total input tokens used in session",
            StatusLineItem::TotalOutputTokens => "Total output tokens used in session",
            StatusLineItem::SessionId => "Current thread identifier (omitted until thread starts)",
            StatusLineItem::FastMode => "Whether Fast mode is currently active",
            StatusLineItem::RawOutput => "Whether raw scrollback mode is active",
            StatusLineItem::ThreadTitle => {
                "Current thread title, or thread identifier when unnamed"
            }
            StatusLineItem::WorkspaceHeadline => {
                "Workspace notification headline (Enterprise workspaces only; omitted when unavailable)"
            }
            StatusLineItem::TaskProgress => {
                "Latest task progress from update_plan (omitted until available)"
            }
        }
    }

    pub(crate) fn preview_item(self) -> StatusSurfacePreviewItem {
        match self {
            StatusLineItem::ModelName => StatusSurfacePreviewItem::Model,
            StatusLineItem::ModelWithReasoning => StatusSurfacePreviewItem::ModelWithReasoning,
            StatusLineItem::Reasoning => StatusSurfacePreviewItem::Reasoning,
            StatusLineItem::CurrentDir => StatusSurfacePreviewItem::CurrentDir,
            StatusLineItem::ProjectRoot => StatusSurfacePreviewItem::ProjectRoot,
            StatusLineItem::GitBranch => StatusSurfacePreviewItem::GitBranch,
            StatusLineItem::PullRequestNumber => StatusSurfacePreviewItem::PullRequestNumber,
            StatusLineItem::BranchChanges => StatusSurfacePreviewItem::BranchChanges,
            StatusLineItem::Status => StatusSurfacePreviewItem::Status,
            StatusLineItem::Permissions => StatusSurfacePreviewItem::Permissions,
            StatusLineItem::ApprovalMode => StatusSurfacePreviewItem::ApprovalMode,
            StatusLineItem::ContextRemaining => StatusSurfacePreviewItem::ContextRemaining,
            StatusLineItem::ContextUsed => StatusSurfacePreviewItem::ContextUsed,
            StatusLineItem::FiveHourLimit => StatusSurfacePreviewItem::FiveHourLimit,
            StatusLineItem::WeeklyLimit => StatusSurfacePreviewItem::WeeklyLimit,
            StatusLineItem::ReflectVersion => StatusSurfacePreviewItem::ReflectVersion,
            StatusLineItem::ContextWindowSize => StatusSurfacePreviewItem::ContextWindowSize,
            StatusLineItem::UsedTokens => StatusSurfacePreviewItem::UsedTokens,
            StatusLineItem::TotalInputTokens => StatusSurfacePreviewItem::TotalInputTokens,
            StatusLineItem::TotalOutputTokens => StatusSurfacePreviewItem::TotalOutputTokens,
            StatusLineItem::SessionId => StatusSurfacePreviewItem::SessionId,
            StatusLineItem::FastMode => StatusSurfacePreviewItem::FastMode,
            StatusLineItem::RawOutput => StatusSurfacePreviewItem::RawOutput,
            StatusLineItem::ThreadTitle => StatusSurfacePreviewItem::ThreadTitle,
            StatusLineItem::WorkspaceHeadline => StatusSurfacePreviewItem::WorkspaceHeadline,
            StatusLineItem::TaskProgress => StatusSurfacePreviewItem::TaskProgress,
        }
    }
}

/// 用于配置状态行中显示哪些条目的交互式视图。
///
/// 包装一个 [`MultiSelectPicker`] 并附加状态行特定的行为：
/// - 从当前配置预填充条目
/// - 显示配置的状态行的实时预览
/// - 确认时发出 [`AppEvent::StatusLineSetup`]
/// - 取消时发出 [`AppEvent::StatusLineSetupCancelled`]
pub(crate) struct StatusLineSetupView {
    /// 底层的多选选择器组件。
    picker: MultiSelectPicker,
}

impl StatusLineSetupView {
    /// 创建一个新的状态行设置视图。
    ///
    /// # 参数
    ///
    /// * `status_line_items` - 当前配置的条目 ID（按显示顺序），
    ///   或 `None` 以所有条目禁用开始
    /// * `use_theme_colors` - 预览和保存的状态行是否使用活动
    ///   主题的颜色
    /// * `app_event_tx` - 用于分派配置更改的事件发送器
    ///
    /// 来自 `status_line_items` 的条目先显示（按顺序）并标记为
    /// 启用。其余条目被追加并标记为禁用。
    pub(crate) fn new(
        status_line_items: Option<&[String]>,
        use_theme_colors: bool,
        preview_data: StatusSurfacePreviewData,
        app_event_tx: AppEventSender,
        list_keymap: ListKeymap,
    ) -> Self {
        let mut used_ids = HashSet::new();
        let mut items = vec![MultiSelectItem {
            id: STATUS_LINE_USE_THEME_COLORS_ITEM_ID.to_string(),
            name: "Use theme colors".to_string(),
            description: Some("Apply colors from the active /theme".to_string()),
            enabled: use_theme_colors,
            orderable: false,
            section_break_after: true,
        }];

        if let Some(selected_items) = status_line_items.as_ref() {
            for id in *selected_items {
                let Ok(item) = id.parse::<StatusLineItem>() else {
                    continue;
                };
                let item_id = item.to_string();
                if !used_ids.insert(item_id.clone()) {
                    continue;
                }
                items.push(Self::status_line_select_item(
                    item,
                    /*enabled*/ true,
                    &preview_data,
                ));
            }
        }

        for item in StatusLineItem::iter() {
            let item_id = item.to_string();
            if used_ids.contains(&item_id) {
                continue;
            }
            items.push(Self::status_line_select_item(
                item,
                /*enabled*/ false,
                &preview_data,
            ));
        }

        Self {
            picker: MultiSelectPicker::builder(
                "Configure Status Line".to_string(),
                Some("Select which items to display in the status line.".to_string()),
                app_event_tx,
            )
            .list_keymap(list_keymap)
            .items(items)
            .enable_ordering()
            .on_preview(move |items| {
                let use_theme_colors = items
                    .iter()
                    .find(|item| item.id == STATUS_LINE_USE_THEME_COLORS_ITEM_ID)
                    .map(|item| item.enabled)
                    .unwrap_or(true);
                preview_data.status_line_for_items(
                    items
                        .iter()
                        .filter(|item| item.enabled)
                        .filter_map(|item| item.id.parse::<StatusLineItem>().ok()),
                    use_theme_colors,
                )
            })
            .on_confirm(|ids, app_event| {
                let use_theme_colors = ids
                    .iter()
                    .any(|id| id == STATUS_LINE_USE_THEME_COLORS_ITEM_ID);
                let items = ids
                    .iter()
                    .filter_map(|id| id.parse::<StatusLineItem>().ok())
                    .collect::<Vec<_>>();
                app_event.send(AppEvent::StatusLineSetup {
                    items,
                    use_theme_colors,
                });
            })
            .on_cancel(|app_event| {
                app_event.send(AppEvent::StatusLineSetupCancelled);
            })
            .build(),
        }
    }

    /// 将 [`StatusLineItem`] 转换为选择器用的 [`MultiSelectItem`]。
    fn status_line_select_item(
        item: StatusLineItem,
        enabled: bool,
        preview_data: &StatusSurfacePreviewData,
    ) -> MultiSelectItem {
        let default_name = item.to_string();
        let default_description = item.description();
        let (name, description) = match item {
            StatusLineItem::FiveHourLimit | StatusLineItem::WeeklyLimit => (
                preview_data.rate_limit_item_name(item.preview_item(), &default_name),
                preview_data.rate_limit_item_description(item.preview_item(), default_description),
            ),
            _ => (default_name, default_description.to_string()),
        };

        MultiSelectItem {
            id: item.to_string(),
            name,
            description: Some(description),
            enabled,
            orderable: true,
            section_break_after: false,
        }
    }
}

impl BottomPaneView for StatusLineSetupView {
    fn handle_key_event(&mut self, key_event: crossterm::event::KeyEvent) {
        self.picker.handle_key_event(key_event);
    }

    fn is_complete(&self) -> bool {
        self.picker.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.picker.close();
        CancellationEvent::Handled
    }
}

impl Renderable for StatusLineSetupView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.picker.render(area, buf)
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.picker.desired_height(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_core::app_event_sender::AppEventSender;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::text::Line;
    use tokio::sync::mpsc::unbounded_channel;

    use crate::tui_core::app_event::AppEvent;

    #[test]
    fn context_used_accepts_context_usage_legacy_id() {
        assert_eq!(StatusLineItem::ContextUsed.to_string(), "context-used");
        assert_eq!(
            "context-used".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ContextUsed)
        );
        assert_eq!(
            "context-usage".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ContextUsed)
        );
    }

    #[test]
    fn context_remaining_is_selectable_id() {
        assert_eq!(
            "context-remaining".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ContextRemaining)
        );
        assert_eq!(
            StatusLineItem::ContextRemaining.to_string(),
            "context-remaining"
        );
    }
    #[test]
    fn project_name_is_canonical_and_accepts_legacy_ids() {
        assert_eq!(StatusLineItem::ProjectRoot.to_string(), "project-name");
        assert_eq!(
            "project-name".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ProjectRoot)
        );
        assert_eq!(
            "project".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ProjectRoot)
        );
        assert_eq!(
            "project-root".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ProjectRoot)
        );
    }

    #[test]
    fn model_is_canonical_and_accepts_model_name_legacy_id() {
        assert_eq!(StatusLineItem::ModelName.to_string(), "model");
        assert_eq!(
            "model".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ModelName)
        );
        assert_eq!(
            "model-name".parse::<StatusLineItem>(),
            Ok(StatusLineItem::ModelName)
        );
    }

    #[test]
    fn reasoning_is_selectable_id() {
        assert_eq!(StatusLineItem::Reasoning.to_string(), "reasoning");
        assert_eq!(
            "reasoning".parse::<StatusLineItem>(),
            Ok(StatusLineItem::Reasoning)
        );
    }

    #[test]
    fn run_state_is_canonical_and_accepts_status_legacy_id() {
        assert_eq!(StatusLineItem::Status.to_string(), "run-state");
        assert_eq!(
            "run-state".parse::<StatusLineItem>(),
            Ok(StatusLineItem::Status)
        );
        assert_eq!(
            "status".parse::<StatusLineItem>(),
            Ok(StatusLineItem::Status)
        );
    }

    #[test]
    fn git_summary_items_are_selectable_ids() {
        assert_eq!(
            "pull-request-number".parse::<StatusLineItem>(),
            Ok(StatusLineItem::PullRequestNumber)
        );
        assert_eq!(
            "branch-changes".parse::<StatusLineItem>(),
            Ok(StatusLineItem::BranchChanges)
        );
    }

    #[test]
    fn parse_status_line_items_accepts_title_only_variants() {
        let items = ["run-state", "task-progress"]
            .into_iter()
            .map(str::parse::<StatusLineItem>)
            .collect::<Result<Vec<_>, _>>();
        assert_eq!(
            items,
            Ok(vec![StatusLineItem::Status, StatusLineItem::TaskProgress,])
        );
    }

    #[test]
    fn preview_uses_runtime_values() {
        let preview_data = StatusSurfacePreviewData::from_iter([
            (
                StatusLineItem::ModelName.preview_item(),
                "gpt-5".to_string(),
            ),
            (
                StatusLineItem::CurrentDir.preview_item(),
                "/repo".to_string(),
            ),
        ]);
        let items = [
            MultiSelectItem {
                id: StatusLineItem::ModelName.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
            MultiSelectItem {
                id: StatusLineItem::CurrentDir.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
        ];

        assert_eq!(
            line_text(
                preview_data.status_line_for_items(
                    items
                        .iter()
                        .filter_map(|item| item.id.parse::<StatusLineItem>().ok()),
                    /*use_theme_colors*/ true,
                )
            ),
            Some("gpt-5 · /repo".to_string())
        );
    }

    #[test]
    fn preview_uses_placeholders_when_runtime_values_are_missing() {
        let preview_data = StatusSurfacePreviewData::from_iter([(
            StatusSurfacePreviewItem::Model,
            "gpt-5".to_string(),
        )]);
        let items = [
            MultiSelectItem {
                id: StatusLineItem::ModelName.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
            MultiSelectItem {
                id: StatusLineItem::GitBranch.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
        ];

        assert_eq!(
            line_text(
                preview_data.status_line_for_items(
                    items
                        .iter()
                        .filter_map(|item| item.id.parse::<StatusLineItem>().ok()),
                    /*use_theme_colors*/ true,
                )
            ),
            Some("gpt-5 · feat/awesome-feature".to_string())
        );
    }

    #[test]
    fn preview_includes_thread_title() {
        let preview_data = StatusSurfacePreviewData::from_iter([
            (
                StatusLineItem::ModelName.preview_item(),
                "gpt-5".to_string(),
            ),
            (
                StatusLineItem::ThreadTitle.preview_item(),
                "Roadmap cleanup".to_string(),
            ),
        ]);
        let items = [
            MultiSelectItem {
                id: StatusLineItem::ModelName.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
            MultiSelectItem {
                id: StatusLineItem::ThreadTitle.to_string(),
                name: String::new(),
                description: None,
                enabled: true,
                orderable: true,
                section_break_after: false,
            },
        ];

        assert_eq!(
            line_text(
                preview_data.status_line_for_items(
                    items
                        .iter()
                        .filter_map(|item| item.id.parse::<StatusLineItem>().ok()),
                    /*use_theme_colors*/ true,
                )
            ),
            Some("gpt-5 · Roadmap cleanup".to_string())
        );
    }

    #[test]
    fn setup_view_snapshot_uses_runtime_preview_values() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let view = StatusLineSetupView::new(
            Some(&[
                StatusLineItem::ModelName.to_string(),
                StatusLineItem::CurrentDir.to_string(),
                StatusLineItem::GitBranch.to_string(),
            ]),
            /*use_theme_colors*/ true,
            StatusSurfacePreviewData::from_iter([
                (
                    StatusLineItem::ModelName.preview_item(),
                    "gpt-5-pro".to_string(),
                ),
                (
                    StatusLineItem::CurrentDir.preview_item(),
                    "~/reflect-agent".to_string(),
                ),
                (
                    StatusLineItem::GitBranch.preview_item(),
                    "jif/statusline-preview".to_string(),
                ),
                (
                    StatusLineItem::WeeklyLimit.preview_item(),
                    "weekly 82% left".to_string(),
                ),
            ]),
            AppEventSender::new(tx_raw),
            crate::tui_core::keymap::RuntimeKeymap::defaults().list,
        );

        assert_snapshot!(render_lines(&view, /*width*/ 72));
    }

    fn render_lines(view: &StatusLineSetupView, width: u16) -> String {
        let height = view.desired_height(width);
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        (0..area.height)
            .map(|row| {
                let mut line = String::new();
                for col in 0..area.width {
                    let symbol = buf[(area.x + col, area.y + row)].symbol();
                    if symbol.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(symbol);
                    }
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn line_text(line: Option<Line<'static>>) -> Option<String> {
        line.map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
    }
}
