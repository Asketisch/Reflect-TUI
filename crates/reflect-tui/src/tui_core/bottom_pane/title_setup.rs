//! 用于自定义终端窗口/标签页标题的终端标题配置视图。
//!
//! 本模块提供一个交互式选择器，用于选择哪些条目出现在
//! 终端标题中。用户可以：
//!
//! - 选择条目
//! - 调整条目顺序
//! - 预览渲染后的标题

use itertools::Itertools;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use strum::IntoEnumIterator;
use strum_macros::Display;
use strum_macros::EnumIter;
use strum_macros::EnumString;

use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::bottom_pane::ACTION_REQUIRED_PREVIEW_PREFIX;
use crate::tui_core::bottom_pane::CancellationEvent;
use crate::tui_core::bottom_pane::bottom_pane_view::BottomPaneView;
use crate::tui_core::bottom_pane::build_action_required_title_text;
use crate::tui_core::bottom_pane::multi_select_picker::MultiSelectItem;
use crate::tui_core::bottom_pane::multi_select_picker::MultiSelectPicker;
use crate::tui_core::bottom_pane::status_surface_preview::StatusSurfacePreviewData;
use crate::tui_core::bottom_pane::status_surface_preview::StatusSurfacePreviewItem;
use crate::tui_core::keymap::ListKeymap;
use crate::tui_core::render::renderable::Renderable;

/// 可显示在终端标题中的可用条目。
///
/// 各变体通过 strum 序列化为 kebab-case 标识符（例如 `AppName` -> `"app-name"`）。
/// 这些标识符会持久化到用户配置文件中，因此重命名
/// 或移除某个变体属于破坏性的配置变更。
#[derive(EnumIter, EnumString, Display, Debug, Clone, Copy, Eq, PartialEq, Hash)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum TerminalTitleItem {
    /// Reflect 应用名称。
    AppName,
    /// 项目根目录名称，或回退为紧凑的 cwd。
    #[strum(to_string = "project-name", serialize = "project")]
    Project,
    /// 当前工作目录路径。
    CurrentDir,
    /// 活动期间显示在终端标题中的活动指示器。
    #[strum(to_string = "activity", serialize = "spinner")]
    Spinner,
    /// 紧凑的运行时运行状态文本。
    #[strum(to_string = "run-state", serialize = "status")]
    Status,
    /// 当前线程标题（如可用）。
    #[strum(to_string = "thread-title", serialize = "thread")]
    Thread,
    /// 当前 git 分支（如可用）。
    GitBranch,
    /// 上下文窗口剩余百分比。
    ContextRemaining,
    /// 上下文窗口已用百分比。
    #[strum(to_string = "context-used", serialize = "context-usage")]
    ContextUsed,
    /// 主速率限制的剩余用量。
    FiveHourLimit,
    /// 次速率限制的剩余用量。
    WeeklyLimit,
    /// Reflect 应用版本。
    ReflectVersion,
    /// 当前会话使用的总 token 数。
    UsedTokens,
    /// 已消耗的总输入 token 数。
    TotalInputTokens,
    /// 已生成的总输出 token 数。
    TotalOutputTokens,
    /// 完整的线程 UUID。
    #[strum(to_string = "thread-id", serialize = "session-id")]
    SessionId,
    /// Fast 模式当前是否处于活动状态。
    FastMode,
    /// 当前模型名称。
    #[strum(to_string = "model", serialize = "model-name")]
    Model,
    /// 当前模型名称及推理级别。
    ModelWithReasoning,
    /// 当前推理级别。
    Reasoning,
    /// 来自 `update_plan` 的最新清单任务进度（如可用）。
    TaskProgress,
}

