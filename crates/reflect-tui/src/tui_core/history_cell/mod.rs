//! Reflect TUI 的转录/历史单元。
//!
//! `HistoryCell` 是会话 UI 中显示的最小单元，既表示已提交的转录条目，
//! 也临时表示一个在流式传输期间可原地变更的进行中活动单元。
//!
//! 转录浮层（`Ctrl+T`）会追加从活动单元派生的缓存实时尾部，
//! 并且该缓存尾部会基于活动单元缓存键进行刷新。
//! 那些随时间流逝而变化的单元会暴露 `transcript_animation_tick()`，
//! 而原地变更活动单元的代码会增加 `ChatWidget` 所追踪的活动单元版本号，
//! 因此每当渲染的转录输出可能变化时，缓存键都会随之改变。

use crate::app_server_protocol::AskForApproval;
use crate::app_server_protocol::McpAuthStatus;
use crate::app_server_protocol::McpServerStatus;
use crate::app_server_protocol::McpServerStatusDetail;
use crate::app_server_protocol::ToolRequestUserInputAnswer;
use crate::app_server_protocol::ToolRequestUserInputQuestion;
use crate::app_server_protocol::WebSearchAction;
#[cfg(test)]
use crate::config_compat::types::McpServerTransportConfig;
#[cfg(test)]
use crate::mcp::qualified_mcp_tool_name_prefix;
use crate::otel::RuntimeMetricsSummary;
use crate::protocol_compat::account::PlanType;
use crate::protocol_compat::approvals::ExecPolicyAmendment;
use crate::protocol_compat::approvals::NetworkPolicyAmendment;
#[cfg(test)]
use crate::protocol_compat::mcp::Resource;
#[cfg(test)]
use crate::protocol_compat::mcp::ResourceTemplate;
use crate::protocol_compat::models::ManagedFileSystemPermissions;
use crate::protocol_compat::models::PermissionProfile;
use crate::protocol_compat::models::local_image_label_text;
use crate::protocol_compat::openai_models::ReasoningEffort as ReasoningEffortConfig;
use crate::protocol_compat::permissions::NetworkSandboxPolicy;
use crate::protocol_compat::plan_tool::PlanItemArg;
use crate::protocol_compat::plan_tool::StepStatus;
use crate::protocol_compat::plan_tool::UpdatePlanArgs;
use crate::protocol_compat::user_input::TextElement;
use crate::tui_core::diff_model::FileChange;
use crate::tui_core::diff_render::create_diff_summary;
use crate::tui_core::diff_render::display_path_for;
use crate::tui_core::exec_cell::CommandOutput;
use crate::tui_core::exec_cell::OutputLinesParams;
use crate::tui_core::exec_cell::TOOL_CALL_MAX_LINES;
use crate::tui_core::exec_cell::output_lines;
use crate::tui_core::exec_command::relativize_to_home;
use crate::tui_core::exec_command::strip_bash_lc_and_escape;
use crate::tui_core::legacy_core::config::Config;
use crate::tui_core::live_wrap::take_prefix_by_width;
use crate::tui_core::markdown::append_markdown;
use crate::tui_core::motion::MotionMode;
use crate::tui_core::motion::ReducedMotionIndicator;
use crate::tui_core::motion::activity_indicator;
use crate::tui_core::render::line_utils::line_to_static;
use crate::tui_core::render::line_utils::prefix_lines;
use crate::tui_core::render::line_utils::push_owned_lines;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::session_state::ThreadSessionState;
use crate::tui_core::style::user_message_style;
use crate::tui_core::terminal_hyperlinks::HyperlinkLine;
use crate::tui_core::terminal_hyperlinks::mark_buffer_hyperlinks;
use crate::tui_core::terminal_hyperlinks::plain_hyperlink_lines;
use crate::tui_core::terminal_hyperlinks::prefix_hyperlink_lines;
use crate::tui_core::terminal_hyperlinks::visible_lines;
use crate::tui_core::terminal_hyperlinks::visible_lines_ref;
#[cfg(all(test, feature = "tui-upstream-tests"))]
use crate::tui_core::test_support::PathBufExt;
#[cfg(all(test, feature = "tui-upstream-tests"))]
use crate::tui_core::test_support::test_path_buf;
use crate::tui_core::text_formatting::format_and_truncate_tool_result;
use crate::tui_core::text_formatting::truncate_text;
use crate::tui_core::tooltips;
use crate::tui_core::ui_consts::LIVE_PREFIX_COLS;
use crate::tui_core::update_action::UpdateAction;
use crate::tui_core::version::REFLECT_CLI_VERSION;
use crate::tui_core::wrapping::RtOptions;
use crate::tui_core::wrapping::adaptive_wrap_line;
use crate::tui_core::wrapping::adaptive_wrap_lines;
use crate::utils_absolute_path::AbsolutePathBuf;
#[cfg(test)]
use crate::utils_cli::format_env_display;
use base64::Engine;
use image::DynamicImage;
use image::ImageReader;
use ratatui::prelude::*;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Styled;
use ratatui::style::Stylize;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::any::Any;
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tracing::error;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use url::Url;