impl TerminalTitleItem {
    pub(crate) fn description(self) -> &'static str {
        match self {
            TerminalTitleItem::AppName => "Reflect app name",
            TerminalTitleItem::Project => "Project name (falls back to current directory name)",
            TerminalTitleItem::CurrentDir => "Current working directory",
            TerminalTitleItem::Spinner => {
                "Spinner while working, action-required message while blocked."
            }
            TerminalTitleItem::Status => {
                "Compact session run-state text (Ready, Working, Thinking)"
            }
            TerminalTitleItem::Thread => "Current thread title, or thread identifier when unnamed",
            TerminalTitleItem::GitBranch => "Current Git branch (omitted when unavailable)",
            TerminalTitleItem::ContextRemaining => {
                "Percentage of context window remaining (omitted when unknown)"
            }
            TerminalTitleItem::ContextUsed => {
                "Percentage of context window used (omitted when unknown)"
            }
            TerminalTitleItem::FiveHourLimit => {
                "Remaining usage on the primary usage limit (omitted when unavailable)"
            }
            TerminalTitleItem::WeeklyLimit => {
                "Remaining usage on the secondary usage limit (omitted when unavailable)"
            }
            TerminalTitleItem::ReflectVersion => "Reflect application version",
            TerminalTitleItem::UsedTokens => "Total tokens used in session (omitted when zero)",
            TerminalTitleItem::TotalInputTokens => "Total input tokens used in session",
            TerminalTitleItem::TotalOutputTokens => "Total output tokens used in session",
            TerminalTitleItem::SessionId => {
                "Current thread identifier (omitted until thread starts)"
            }
            TerminalTitleItem::FastMode => "Whether Fast mode is currently active",
            TerminalTitleItem::Model => "Current model name",
            TerminalTitleItem::ModelWithReasoning => "Current model name with reasoning level",
            TerminalTitleItem::Reasoning => "Current reasoning level",
            TerminalTitleItem::TaskProgress => {
                "Latest task progress from update_plan (omitted until available)"
            }
        }
    }

    pub(crate) fn preview_item(self) -> Option<StatusSurfacePreviewItem> {
        match self {
            TerminalTitleItem::AppName => Some(StatusSurfacePreviewItem::AppName),
            TerminalTitleItem::Project => Some(StatusSurfacePreviewItem::ProjectName),
            TerminalTitleItem::CurrentDir => Some(StatusSurfacePreviewItem::CurrentDir),
            TerminalTitleItem::Spinner => None,
            TerminalTitleItem::Status => Some(StatusSurfacePreviewItem::Status),
            TerminalTitleItem::Thread => Some(StatusSurfacePreviewItem::ThreadTitle),
            TerminalTitleItem::GitBranch => Some(StatusSurfacePreviewItem::GitBranch),
            TerminalTitleItem::ContextRemaining => Some(StatusSurfacePreviewItem::ContextRemaining),
            TerminalTitleItem::ContextUsed => Some(StatusSurfacePreviewItem::ContextUsed),
            TerminalTitleItem::FiveHourLimit => Some(StatusSurfacePreviewItem::FiveHourLimit),
            TerminalTitleItem::WeeklyLimit => Some(StatusSurfacePreviewItem::WeeklyLimit),
            TerminalTitleItem::ReflectVersion => Some(StatusSurfacePreviewItem::ReflectVersion),
            TerminalTitleItem::UsedTokens => Some(StatusSurfacePreviewItem::UsedTokens),
            TerminalTitleItem::TotalInputTokens => Some(StatusSurfacePreviewItem::TotalInputTokens),
            TerminalTitleItem::TotalOutputTokens => {
                Some(StatusSurfacePreviewItem::TotalOutputTokens)
            }
            TerminalTitleItem::SessionId => Some(StatusSurfacePreviewItem::SessionId),
            TerminalTitleItem::FastMode => Some(StatusSurfacePreviewItem::FastMode),
            TerminalTitleItem::Model => Some(StatusSurfacePreviewItem::Model),
            TerminalTitleItem::ModelWithReasoning => {
                Some(StatusSurfacePreviewItem::ModelWithReasoning)
            }
            TerminalTitleItem::Reasoning => Some(StatusSurfacePreviewItem::Reasoning),
            TerminalTitleItem::TaskProgress => Some(StatusSurfacePreviewItem::TaskProgress),
        }
    }

    /// 返回在渲染标题中置于该条目之前的分隔符。
    ///
    /// 活动指示器两侧使用普通空格，使其呈现为
    /// `my-project <activity> Working` 而非 `my-project | <activity> | Working`。
    /// 其余所有相邻条目之间均以 ` | ` 连接。
    pub(crate) fn separator_from_previous(self, previous: Option<Self>) -> &'static str {
        match previous {
            None => "",
            Some(previous)
                if previous == TerminalTitleItem::Spinner || self == TerminalTitleItem::Spinner =>
            {
                " "
            }
            Some(_) => " | ",
        }
    }
}

pub(crate) fn preview_line_for_title_items(
    items: &[TerminalTitleItem],
    preview_data: &StatusSurfacePreviewData,
) -> Option<Line<'static>> {
    if items.contains(&TerminalTitleItem::Spinner) {
        let preview = build_action_required_title_text(
            ACTION_REQUIRED_PREVIEW_PREFIX,
            items.iter().copied(),
            &[],
            |item| {
                item.preview_item()
                    .and_then(|preview_item| preview_data.value_for(preview_item))
                    .map(str::to_owned)
            },
        );
        return Some(Line::from(preview));
    }

    let mut previous = None;
    let preview = items
        .iter()
        .copied()
        .fold(String::new(), |mut preview, item| {
            let Some(value) = item
                .preview_item()
                .and_then(|preview_item| preview_data.value_for(preview_item))
            else {
                return preview;
            };
            preview.push_str(item.separator_from_previous(previous));
            preview.push_str(value);
            previous = Some(item);
            preview
        });
    if preview.is_empty() {
        None
    } else {
        Some(Line::from(preview))
    }
}

fn parse_terminal_title_items<T>(ids: impl Iterator<Item = T>) -> Option<Vec<TerminalTitleItem>>
where
    T: AsRef<str>,
{
    // 将解析视为全有或全无，使预览/确认回调绝不会输出
    // 仅被部分解释的排序。构建选择器时会忽略无效 id，
    // 但一旦用户开始与选择器交互，我们只希望
    // 持久化或预览完全有效的选择。
    ids.map(|id| id.as_ref().parse::<TerminalTitleItem>())
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

/// 用于配置终端标题条目的交互式视图。
pub(crate) struct TerminalTitleSetupView {
    picker: MultiSelectPicker,
}

impl TerminalTitleSetupView {
    /// 创建终端标题选择器，优先保留已配置的条目顺序。
    ///
    /// 未知的已配置 id 在此处会被跳过而非内联提示。
    /// 主 TUI 在渲染实际标题时仍会对它们发出警告，但
    /// 选择器本身只公开它能够有意义地预览
    /// 和持久化的可选条目。
    pub(crate) fn new(
        title_items: Option<&[String]>,
        preview_data: StatusSurfacePreviewData,
        app_event_tx: AppEventSender,
        list_keymap: ListKeymap,
    ) -> Self {
        let selected_items = title_items
            .into_iter()
            .flatten()
            .filter_map(|id| id.parse::<TerminalTitleItem>().ok())
            .unique()
            .collect_vec();
        let selected_set = selected_items
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let items = selected_items
            .into_iter()
            .map(|item| Self::title_select_item(item, /*enabled*/ true, &preview_data))
            .chain(
                TerminalTitleItem::iter()
                    .filter(|item| !selected_set.contains(item))
                    .map(|item| {
                        Self::title_select_item(item, /*enabled*/ false, &preview_data)
                    }),
            )
            .collect();

        Self {
            picker: MultiSelectPicker::builder(
                "Configure Terminal Title".to_string(),
                Some("Select which items to display in the terminal title.".to_string()),
                app_event_tx,
            )
            .list_keymap(list_keymap)
            .items(items)
            .enable_ordering()
            .on_preview(move |items| {
                let items = parse_terminal_title_items(
                    items
                        .iter()
                        .filter(|item| item.enabled)
                        .map(|item| item.id.as_str()),
                )?;
                preview_line_for_title_items(&items, &preview_data)
            })
            .on_change(|items, app_event| {
                let Some(items) = parse_terminal_title_items(
                    items
                        .iter()
                        .filter(|item| item.enabled)
                        .map(|item| item.id.as_str()),
                ) else {
                    return;
                };
                app_event.send(AppEvent::TerminalTitleSetupPreview { items });
            })
            .on_confirm(|ids, app_event| {
                let Some(items) = parse_terminal_title_items(ids.iter().map(String::as_str)) else {
                    return;
                };
                app_event.send(AppEvent::TerminalTitleSetup { items });
            })
            .on_cancel(|app_event| {
                app_event.send(AppEvent::TerminalTitleSetupCancelled);
            })
            .build(),
        }
    }

    fn title_select_item(
        item: TerminalTitleItem,
        enabled: bool,
        preview_data: &StatusSurfacePreviewData,
    ) -> MultiSelectItem {
        let default_name = item.to_string();
        let default_description = item.description();
        let (name, description) = match item.preview_item() {
            Some(
                preview_item @ (StatusSurfacePreviewItem::FiveHourLimit
                | StatusSurfacePreviewItem::WeeklyLimit),
            ) => (
                preview_data.rate_limit_item_name(preview_item, &default_name),
                preview_data.rate_limit_item_description(preview_item, default_description),
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

impl BottomPaneView for TerminalTitleSetupView {
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

impl Renderable for TerminalTitleSetupView {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.picker.render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.picker.desired_height(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use tokio::sync::mpsc::unbounded_channel;

    fn render_lines(view: &TerminalTitleSetupView, width: u16) -> String {
        let height = view.desired_height(width);
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        view.render(area, &mut buf);

        let lines: Vec<String> = (0..area.height)
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
            .collect();
        lines.join("\n")
    }

    #[test]
    fn renders_title_setup_popup() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let selected = [
            "project-name".to_string(),
            "activity".to_string(),
            "run-state".to_string(),
            "thread-title".to_string(),
        ];
        let view = TerminalTitleSetupView::new(
            Some(&selected),
            StatusSurfacePreviewData::default(),
            tx,
            crate::tui_core::keymap::RuntimeKeymap::defaults().list,
        );
        assert_snapshot!(
            "terminal_title_setup_basic",
            render_lines(&view, /*width*/ 84)
        );
    }

    #[test]
    fn parse_terminal_title_items_preserves_order() {
        let items = parse_terminal_title_items(
            ["project-name", "activity", "run-state", "thread-title"].into_iter(),
        );
        assert_eq!(
            items,
            Some(vec![
                TerminalTitleItem::Project,
                TerminalTitleItem::Spinner,
                TerminalTitleItem::Status,
                TerminalTitleItem::Thread,
            ])
        );
    }

    #[test]
    fn parse_terminal_title_items_rejects_invalid_ids() {
        let items = parse_terminal_title_items(["project", "not-a-title-item"].into_iter());
        assert_eq!(items, None);
    }

    #[test]
    fn activity_is_canonical_and_accepts_spinner_legacy_id() {
        assert_eq!(TerminalTitleItem::Spinner.to_string(), "activity");
        assert_eq!(
            "activity".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Spinner)
        );
        assert_eq!(
            "spinner".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Spinner)
        );
    }

    #[test]
    fn project_name_is_canonical_and_accepts_project_legacy_id() {
        assert_eq!(TerminalTitleItem::Project.to_string(), "project-name");
        assert_eq!(
            "project-name".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Project)
        );
        assert_eq!(
            "project".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Project)
        );
    }

    #[test]
    fn thread_title_is_canonical_and_accepts_thread_legacy_id() {
        assert_eq!(TerminalTitleItem::Thread.to_string(), "thread-title");
        assert_eq!(
            "thread-title".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Thread)
        );
        assert_eq!(
            "thread".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Thread)
        );
    }

    #[test]
    fn model_is_canonical_and_accepts_model_name_legacy_id() {
        assert_eq!(TerminalTitleItem::Model.to_string(), "model");
        assert_eq!(
            "model".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Model)
        );
        assert_eq!(
            "model-name".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Model)
        );
    }

    #[test]
    fn run_state_is_canonical_and_accepts_status_legacy_id() {
        assert_eq!(TerminalTitleItem::Status.to_string(), "run-state");
        assert_eq!(
            "run-state".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Status)
        );
        assert_eq!(
            "status".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Status)
        );
    }

    #[test]
    fn model_with_reasoning_has_distinct_id() {
        assert_eq!(
            TerminalTitleItem::ModelWithReasoning.to_string(),
            "model-with-reasoning"
        );
        assert_eq!(
            "model-with-reasoning".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::ModelWithReasoning)
        );
    }

    #[test]
    fn reasoning_is_selectable_id() {
        assert_eq!(TerminalTitleItem::Reasoning.to_string(), "reasoning");
        assert_eq!(
            "reasoning".parse::<TerminalTitleItem>(),
            Ok(TerminalTitleItem::Reasoning)
        );
    }

    #[test]
    fn parse_terminal_title_items_accepts_kebab_case_variants() {
        let items = parse_terminal_title_items(
            [
                "app-name",
                "context-remaining",
                "context-used",
                "five-hour-limit",
                "git-branch",
                "activity",
                "current-dir",
                "project-name",
                "model",
                "model-with-reasoning",
                "reasoning",
                "weekly-limit",
                "reflect-version",
                "used-tokens",
                "total-input-tokens",
                "total-output-tokens",
                "session-id",
                "fast-mode",
            ]
            .into_iter(),
        );
        assert_eq!(
            items,
            Some(vec![
                TerminalTitleItem::AppName,
                TerminalTitleItem::ContextRemaining,
                TerminalTitleItem::ContextUsed,
                TerminalTitleItem::FiveHourLimit,
                TerminalTitleItem::GitBranch,
                TerminalTitleItem::Spinner,
                TerminalTitleItem::CurrentDir,
                TerminalTitleItem::Project,
                TerminalTitleItem::Model,
                TerminalTitleItem::ModelWithReasoning,
                TerminalTitleItem::Reasoning,
                TerminalTitleItem::WeeklyLimit,
                TerminalTitleItem::ReflectVersion,
                TerminalTitleItem::UsedTokens,
                TerminalTitleItem::TotalInputTokens,
                TerminalTitleItem::TotalOutputTokens,
                TerminalTitleItem::SessionId,
                TerminalTitleItem::FastMode,
            ])
        );
    }
}