const RAW_DIFF_SUMMARY_WIDTH: usize = 10_000;
const RAW_TOOL_OUTPUT_WIDTH: usize = 10_000;

mod approvals;
mod base;
mod exec;
mod hook_cell;
mod markdown_render_cache;
mod mcp;
mod messages;
mod notices;
mod patches;
mod plans;
mod request_user_input;
mod search;
mod separators;
mod session;

pub(crate) use approvals::*;
pub(crate) use base::*;
pub(crate) use exec::*;
pub(crate) use hook_cell::HookCell;
pub(crate) use hook_cell::new_active_hook_cell;
pub(crate) use hook_cell::new_completed_hook_cell;
pub(crate) use mcp::*;
pub(crate) use messages::*;
pub(crate) use notices::*;
pub(crate) use patches::*;
pub(crate) use plans::*;
pub(crate) use request_user_input::*;
pub(crate) use search::*;
pub(crate) use separators::*;
pub(crate) use session::*;

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryRenderMode {
    Rich,
    Raw,
}

pub(crate) fn raw_lines_from_source(source: &str) -> Vec<Line<'static>> {
    if source.is_empty() {
        return Vec::new();
    }

    let mut parts = source.split('\n').collect::<Vec<_>>();
    if source.ends_with('\n') {
        parts.pop();
    }

    parts
        .into_iter()
        .map(|line| Line::from(line.to_string()))
        .collect()
}

pub(crate) fn plain_lines(lines: impl IntoIterator<Item = Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let text = line
                .spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>();
            Line::from(text)
        })
        .collect()
}

/// 会话历史的单个可渲染单元。
///
/// 每个单元都会生成逻辑 `Line`，并报告这些行在给定终端宽度下占用的视口行数。
/// 默认的高度实现使用 `Paragraph::wrap` 来考虑超出视口宽度的行
/// （例如被自适应换行保持完整的长 URL）。具体类型仅在需要应用超出
/// `Paragraph::line_count` 范围之外的额外布局逻辑时才需要重写高度。
pub(crate) trait HistoryCell: std::fmt::Debug + Send + Sync + Any {
    /// 返回主聊天视口的逻辑行。
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

    /// 返回用于原始回滚模式的、便于复制的纯逻辑行。
    fn raw_lines(&self) -> Vec<Line<'static>>;

    /// 返回富文本可见行以及终端超链接元数据。
    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.display_lines(width))
    }

    fn display_lines_for_mode(&self, width: u16, mode: HistoryRenderMode) -> Vec<Line<'static>> {
        match mode {
            HistoryRenderMode::Rich => visible_lines(self.display_hyperlink_lines(width)),
            HistoryRenderMode::Raw => self.raw_lines(),
        }
    }

    fn display_hyperlink_lines_for_mode(
        &self,
        width: u16,
        mode: HistoryRenderMode,
    ) -> Vec<HyperlinkLine> {
        match mode {
            HistoryRenderMode::Rich => self.display_hyperlink_lines(width),
            HistoryRenderMode::Raw => plain_hyperlink_lines(self.raw_lines()),
        }
    }

    /// 返回渲染该单元所需的视口行数。
    ///
    /// 默认实现会委托给 `Paragraph::line_count` 并使用 `Wrap { trim: false }`，
    /// 用于度量在 ratatui 视口级字符折行后的实际行数。这对于包含宽度超出终端的
    /// 类 URL 标记的行至关重要——仅靠逻辑行计数会得出偏小的结果。
    fn desired_height(&self, width: u16) -> u16 {
        self.desired_height_for_mode(width, HistoryRenderMode::Rich)
    }

    fn desired_height_for_mode(&self, width: u16, mode: HistoryRenderMode) -> u16 {
        Paragraph::new(Text::from(self.display_lines_for_mode(width, mode)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

    /// 返回转录浮层（`Ctrl+T`）所使用的行。
    ///
    /// 默认为 `display_lines`。当转录表示与显示不同时需重写
    /// （例如 `ExecCell` 会展示所有以 `$` 开头的命令调用及其退出状态）。
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.display_lines(width)
    }

    /// 返回转录浮层行以及终端超链接元数据。
    ///
    /// 默认为纯转录表示，因为某些单元的显示内容和转录内容不同。
    /// 转录内容与显示一致的富文本单元应委托给 `display_hyperlink_lines`。
    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.transcript_lines(width))
    }

    /// 返回转录浮层所需的视口行数。
    ///
    /// 使用与 `desired_height` 相同的 `Paragraph::line_count` 度量方式。
    /// 包含一个针对其 bug 的变通方案：在 ratatui 中，单个仅包含空白字符的行
    /// 会返回 2 行而不是 1 行。
    fn desired_transcript_height(&self, width: u16) -> u16 {
        let lines = visible_lines(self.transcript_hyperlink_lines(width));
        // 变通方案：ratatui 的 line_count 对单个仅含空白字符的行会返回 2。
        // 在这种情况下将其夹紧为 1。
        if let [line] = &lines[..]
            && line
                .spans
                .iter()
                .all(|s| s.content.chars().all(char::is_whitespace))
        {
            return 1;
        }

        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

/// 指示在后续浮层渲染中转录高度是否仍然有效。
///
/// 由外部状态支持的单元应返回 `false`，以便分页器在渲染前重新测量它们，
/// 而不是复用可能会裁剪其内容的高度。
    fn has_stable_transcript_height(&self) -> bool {
        true
    }

    fn is_stream_continuation(&self) -> bool {
        false
    }

/// 当转录输出与时间相关时，返回一个粗粒度的“动画刻度”。
///
/// 转录浮层会缓存进行中活动单元的渲染输出，
/// 因此包含时间相关 UI（旋转器、闪光等）的单元应返回一个会随时间变化的刻度，
/// 以提示应重新计算缓存的尾部。返回 `None` 表示转录行是稳定的，
/// 而在动画进行中返回 `Some(tick)` 可让浮层与主视口保持同步。
///
/// 如果单元使用时间相关的视觉效果但始终返回 `None`，`Ctrl+T` 可能会在
/// 首次渲染的帧上看起来“冻结”，即便主视口仍在动画。
    fn transcript_animation_tick(&self) -> Option<u64> {
        None
    }
}

impl Renderable for Box<dyn HistoryCell> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let hyperlink_lines = self.display_hyperlink_lines(area.width);
        let lines = visible_lines_ref(&hyperlink_lines);
        let paragraph = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
        let y = if area.height == 0 {
            0
        } else {
            let overflow = paragraph
                .line_count(area.width)
                .saturating_sub(usize::from(area.height));
            u16::try_from(overflow).unwrap_or(u16::MAX)
        };
        // 在调整大小或流更新期间，活动单元内容可能会发生剧烈重排。
        // 先清除整个绘制区域，以避免上一帧的陈旧字符残留。
        Clear.render(area, buf);
        paragraph.scroll((y, 0)).render(area, buf);
        mark_buffer_hyperlinks(buf, area, &hyperlink_lines, usize::from(y));
    }
    fn desired_height(&self, width: u16) -> u16 {
        HistoryCell::desired_height(self.as_ref(), width)
    }
}

impl dyn HistoryCell {
    pub(crate) fn as_any(&self) -> &dyn Any {
        self
    }

    pub(crate) fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
