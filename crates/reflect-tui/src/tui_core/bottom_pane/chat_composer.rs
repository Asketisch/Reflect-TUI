//! ChatComposer 聊天编辑器组件。
//!
//! 注：本文件超过 800 行红线，但 `impl ChatComposer`（约 3800 行）为内聚的完整状态机
//! （输入处理/mention/popup 渲染/历史/slash 命令），与上游逐行对应，
//! 按 AGENTS.md「内聚完整状态机例外」保留。
//!
//! 聊天编辑器是底部面板的文本输入状态机，负责：
//!
//! - 编辑输入缓冲区（一个 [`TextArea`]），包括附件的占位符 "元素"。
//! - 将按键路由到活动弹窗（slash 命令、文件搜索、技能/应用提及）。
//! - 在命令名完成时，将输入的 slash 命令提升为原子元素。
//! - 处理 Enter 键的提交与换行逻辑。
//! - 将原始按键流转换为显式的粘贴操作（尤其在终端不提供可靠 bracketed paste 的平台上，如 Windows）。
//!
//! # 按键事件路由
//!
//! 大部分按键处理通过 [`ChatComposer::handle_key_event`]，如果弹窗可见则分派到弹窗特定处理器，
//! 否则分派到 [`ChatComposer::handle_key_event_without_popup`]。每次按键处理后，调用
//! [`ChatComposer::sync_popups`] 使 UI 状态跟随最新的缓冲区/光标。
//!
//! # 补全与弹窗关闭
//!
//! 弹窗目标解析解析光标周围的可编辑标记范围，并将原子文本元素视为硬边界。当该范围紧邻原子元素之后时，
//! 插入路径首先添加水平分隔符并偏移范围，使补全不能与原子元素合并。替换范围后，补全将光标留在一
//! 个水平分隔符之后。没有后缀可合并时，它跨过现有分隔符前进；否则插入空格，保留非空白后缀前的
//! 现有分隔符。遇到换行时也会插入空格而不是跨行。
//!
//! `Esc` 记录活动标记为已取消。以 `@` 或 `$` 开头的已完成值在弹窗同步前也会重新取消，因为分隔符
//! 亲和性仍能将光标左侧的补全标记识别为完成标记。同步仅在查询、完整标记文本和匹配出现中的序数
//! 保持不变时才保持弹窗隐藏。空白和原子元素边界限定出现，而嵌套符号则不。这样可在仅偏移编辑时
//! 保持取消状态，同时不抑制后续相同标记。
//!
//! Slash 命令的取消状态单独追踪。在命令弹窗活动时按 `Esc` 记录首行 `/name` 标记，并在此标记
//! 不变时保持弹窗关闭。编辑命令标记会清除取消状态，允许弹窗重新打开。
//!
//! # 历史导航（↑/↓）
//!
//! 上/下历史路径由 [`ChatComposerHistory`] 管理，它合并：
//!
//! - 持久化跨会话历史（纯文本；无元素范围或附件）。
//! - 本地会话内历史（完整文本 + 文本元素 + 本地/远程图像附件）。
//!
//! 当回忆本地条目时，编辑器重新注入文本元素和两种附件（本地图像路径 + 远程图像 URL）。
//! 当回忆持久化条目时，仅恢复文本。
//! 回忆的条目将光标移到行尾，使重复上/下按键保持 shell 式历史遍历语义，而不是掉落到列 0。
//! `Ctrl+R` 打开反向增量搜索模式。页脚变为搜索输入；查询非空时，编辑器主体预览当前匹配。
//! `Enter` 接受预览为可编辑草稿，`Esc` 恢复搜索激活时的草稿。
//!
//! Slash 命令在本地历史中暂存而不是立即记录。命令回忆是两阶段移交：在此处暂存提交的 slash 文本，
//! 然后在 `ChatWidget` 分派命令后记录。
//!
//! # 提交与提示扩展
//!
//! `Enter` 立即提交。`Tab` 在任务运行时请求排队；如果没有任务运行，`Tab` 像 Enter 一样提交，
//! 确保输入不被丢弃。
//! `Tab` 在输入 `!` shell 命令时不提交。
//!
//! 在提交/排队路径上，编辑器：
//!
//! - 扩展待处理的粘贴占位符，使元素范围与最终文本对齐。
//! - 修剪空白并根据文本元素重新定位。
//! - 清理本地附加图像，使只有扩展后存活的占位符被发送。
//! - 即使文本为空，也将远程图像 URL 作为独立附件保留。
//!
//! 当这些路径在成功提交或 slash 命令分派后清除可见的 textarea 时，它们有意识地保留 textarea
//! 的 kill 缓冲区。这样用户可以用 `Ctrl+K` 删除草稿的一部分，执行编辑器操作（如更改推理级别），
//! 然后用 `Ctrl+Y` 将删除的文本恢复到现在为空的草稿中。
//!
//! 斜杠弹窗使用的数字自动提交路径执行相同的待处理粘贴扩展和附件清理，并在成功时清除待处理粘贴状态。
//! 带参数的 Slash 命令（如 `/plan` 和 `/review`）复用相同的准备路径，以便在提取参数时保留
//! 粘贴内容和文本元素。
//!
//! # 父属线程模式
//!
//! 父属子代理线程在阻止代理驱动的提交时保持草稿可编辑。
//! 在 `Enter` 和 `Tab` 提交路径上，普通提示、不允许的 slash 命令和 `!` shell 命令返回
//! `ParentOwnedInputBlocked` 而不清除草稿。纯本地和导航 slash 命令仍然可用，因此用户可以离开或
//! 管理视图。
//!
//! # 推理努力动画
//!
//! 编辑器在模型相关的表面刷新时观察有效的推理层级。
//! 首次观察、会话配置和恢复的线程建立基线；Max/Ultra 的真正变化可以随后排队编辑器和状态行转换
//! （当 motion 和颜色支持允许时）。
//! 重复选择不重启任何效果，降到 Max 以下清除它们，恢复清除重放保存会话时排队的任何转换。
//! 渲染通过帧请求器推进活动转换直到完成。
//!
//! # 大粘贴占位符
//!
//! 大粘贴在缓冲区中插入一个元素占位符，并将完整文本存储在 `pending_pastes` 中。占位符标签
//! 派生自粘贴的字符数：
//!
//! - 首次粘贴使用 `[Pasted Content N chars]`。
//! - 相同大小的额外待处理粘贴添加数字后缀（`#2`、`#3`、...），后缀从 `pending_pastes` 中仍存在的
//!   占位符计算。
//! - 当所有相同大小的占位符被清除或删除时，下一个相同大小的粘贴重用不带后缀的基础标签。
//!
//! # 远程图像行（上/下/删除）
//!
//! 远程图像 URL 在 textarea 上方（在同一编辑器块内）渲染为不可编辑的 `[Image #N]` 行。这些行
//! 代表从 app-server/backtrack 历史中重新注入的图像附件；TUI 用户可以删除它们，但不能在该行区域
//! 输入。
//!
//! 键盘行为：
//!
//! - 在 textarea 光标 `0` 处按 `上` 进入最后一张远程图像的行选择。
//! - `上`/`下` 在远程行之间移动选择。
//! - 在最后一行按 `下` 清除选择并将控制权返回 textarea。
//! - `Delete`/`Backspace` 删除选中的远程图像行。
//!
//! 占位符编号在远程和本地图像之间统一：
//!
//! - 远程行占用 `[Image #1]..[Image #M]`。
//! - 本地占位符在该范围之后偏移（`[Image #M+1]..`）。
//! - 删除远程行重新标记本地占位符以保持编号连续。
//!
//! # 非 bracketed 粘贴突发
//!
//! 在某些终端（尤其是 Windows 上），粘贴以 `KeyCode::Char` 和 `KeyCode::Enter` 按键事件的快速
//! 序列到达，而不是单个粘贴事件。
//!
//! 为避免将这些突发误解释为真实输入（并防止短暂 UI 效果如在粘贴的 `?` 上切换快捷键覆盖），我们
//! 将 "普通" 字符事件送入 [`PasteBurst`](super::paste_burst::PasteBurst)，它缓冲突发并随后
//! 通过 [`ChatComposer::handle_paste`] 刷新。
//!
//! 突发检测器有意地区分 ASCII 和非 ASCII：
//!
//! - ASCII：我们短暂保留第一个快速字符（闪烁抑制），直到知道流是否像粘贴。
//! - 非 ASCII：我们不保留第一个字符（IME 输入会感觉丢失），但仍允许实际粘贴流的突发检测。
//!
//! 突发检测器也可以被禁用（`disable_paste_burst`），这会绕过状态机并将按键流视为正常输入。从
//! 启用到禁用的切换时，编辑器刷新/清除任何进行中的突发状态，防止其泄漏到后续输入中。
//!
//! 详细的突发状态机见 paste_burst.rs。
//!
//! # PasteBurst 集成点
//!
//! 突发检测器在几个特定位置被咨询：
//!
//! - [`ChatComposer::handle_input_basic`]：首先刷新任何到期的突发，然后拦截普通字符输入以缓冲或
//!   正常插入。
//! - [`ChatComposer::handle_non_ascii_char`]：处理非 ASCII/IME 路径而不保留第一个字符，同时
//!   仍允许通过回溯捕获进行粘贴检测。
//! - [`ChatComposer::flush_paste_burst_if_due`]/[`ChatComposer::handle_paste_burst_flush`]：
//!   从 UI 刻度调用，将待处理突发转换为显式粘贴（`handle_paste`）或正常输入的字符。
//!
//! # 输入禁用模式
//!
//! 编辑器可以暂时只读（`input_enabled = false`）。该模式下忽略编辑并渲染占位符提示而不是可编辑
//! 的 textarea。这是整体状态机的一部分，因为它影响从给定 UI 状态哪些转换是可能的。
//!
use crate::message_history::HistoryBatchCursor;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::key_hint::has_ctrl_or_alt;
use crate::tui_core::line_truncation::truncate_line_with_ellipsis_if_overflow;
use crate::tui_core::ui_consts::FOOTER_INDENT_COLS;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Margin;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;

use crate::protocol_compat::openai_models::ReasoningEffort;

use super::chat_composer_history::ChatComposerHistory;
use super::chat_composer_history::HistoryEntry;
use super::chat_composer_history::HistoryEntryResponse;
use super::chat_composer_history::HistorySearchResult;
use super::command_popup::CommandItem;
use super::effort_ignition::EffortIgnition;
use super::effort_ignition::EffortTier;
use super::effort_ignition::IGNITION_FRAME_TICK;
use super::effort_ignition::IgnitionStyle;
use super::effort_status_line::EFFORT_STATUS_LINE_FRAME_TICK;
use super::effort_status_line::EffortStatusLineTransition;
use super::file_search_popup::FileSearchPopup;
use super::footer::CollaborationModeIndicator;
use super::footer::FooterKeyHints;
use super::footer::FooterMode;
use super::footer::FooterProps;
use super::footer::GoalStatusIndicator;
use super::footer::SummaryLeft;
use super::footer::can_show_left_with_context;
use super::footer::context_window_line;
use super::footer::esc_hint_mode;
use super::footer::footer_height;
use super::footer::footer_hint_items_width;
use super::footer::footer_line_width;
use super::footer::inset_footer_hint_area;
use super::footer::max_left_width_for_right;
use super::footer::passive_footer_status_line;
use super::footer::render_context_right;
use super::footer::render_footer_from_props;
use super::footer::render_footer_hint_items;
use super::footer::render_footer_line;
use super::footer::reset_mode_after_activity;
use super::footer::side_conversation_context_line;
use super::footer::single_line_footer_layout;
use super::footer::status_line_right_indicator_line;
use super::footer::toggle_shortcut_mode;
use super::footer::uses_passive_footer_status_layout;
use super::mentions_v2::MentionV2Popup;
use super::mentions_v2::MentionV2Selection;
use super::paste_burst::CharDecision;
use super::paste_burst::PasteBurst;
use super::prompt_args::parse_slash_name;
use super::skill_popup::MentionItem;
use super::skill_popup::SkillPopup;
use super::slash_commands::BuiltinCommandFlags;
use super::slash_commands::ServiceTierCommand;
use super::slash_commands::SlashCommandItem;
use crate::protocol_compat::ThreadId;
use crate::protocol_compat::user_input::ByteRange;
use crate::protocol_compat::user_input::MAX_USER_INPUT_TEXT_CHARS;
use crate::protocol_compat::user_input::TextElement;
use crate::tui_core::bottom_pane::paste_burst::FlushResult;
use crate::tui_core::history_cell::sanitize_user_text;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::keymap::EditorKeymap;
use crate::tui_core::keymap::RuntimeKeymap;
use crate::tui_core::keymap::VimNormalKeymap;
use crate::tui_core::keymap::primary_binding;
use crate::tui_core::onboarding::mark_underlined_hyperlink;
use crate::tui_core::render::Insets;
use crate::tui_core::render::RectExt;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::slash_command::SlashCommand;
use crate::tui_core::style::user_message_style;

mod attachment_state;
mod completion_target;
mod draft_state;
mod footer_state;
mod helpers;
mod history_search;
mod mention;
mod popup_state;
mod slash_input;

use self::helpers::footer_insert_newline_key;
use self::helpers::parent_owned_command_is_allowed;
use self::helpers::plan_mode_nudge_line;
use self::helpers::skill_description;
use self::helpers::user_input_too_large_message;

use self::mention::find_next_mention_token_range;
use self::mention::is_mention_name_char;

use self::attachment_state::AttachmentState;
use self::draft_state::ComposerMentionBinding;
use self::draft_state::DraftState;
use self::footer_state::FooterState;
use self::history_search::HistorySearchSession;
use self::popup_state::ActivePopup;
use self::popup_state::DismissedToken;
use self::popup_state::PopupState;
use self::slash_input::SlashInput;
use self::slash_input::SlashValidation;
use self::slash_input::SubmissionValidation;
#[cfg(test)]
use crate::app_server_protocol::SkillInterface;
use crate::app_server_protocol::SkillMetadata;
use crate::connectors::AppInfo;
use crate::file_search::FileMatch;
#[cfg(test)]
use crate::plugin::AppConnectorId;
use crate::plugin::PluginCapabilitySummary;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::app_event::ConnectorsSnapshot;
use crate::tui_core::app_event_sender::AppEventSender;
use crate::tui_core::bottom_pane::LocalImageAttachment;
use crate::tui_core::bottom_pane::MentionBinding;
use crate::tui_core::bottom_pane::textarea::TextArea;
use crate::tui_core::clipboard_paste::normalize_pasted_path;
use crate::tui_core::clipboard_paste::pasted_image_format;
use crate::tui_core::history_cell;
use crate::tui_core::skills_helpers::skill_display_name;
use crate::tui_core::tui::FrameRequester;
use crate::tui_core::ui_consts::LIVE_PREFIX_COLS;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use ratatui::style::Color;

/// 如果粘贴的内容超过此字符数，则在 UI 中用替换为
/// 占位符。
const LARGE_PASTE_CHAR_THRESHOLD: usize = 1000;

/// 用户与文本区域交互时返回的结果。
#[derive(Debug, PartialEq)]
pub enum InputResult {
    Submitted {
        text: String,
        text_elements: Vec<TextElement>,
    },
    Queued {
        text: String,
        text_elements: Vec<TextElement>,
        action: QueuedInputAction,
        pending_pastes: Vec<(String, String)>,
    },
    /// 由编辑器解析出的裸斜杠命令。
    ///
    /// 分派此变体的调用方还需要负责解析编辑器在清空可见输入之前
    /// 暂存的任何待处理本地命令历史条目。
    Command(SlashCommand),
    /// 由编辑器解析出的裸模型服务档位命令。
    ServiceTierCommand(ServiceTierCommand),
    /// 内联斜杠命令及其修剪后的参数文本。
    ///
    /// `TextElement` 范围会被重新映射到参数字符串中，而任何待处理的本地
    /// 命令历史条目仍表示原始的命令调用，只有分派接受时才应提交。
    CommandWithArgs(SlashCommand, String, Vec<TextElement>),
    /// 在查看父拥有的派生子线程时尝试发起了面向代理的输入。
    ParentOwnedInputBlocked,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuedInputAction {
    Plain,
    ParseSlash,
    RunShell,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingPasteHandling {
    Expand,
    Preserve,
}

/// 在其他底部面板界面中复用聊天编辑器的功能标志。
///
/// 默认值保持当前行为不变。其他调用点可以通过构造一个将
/// 特定标志设置为 `false` 的配置来选择退出这些行为。
#[derive(Clone, Copy, Debug)]
pub(crate) struct ChatComposerConfig {
    /// 是否允许显示命令/文件/技能弹出窗口。
    pub(crate) popups_enabled: bool,
    /// 是否将 `/...` 输入解析并作为斜杠命令分派。
    pub(crate) slash_commands_enabled: bool,
    /// 粘贴文件路径是否可附加本地图像。
    pub(crate) image_paste_enabled: bool,
}

impl Default for ChatComposerConfig {
    fn default() -> Self {
        Self {
            popups_enabled: true,
            slash_commands_enabled: true,
            image_paste_enabled: true,
        }
    }
}

impl ChatComposerConfig {
    /// 用于嵌入其他界面的纯文本输入的极简预设。
    ///
    /// 这会禁用弹出窗口、斜杠命令和图像路径附加行为，
    /// 使编辑器像简单的便签字段一样工作。
    pub(crate) const fn plain_text() -> Self {
        Self {
            popups_enabled: false,
            slash_commands_enabled: false,
            image_paste_enabled: false,
        }
    }
}

pub(crate) struct ChatComposer {
    draft: DraftState,
    popups: PopupState,
    app_event_tx: AppEventSender,
    history: ChatComposerHistory,
    footer: FooterState,
    has_focus: bool,
    frame_requester: Option<FrameRequester>,
    effort_tier: Option<EffortTier>,
    effort_animation_style: Option<IgnitionStyle>,
    effort_ignition: Option<EffortIgnition>,
    effort_status_line_transition: Option<EffortStatusLineTransition>,
    effort_observed: bool,
    attachments: AttachmentState,
    placeholder_text: String,
    blocks_direct_input: bool,
    is_task_running: bool,
    queue_submissions: bool,
    /// 为应用级分派后的本地召回暂存的斜杠命令草稿。
    ///
    /// 该槽位刻意与 `ChatComposerHistory` 分开，以便内联斜杠命令可以
    /// 准备其参数文本，而不会重复记录完整的命令调用。
    pending_slash_command_history: Option<HistoryEntry>,
    skills: Option<Vec<SkillMetadata>>,
    plugins: Option<Vec<PluginCapabilitySummary>>,
    connectors_snapshot: Option<ConnectorsSnapshot>,
    collaboration_modes_enabled: bool,
    config: ChatComposerConfig,
    connectors_enabled: bool,
    plugins_command_enabled: bool,
    token_activity_command_enabled: bool,
    service_tier_commands_enabled: bool,
    service_tier_commands: Vec<ServiceTierCommand>,
    mentions_v2_enabled: bool,
    goal_command_enabled: bool,
    personality_command_enabled: bool,
    windows_degraded_sandbox_active: bool,
    side_conversation_active: bool,
    history_search: Option<HistorySearchSession>,
    submit_keys: Vec<KeyBinding>,
    queue_keys: Vec<KeyBinding>,
    toggle_shortcuts_keys: Vec<KeyBinding>,
    history_search_previous_keys: Vec<KeyBinding>,
    history_search_next_keys: Vec<KeyBinding>,
    editor_keymap: EditorKeymap,
    vim_normal_keymap: VimNormalKeymap,
}

/// 已解析的遗留 `$` 目标，以及在消除 shell 语法歧义时构建的任何目录。
struct MentionCompletionTarget {
    range: Range<usize>,
    query: String,
    prebuilt_mentions: Option<Vec<MentionItem>>,
}

#[derive(Clone, Debug)]
struct ComposerDraft {
    text: String,
    text_elements: Vec<TextElement>,
    local_image_paths: Vec<PathBuf>,
    remote_image_urls: Vec<String>,
    mention_bindings: Vec<MentionBinding>,
    pending_pastes: Vec<(String, String)>,
    cursor: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ComposerDraftSnapshot {
    pub(crate) text: String,
    pub(crate) text_elements: Vec<TextElement>,
    pub(crate) local_images: Vec<LocalImageAttachment>,
    pub(crate) remote_image_urls: Vec<String>,
    pub(crate) mention_bindings: Vec<MentionBinding>,
    pub(crate) pending_pastes: Vec<(String, String)>,
}

const FOOTER_SPACING_HEIGHT: u16 = 0;

impl ChatComposer {
    fn slash_input(&self) -> SlashInput<'_> {
        SlashInput::new(
            self.slash_commands_enabled(),
            self.draft.is_bash_mode,
            self.builtin_command_flags(),
            &self.service_tier_commands,
        )
    }

    fn builtin_command_flags(&self) -> BuiltinCommandFlags {
        BuiltinCommandFlags {
            collaboration_modes_enabled: self.collaboration_modes_enabled,
            connectors_enabled: self.connectors_enabled,
            plugins_command_enabled: self.plugins_command_enabled,
            token_activity_command_enabled: self.token_activity_command_enabled,
            service_tier_commands_enabled: self.service_tier_commands_enabled,
            goal_command_enabled: self.goal_command_enabled,
            personality_command_enabled: self.personality_command_enabled,
            allow_elevate_sandbox: self.windows_degraded_sandbox_active,
            side_conversation_active: self.side_conversation_active,
        }
    }

    pub fn new(
        has_input_focus: bool,
        app_event_tx: AppEventSender,
        enhanced_keys_supported: bool,
        placeholder_text: String,
        disable_paste_burst: bool,
    ) -> Self {
        Self::new_with_config(
            has_input_focus,
            app_event_tx,
            enhanced_keys_supported,
            placeholder_text,
            disable_paste_burst,
            ChatComposerConfig::default(),
        )
    }

    /// 使用显式功能门控构造编辑器。
    ///
    /// 这使得它能在 request-user-input 等场景中复用，
    /// 在无需斜杠命令或弹出窗口的情况下获得相同的视觉和编辑行为。
    pub(crate) fn new_with_config(
        has_input_focus: bool,
        app_event_tx: AppEventSender,
        enhanced_keys_supported: bool,
        placeholder_text: String,
        disable_paste_burst: bool,
        config: ChatComposerConfig,
    ) -> Self {
        let use_shift_enter_hint = enhanced_keys_supported;
        let default_keymap = RuntimeKeymap::defaults();
        let default_editor_keymap = default_keymap.editor.clone();
        let default_vim_normal_keymap = default_keymap.vim_normal.clone();

        let mut this = Self {
            draft: DraftState::new(),
            popups: PopupState::default(),
            app_event_tx,
            history: ChatComposerHistory::new(),
            footer: FooterState {
                quit_shortcut_expires_at: None,
                quit_shortcut_key: key_hint::ctrl(KeyCode::Char('c')),
                esc_backtrack_hint: false,
                use_shift_enter_hint,
                mode: FooterMode::ComposerEmpty,
                hint_override: None,
                plan_mode_nudge_visible: false,
                flash: None,
                context_window_percent: None,
                context_window_used_tokens: None,
                collaboration_mode_indicator: None,
                goal_status_indicator: None,
                ide_context_active: false,
                status_line_value: None,
                status_line_hyperlink_url: None,
                status_line_enabled: false,
                side_conversation_context_label: None,
                active_agent_label: None,
                external_editor_key: Some(key_hint::ctrl(KeyCode::Char('g'))),
                show_transcript_key: Some(key_hint::ctrl(KeyCode::Char('t'))),
                insert_newline_key: footer_insert_newline_key(
                    &default_keymap.editor.insert_newline,
                    use_shift_enter_hint,
                ),
                queue_key: Some(key_hint::plain(KeyCode::Tab)),
                toggle_shortcuts_key: Some(key_hint::plain(KeyCode::Char('?'))),
                history_search_key: primary_binding(
                    &default_keymap.composer.history_search_previous,
                ),
                reasoning_down_key: primary_binding(&default_keymap.chat.decrease_reasoning_effort),
                reasoning_up_key: primary_binding(&default_keymap.chat.increase_reasoning_effort),
            },
            has_focus: has_input_focus,
            frame_requester: None,
            effort_tier: None,
            effort_animation_style: None,
            effort_ignition: None,
            effort_status_line_transition: None,
            effort_observed: false,
            attachments: AttachmentState::default(),
            placeholder_text,
            blocks_direct_input: false,
            is_task_running: false,
            queue_submissions: false,
            pending_slash_command_history: None,
            skills: None,
            plugins: None,
            connectors_snapshot: None,
            collaboration_modes_enabled: false,
            config,
            connectors_enabled: false,
            plugins_command_enabled: false,
            token_activity_command_enabled: false,
            service_tier_commands_enabled: false,
            service_tier_commands: Vec::new(),
            mentions_v2_enabled: false,
            goal_command_enabled: false,
            personality_command_enabled: false,
            windows_degraded_sandbox_active: false,
            side_conversation_active: false,
            history_search: None,
            submit_keys: vec![key_hint::plain(KeyCode::Enter)],
            queue_keys: vec![key_hint::plain(KeyCode::Tab)],
            toggle_shortcuts_keys: vec![
                key_hint::plain(KeyCode::Char('?')),
                key_hint::shift(KeyCode::Char('?')),
            ],
            history_search_previous_keys: default_keymap.composer.history_search_previous.clone(),
            history_search_next_keys: default_keymap.composer.history_search_next.clone(),
            editor_keymap: default_editor_keymap,
            vim_normal_keymap: default_vim_normal_keymap,
        };
        // 通过 setter 应用配置，使副作用集中处理。
        this.set_disable_paste_burst(disable_paste_burst);
        this
    }

    pub(crate) fn set_frame_requester(&mut self, frame_requester: FrameRequester) {
        self.frame_requester = Some(frame_requester);
    }

    /// 记录有效的推理档位，捕获外发状态
    /// 行，并在初始基线之后为真正的 Max/Ultra 变更排队一次性效果。
    pub(crate) fn set_active_reasoning_effort(
        &mut self,
        effort: Option<&ReasoningEffort>,
        animations_enabled: bool,
    ) -> bool {
        let tier = EffortTier::from_effort(effort);
        let is_baseline = !self.effort_observed;
        self.effort_observed = true;
        if self.effort_tier == tier {
            return false;
        }
        self.effort_tier = tier;
        self.effort_ignition = None;
        self.effort_status_line_transition = None;
        if let Some(tier) = tier
            && !is_baseline
            && animations_enabled
        {
            let style = IgnitionStyle::random(self.effort_animation_style);
            self.effort_ignition = Some(EffortIgnition::new(tier, style));
            self.effort_animation_style = Some(style);
            if self.footer.status_line_enabled
                && !self.footer.plan_mode_nudge_visible
                && let Some(previous) = passive_footer_status_line(&self.footer_props())
            {
                self.effort_status_line_transition =
                    Some(EffortStatusLineTransition::new(tier, previous));
            }
            if let Some(frame_requester) = &self.frame_requester {
                frame_requester.schedule_frame();
            }
        }
        true
    }

    /// 建立当前档位而不保留或启动一次性效果。
    pub(crate) fn set_active_reasoning_effort_baseline(
        &mut self,
        effort: Option<&ReasoningEffort>,
    ) {
        self.effort_tier = EffortTier::from_effort(effort);
        self.effort_observed = true;
        self.effort_ignition = None;
        self.effort_status_line_transition = None;
    }

    pub fn set_skill_mentions(&mut self, skills: Option<Vec<SkillMetadata>>) {
        self.skills = skills;
        self.sync_popups();
    }

    pub fn set_plugin_mentions(&mut self, plugins: Option<Vec<PluginCapabilitySummary>>) {
        self.plugins = plugins;
        self.sync_popups();
    }

    pub fn set_plugins_command_enabled(&mut self, enabled: bool) {
        self.plugins_command_enabled = enabled;
    }

    pub fn set_token_activity_command_enabled(&mut self, enabled: bool) {
        self.token_activity_command_enabled = enabled;
    }

    pub fn set_mentions_v2_enabled(&mut self, enabled: bool) {
        self.mentions_v2_enabled = enabled;
        self.history.set_at_mention_restore_enabled(enabled);
        self.sync_popups();
    }

    /// 切换编辑器侧的图像粘贴处理。
    ///
    /// 这仅影响类似图像的粘贴内容是否被转换为附件；
    /// `ChatWidget` 层在图像被提交前仍会执行能力检查。
    pub fn set_image_paste_enabled(&mut self, enabled: bool) {
        self.config.image_paste_enabled = enabled;
    }

    pub fn set_connector_mentions(&mut self, connectors_snapshot: Option<ConnectorsSnapshot>) {
        self.connectors_snapshot = connectors_snapshot;
        self.sync_popups();
    }

    pub(crate) fn take_mention_bindings(&mut self) -> Vec<MentionBinding> {
        let elements = self.current_mention_elements();
        let mut ordered = Vec::new();
        for (id, sigil, mention) in elements {
            if let Some(binding) = self.draft.mention_bindings.remove(&id)
                && binding.sigil == sigil
                && binding.mention == mention
            {
                ordered.push(MentionBinding {
                    sigil: binding.sigil,
                    mention: binding.mention,
                    path: binding.path,
                });
            }
        }
        self.draft.mention_bindings.clear();
        ordered
    }

    pub fn set_collaboration_modes_enabled(&mut self, enabled: bool) {
        self.collaboration_modes_enabled = enabled;
    }

    pub fn set_connectors_enabled(&mut self, enabled: bool) {
        self.connectors_enabled = enabled;
    }

    pub fn set_service_tier_commands_enabled(&mut self, enabled: bool) {
        self.service_tier_commands_enabled = enabled;
    }

    pub fn set_service_tier_commands(&mut self, commands: Vec<ServiceTierCommand>) {
        self.service_tier_commands = commands;
        self.sync_popups();
    }

    pub fn set_goal_command_enabled(&mut self, enabled: bool) {
        self.goal_command_enabled = enabled;
    }

    /// 从单个运行时快照替换编辑器、编辑器和底部提示的按键绑定。
    ///
    /// 提交和排队绑定在这里缓存，因为编辑器分派必须
    /// 在通用文本区域编辑之前检查它们。内嵌的文本区域接收
    /// 同一快照的编辑器绑定，以便实时重映射不会让提交
    /// 键已更新而光标/编辑键仍用旧默认值。
    pub(crate) fn set_keymap_bindings(&mut self, keymap: &RuntimeKeymap) {
        self.submit_keys = keymap.composer.submit.clone();
        self.queue_keys = keymap.composer.queue.clone();
        self.toggle_shortcuts_keys = keymap.composer.toggle_shortcuts.clone();
        self.history_search_previous_keys = keymap.composer.history_search_previous.clone();
        self.history_search_next_keys = keymap.composer.history_search_next.clone();
        self.editor_keymap = keymap.editor.clone();
        self.vim_normal_keymap = keymap.vim_normal.clone();
        self.draft.textarea.set_keymap_bindings(keymap);
        self.footer.external_editor_key = primary_binding(&keymap.app.open_external_editor);
        self.footer.show_transcript_key = primary_binding(&keymap.app.open_transcript);
        self.footer.insert_newline_key = footer_insert_newline_key(
            &keymap.editor.insert_newline,
            self.footer.use_shift_enter_hint,
        );
        self.footer.queue_key = primary_binding(&keymap.composer.queue);
        self.footer.toggle_shortcuts_key = primary_binding(&keymap.composer.toggle_shortcuts);
        self.footer.history_search_key = primary_binding(&keymap.composer.history_search_previous);
        self.footer.reasoning_down_key = primary_binding(&keymap.chat.decrease_reasoning_effort);
        self.footer.reasoning_up_key = primary_binding(&keymap.chat.increase_reasoning_effort);
    }

    pub fn set_collaboration_mode_indicator(
        &mut self,
        indicator: Option<CollaborationModeIndicator>,
    ) {
        self.footer.collaboration_mode_indicator = indicator;
    }

    pub fn set_goal_status_indicator(&mut self, indicator: Option<GoalStatusIndicator>) {
        self.footer.goal_status_indicator = indicator;
    }

    pub fn set_ide_context_active(&mut self, active: bool) {
        self.footer.ide_context_active = active;
    }

    pub fn set_personality_command_enabled(&mut self, enabled: bool) {
        self.personality_command_enabled = enabled;
    }

    pub fn set_side_conversation_active(&mut self, active: bool) {
        self.side_conversation_active = active;
    }

    /// 供仍切换已移除的转向模式标志的测试使用的兼容垫片。
    #[cfg(test)]
    pub fn set_steer_enabled(&mut self, _enabled: bool) {}
    /// 集中式功能门控使配置检查远离调用点。
    fn popups_enabled(&self) -> bool {
        self.config.popups_enabled
    }

    fn slash_commands_enabled(&self) -> bool {
        self.config.slash_commands_enabled
    }

    fn image_paste_enabled(&self) -> bool {
        self.config.image_paste_enabled
    }
    #[cfg(target_os = "windows")]
    pub fn set_windows_degraded_sandbox_active(&mut self, enabled: bool) {
        self.windows_degraded_sandbox_active = enabled;
    }
    fn layout_areas(&self, area: Rect) -> [Rect; 4] {
        self.layout_areas_with_textarea_right_reserve(area, /*textarea_right_reserve*/ 0)
    }

    fn layout_areas_with_textarea_right_reserve(
        &self,
        area: Rect,
        textarea_right_reserve: u16,
    ) -> [Rect; 4] {
        let footer_props = self.footer_props();
        let footer_hint_height = self
            .custom_footer_height()
            .unwrap_or_else(|| footer_height(&footer_props));
        let footer_spacing = Self::footer_spacing(footer_hint_height);
        let footer_total_height = footer_hint_height + footer_spacing;
        let popup_constraint = match &self.popups.active {
            ActivePopup::Command(popup) => {
                Constraint::Max(popup.calculate_required_height(area.width))
            }
            ActivePopup::File(popup) => Constraint::Max(popup.calculate_required_height()),
            ActivePopup::Skill(popup) => {
                Constraint::Max(popup.calculate_required_height(area.width))
            }
            ActivePopup::MentionV2(popup) => {
                Constraint::Max(popup.calculate_required_height(area.width))
            }
            ActivePopup::None => Constraint::Max(footer_total_height),
        };
        let [composer_rect, popup_rect] =
            Layout::vertical([Constraint::Min(3), popup_constraint]).areas(area);
        let mut textarea_rect = composer_rect.inset(Insets::tlbr(
            /*top*/ 1,
            LIVE_PREFIX_COLS,
            /*bottom*/ 1,
            /*right*/ 1u16.saturating_add(textarea_right_reserve),
        ));
        let remote_images_height = self
            .attachments
            .remote_image_lines()
            .len()
            .try_into()
            .unwrap_or(u16::MAX)
            .min(textarea_rect.height.saturating_sub(1));
        let remote_images_separator = u16::from(remote_images_height > 0);
        let consumed = remote_images_height.saturating_add(remote_images_separator);
        let remote_images_rect = Rect {
            x: textarea_rect.x,
            y: textarea_rect.y,
            width: textarea_rect.width,
            height: remote_images_height,
        };
        textarea_rect.y = textarea_rect.y.saturating_add(consumed);
        textarea_rect.height = textarea_rect.height.saturating_sub(consumed);
        [composer_rect, remote_images_rect, textarea_rect, popup_rect]
    }

    fn footer_spacing(footer_hint_height: u16) -> u16 {
        if footer_hint_height == 0 {
            0
        } else {
            FOOTER_SPACING_HEIGHT
        }
    }

    pub fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.cursor_pos_with_textarea_right_reserve(area, /*textarea_right_reserve*/ 0)
    }

    pub(crate) fn cursor_pos_with_textarea_right_reserve(
        &self,
        area: Rect,
        textarea_right_reserve: u16,
    ) -> Option<(u16, u16)> {
        if !self.draft.input_enabled || self.attachments.selected_remote_image_index.is_some() {
            return None;
        }

        if let Some(pos) = self.history_search_cursor_pos(area) {
            return Some(pos);
        }

        let [_, _, textarea_rect, _] =
            self.layout_areas_with_textarea_right_reserve(area, textarea_right_reserve);
        let state = *self.draft.textarea_state.borrow();
        self.draft
            .textarea
            .cursor_pos_with_state(textarea_rect, state)
    }
    /// 编辑器当前是否不含用户输入的内容。
    pub(crate) fn is_empty(&self) -> bool {
        self.draft.textarea.is_empty() && !self.draft.is_bash_mode && self.attachments.is_empty()
    }

    /// 记录本地持久历史元数据，以便编辑器可以跨
    /// 会话历史导航。
    pub(crate) fn set_history_metadata(
        &mut self,
        thread_id: ThreadId,
        log_id: u64,
        entry_count: usize,
    ) {
        self.history.set_metadata(thread_id, log_id, entry_count);
    }

    /// 集成按需历史查找的异步响应。
    ///
    /// 如果条目存在且偏移量仍匹配活动历史光标，编辑器会
    /// 立即恢复该条目。此路径刻意经由
    /// [`Self::apply_history_entry`] 路由，使光标放置与键盘历史
    /// 召回语义保持一致。
    pub(crate) fn on_history_entry_response(
        &mut self,
        log_id: u64,
        offset: usize,
        entry: Option<String>,
    ) -> bool {
        match self
            .history
            .on_entry_response(log_id, offset, entry, &self.app_event_tx)
        {
            HistoryEntryResponse::Found(entry) => {
                // 持久 ↑/↓ 历史是纯文本的（向后兼容且避免持久化
                // 附件），但会话内本地 ↑/↓ 历史可以恢复元素和图像路径。
                self.apply_history_entry(entry);
                true
            }
            HistoryEntryResponse::Search(result) => {
                self.apply_history_search_result(result);
                true
            }
            HistoryEntryResponse::Ignored => false,
        }
    }

    pub(crate) fn on_history_batch_response(
        &mut self,
        log_id: u64,
        cursor: HistoryBatchCursor,
        entries: Vec<crate::tui_core::app_event::HistoryBatchEntryResponse>,
        next_older_cursor: Option<HistoryBatchCursor>,
    ) -> bool {
        let result = self.history.on_batch_response(
            log_id,
            cursor,
            entries,
            next_older_cursor,
            &self.app_event_tx,
        );
        self.apply_history_batch_result(result)
    }

    /// 应用失败的批量查找，而不将其与历史耗尽混为一谈。
    ///
    /// 历史状态机要么安排有界的重试，要么保留现有匹配，要么
    /// 在没有选择匹配时使搜索 UI 返回其空闲草稿。
    pub(crate) fn on_history_batch_error(
        &mut self,
        log_id: u64,
        cursor: HistoryBatchCursor,
    ) -> bool {
        let result = self
            .history
            .on_batch_error(log_id, cursor, &self.app_event_tx);
        self.apply_history_batch_result(result)
    }

    fn apply_history_batch_result(&mut self, result: Option<HistorySearchResult>) -> bool {
        let Some(result) = result else {
            return false;
        };
        self.apply_history_search_result(result);
        true
    }

    pub(crate) fn record_replayed_user_message_history(&mut self, entry: HistoryEntry) {
        self.history.record_replayed_submission(entry);
    }

    /// 将粘贴文本集成到编辑器中。
    ///
    /// 作为粘贴文本集成的唯一位置，适用于：
    ///
    /// - 终端突显的真实/显式粘贴事件，以及
    /// - [`PasteBurst`](super::paste_burst::PasteBurst) 缓冲并稍后
    ///   在此刷新的无括号"粘贴突发"。
    ///
    /// 行为：
    ///
    /// - 如果粘贴大于 `LARGE_PASTE_CHAR_THRESHOLD` 个字符，插入一个占位符
    ///   元素（提交时展开）并将完整文本存储在 `pending_pastes` 中。
    /// - 否则，如果粘贴看起来像图像路径，则附加图像并插入
    ///   尾随空格，以便用户可以自然继续输入。
    /// - 否则，将粘贴文本直接插入文本区域。
    ///
    /// 在所有情况下，清除任何粘贴突发的 Enter 抑制状态，以便真实的粘贴不能影响
    /// 下一次用户 Enter 键，然后同步弹出状态。
    pub fn handle_paste(&mut self, pasted: String) -> bool {
        let pasted = pasted.replace("\r\n", "\n").replace('\r', "\n");
        let pasted = sanitize_user_text(&pasted);
        let char_count = pasted.chars().count();
        if char_count > LARGE_PASTE_CHAR_THRESHOLD {
            let placeholder = self.next_large_paste_placeholder(char_count);
            self.draft.textarea.insert_element(&placeholder);
            self.draft.pending_pastes.push((placeholder, pasted));
        } else if char_count > 1
            && self.image_paste_enabled()
            && self.handle_paste_image_path(pasted.clone())
        {
            self.draft.textarea.insert_str(" ");
        } else {
            self.insert_str(&pasted);
        }
        self.draft.paste_burst.clear_after_explicit_paste();
        self.sync_popups();
        true
    }

    pub fn handle_paste_image_path(&mut self, pasted: String) -> bool {
        let Some(path_buf) = normalize_pasted_path(&pasted) else {
            return false;
        };

        // normalize_pasted_path 已处理 Windows → WSL 路径转换，
        // 因此我们可以直接尝试读取图像尺寸。
        match image::image_dimensions(&path_buf) {
            Ok((width, height)) => {
                tracing::info!("OK: {pasted}");
                tracing::debug!("image dimensions={}x{}", width, height);
                let format = pasted_image_format(&path_buf);
                tracing::debug!("attached image format={}", format.label());
                self.attach_image(path_buf);
                true
            }
            Err(err) => {
                tracing::trace!("ERR: {err}");
                false
            }
        }
    }

    /// 启用或禁用粘贴突发处理。
    ///
    /// `disable_paste_burst` 是针对那些不需要突发启发式或已在
    /// 其他地方处理的终端/平台的逃生舱口。
    ///
    /// 当从启用 → 禁用转换时，我们会"解除"任何进行中的突发状态，使其
    /// 不能影响后续的正常输入：
    ///
    /// - 首先，通过
    ///   [`PasteBurst::flush_before_modified_input`] 立即刷新任何被持有/缓冲的文本，并通过 `handle_paste(String)` 送入。
    ///   这会保留用户输入，并通过与显式
    ///   粘贴相同的集成路径（大粘贴占位符、图像路径检测和弹出同步）路由它。
    /// - 然后通过
    ///   [`PasteBurst::clear_after_explicit_paste`] 清除突发计时和 Enter 抑制窗口。
    ///
    /// 我们有意在此不使用 `clear_window_after_non_char()`：它清除计时状态
    /// 而不发出任何缓冲文本，这可能使非空缓冲区无法稍后
    /// 刷新（因为 `flush_if_due()` 依赖 `last_plain_char_time` 超时）。
    pub(crate) fn set_disable_paste_burst(&mut self, disabled: bool) {
        let was_disabled = self.draft.disable_paste_burst;
        self.draft.disable_paste_burst = disabled;
        if disabled && !was_disabled {
            if let Some(pasted) = self.draft.paste_burst.flush_before_modified_input() {
                self.handle_paste(pasted);
            }
            self.draft.paste_burst.clear_after_explicit_paste();
        }
    }

    /// 用外部编辑器中的文本替换编辑器内容。
    /// 清除待处理的粘贴占位符，仅保留其
    /// 占位符标签仍出现在新文本中的附件。图像占位符
    /// 重新编号为 `[Image #M+1]..[Image #N]`（其中 `M` 是远程图像数）。
    /// 重建元素后将光标放在末尾。
    pub(crate) fn apply_external_edit(&mut self, text: String) {
        self.draft.pending_pastes.clear();
        let (text, _) = self.imported_text_for_textarea(text, Vec::new());

        // 统计新文本中的占位符出现次数。
        let mut placeholder_counts: HashMap<String, usize> = HashMap::new();
        for placeholder in self
            .attachments
            .local_images
            .iter()
            .map(|image| &image.placeholder)
        {
            if placeholder_counts.contains_key(placeholder) {
                continue;
            }
            let count = text.match_indices(placeholder).count();
            if count > 0 {
                placeholder_counts.insert(placeholder.clone(), count);
            }
        }

        // 仅保留仍有匹配出现次数的附件。
        let mut kept_images = Vec::new();
        for img in self.attachments.local_images.drain(..) {
            if let Some(count) = placeholder_counts.get_mut(&img.placeholder)
                && *count > 0
            {
                *count -= 1;
                kept_images.push(img);
            }
        }
        self.attachments.local_images = kept_images;

        // 重建文本区域，使占位符再次成为元素。
        self.draft.textarea.set_text_clearing_elements("");
        let mut remaining: HashMap<&str, usize> = HashMap::new();
        for img in &self.attachments.local_images {
            *remaining.entry(img.placeholder.as_str()).or_insert(0) += 1;
        }

        let mut occurrences: Vec<(usize, &str)> = Vec::new();
        for placeholder in remaining.keys() {
            for (pos, _) in text.match_indices(placeholder) {
                occurrences.push((pos, *placeholder));
            }
        }
        occurrences.sort_unstable_by_key(|(pos, _)| *pos);

        let mut idx = 0usize;
        for (pos, ph) in occurrences {
            let Some(count) = remaining.get_mut(ph) else {
                continue;
            };
            if *count == 0 {
                continue;
            }
            if pos > idx {
                self.draft.textarea.insert_str(&text[idx..pos]);
            }
            self.draft.textarea.insert_element(ph);
            *count -= 1;
            idx = pos + ph.len();
        }
        if idx < text.len() {
            self.draft.textarea.insert_str(&text[idx..]);
        }

        // 在远程图像前缀之后，按附件顺序保持本地图像占位符规范化。
        self.attachments
            .relabel_local_images(&mut self.draft.textarea);
        self.draft
            .textarea
            .set_cursor(self.draft.textarea.text().len());
        self.sync_popups();
    }

    /// 为编辑器文本区域启用或禁用 Vim 编辑。
    ///
    /// 模式更改时编辑器会清除任何进行中的粘贴突发状态，
    /// 因为 Vim 普通模式将快速的字符序列视为
    /// 命令，而非候选的字面粘贴文本。它还会重置瞬态
    /// 底部模式，使可见提示与新编辑界面匹配。
    pub(crate) fn set_vim_enabled(&mut self, enabled: bool) {
        self.draft.textarea.set_vim_enabled(enabled);
        self.draft.paste_burst.clear_after_explicit_paste();
        self.footer.mode = reset_mode_after_activity(self.footer.mode);
    }

    /// 切换 Vim 编辑并返回新的启用状态。
    ///
    /// 这是可配置 Vim 切换
    /// 按键绑定的应用级命令目标；调用方应使用返回值生成状态消息，
    /// 而不是在额外的编辑器变更后重新读取状态。
    pub(crate) fn toggle_vim_enabled(&mut self) -> bool {
        let enabled = !self.draft.textarea.is_vim_enabled();
        self.set_vim_enabled(enabled);
        enabled
    }

    /// 返回测试断言模式转换时 Vim 编辑是否启用。
    #[cfg(test)]
    pub(crate) fn is_vim_enabled(&self) -> bool {
        self.draft.textarea.is_vim_enabled()
    }

    /// 返回 Escape 是否应在弹出窗口之前路由到文本区域。
    ///
    /// Vim 插入模式将 Escape 视为返回普通模式的转换键。应用
    /// 事件层在运行通用 Escape 行为前询问此方法，以便同一个
    /// 键不会既退出插入模式又关闭无关 UI。
    pub(crate) fn should_handle_vim_insert_escape(&self, key_event: KeyEvent) -> bool {
        self.draft
            .textarea
            .should_handle_vim_insert_escape(key_event)
    }

    fn vim_mode_indicator_span(&self) -> Option<Span<'static>> {
        self.draft
            .textarea
            .vim_mode_label()
            .map(|label| match label {
                "Normal" => "Vim: Normal".magenta(),
                "Insert" => "Vim: Insert".green(),
                _ => unreachable!(),
            })
    }

    fn mode_indicator_line(&self, show_cycle_hint: bool) -> Option<Line<'static>> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        if let Some(vim_mode) = self.vim_mode_indicator_span() {
            spans.push(vim_mode);
        }
        if let Some(indicators) = status_line_right_indicator_line(
            self.footer.collaboration_mode_indicator,
            self.footer.goal_status_indicator.as_ref(),
            self.footer.ide_context_active,
            show_cycle_hint,
        ) {
            if !spans.is_empty() {
                spans.push(" | ".dim());
            }
            spans.extend(indicators.spans);
        }
        if spans.is_empty() {
            None
        } else {
            Some(Line::from(spans))
        }
    }

    fn right_footer_line_with_context(&self) -> Line<'static> {
        let mut line = context_window_line(
            self.footer.context_window_percent,
            self.footer.context_window_used_tokens,
        );
        if let Some(vim_mode) = self.vim_mode_indicator_span() {
            line.spans.push(" | ".dim());
            line.spans.push(vim_mode);
        }
        line
    }

    pub(crate) fn current_text_with_pending(&self) -> String {
        let text = self.current_text();
        if self.draft.pending_pastes.is_empty() {
            return text;
        }

        let (text, _) = Self::expand_pending_pastes(
            &text,
            self.current_text_elements(),
            &self.draft.pending_pastes,
        );
        text
    }

    /// 返回编辑器当前是否接受交互式草稿编辑。
    pub(crate) fn input_enabled(&self) -> bool {
        self.draft.input_enabled
    }

    pub(crate) fn pending_pastes(&self) -> Vec<(String, String)> {
        self.draft.pending_pastes.clone()
    }

    pub(crate) fn set_pending_pastes(&mut self, pending_pastes: Vec<(String, String)>) {
        let text = self.current_text();
        self.draft.pending_pastes = pending_pastes
            .into_iter()
            .filter(|(placeholder, _)| text.contains(placeholder))
            .collect();
    }

    /// 覆盖编辑器下方显示的底部提示项。传入
    /// `None` 恢复默认快捷方式底部。
    pub(crate) fn set_footer_hint_override(&mut self, items: Option<Vec<(String, String)>>) {
        self.footer.hint_override = items;
    }

    /// 更新 Plan 模式提示是否替换环境的底部行。
    ///
    /// 仅当渲染的底部可以变化时才返回 `true`，以便调用方在常规编辑器
    /// 更新时重新评估提示策略时可以避免调度冗余重绘。
    pub(crate) fn set_plan_mode_nudge_visible(&mut self, visible: bool) -> bool {
        if self.footer.plan_mode_nudge_visible == visible {
            return false;
        }
        self.footer.plan_mode_nudge_visible = visible;
        true
    }

    #[cfg(test)]
    pub(crate) fn plan_mode_nudge_visible(&self) -> bool {
        self.footer.plan_mode_nudge_visible
    }

    pub(crate) fn set_remote_image_urls(&mut self, urls: Vec<String>) {
        self.attachments
            .set_remote_image_urls(urls, &mut self.draft.textarea);
        self.sync_popups();
    }

    pub(crate) fn remote_image_urls(&self) -> Vec<String> {
        self.attachments.remote_image_urls()
    }

    pub(crate) fn take_remote_image_urls(&mut self) -> Vec<String> {
        let urls = self
            .attachments
            .take_remote_image_urls(&mut self.draft.textarea);
        self.sync_popups();
        urls
    }

    #[cfg(test)]
    pub(crate) fn show_footer_flash(&mut self, line: Line<'static>, duration: Duration) {
        self.footer.show_flash(line, duration);
    }

    /// 用 `text` 替换整个编辑器内容并重置光标。
    ///
    /// 这是"全新草稿"路径：它清除待处理的粘贴负载和
    /// 提及链接目标。恢复之前已提交的、必须保留带符号提及目标解析的
    /// 草稿的调用方应改用
    /// [`Self::set_text_content_with_mention_bindings`]。
    pub(crate) fn set_text_content(
        &mut self,
        text: String,
        text_elements: Vec<TextElement>,
        local_image_paths: Vec<PathBuf>,
    ) {
        self.set_text_content_with_mention_bindings(
            text,
            text_elements,
            local_image_paths,
            Vec::new(),
        );
    }

    /// 替换整个编辑器内容，同时恢复提及链接目标。
    ///
    /// 提及弹出窗口插入既存储可见文本（例如 `$file`）
    /// 也存储用于在
    /// 提交时解析规范目标的隐藏提及绑定。恢复被打断或阻塞的
    /// 草稿时请使用此方法；如果调用方只恢复文本和图像，提及可能对用户
    /// 看起来完整，却在解析为错误目标或在重试时
    /// 丢失。
    ///
    /// 此辅助方法有意将光标放在恢复文本的开头。需要行尾恢复行为
    /// （例如 shell 风格历史召回）的调用方应在
    /// 此方法后调用 [`Self::move_cursor_to_end`]。
    pub(crate) fn set_text_content_with_mention_bindings(
        &mut self,
        text: String,
        text_elements: Vec<TextElement>,
        local_image_paths: Vec<PathBuf>,
        mention_bindings: Vec<MentionBinding>,
    ) {
        // 首先清除任何现有内容、占位符和附件。
        self.draft.textarea.set_text_clearing_elements("");
        self.draft.is_bash_mode = false;
        self.draft.pending_pastes.clear();
        self.draft.mention_bindings.clear();

        let (text, text_elements) = self.imported_text_for_textarea(text, text_elements);
        self.draft
            .textarea
            .set_text_with_elements(&text, &text_elements);
        self.attachments
            .reset_local_images(local_image_paths, &mut self.draft.textarea);

        self.bind_mentions_from_snapshot(mention_bindings);
        self.draft.textarea.set_cursor(/*pos*/ 0);
        self.sync_popups();
    }

    fn current_cursor(&self) -> usize {
        self.draft.textarea.cursor() + if self.draft.is_bash_mode { 1 } else { 0 }
    }

    #[cfg(test)]
    pub(crate) fn cursor(&self) -> usize {
        self.current_cursor()
    }

    fn history_navigation_cursor(&self) -> usize {
        if self.draft.is_bash_mode && self.draft.textarea.cursor() == 0 {
            0
        } else if self.draft.textarea.is_vim_normal_mode()
            && !self.draft.textarea.text().is_empty()
            && self.draft.textarea.cursor() == self.draft.textarea.vim_normal_end_cursor()
        {
            self.current_text().len()
        } else {
            self.current_cursor()
        }
    }

    fn set_current_cursor(&mut self, cursor: usize) {
        let visible_cursor = if self.draft.is_bash_mode {
            cursor.saturating_sub(1)
        } else {
            cursor
        };
        self.draft
            .textarea
            .set_cursor(visible_cursor.min(self.draft.textarea.text().len()));
    }

    fn current_text_elements(&self) -> Vec<TextElement> {
        let shift = if self.draft.is_bash_mode { 1 } else { 0 };
        self.draft
            .textarea
            .text_elements()
            .into_iter()
            .filter_map(|element| Self::shift_text_element(element, shift))
            .collect()
    }

    fn shift_text_element(element: TextElement, shift: isize) -> Option<TextElement> {
        let start = element.byte_range.start.checked_add_signed(shift)?;
        let end = element.byte_range.end.checked_add_signed(shift)?;
        if start >= end {
            return None;
        }

        Some(element.map_range(|_| (start..end).into()))
    }

    fn snapshot_draft(&self) -> ComposerDraft {
        ComposerDraft {
            text: self.current_text(),
            text_elements: self.current_text_elements(),
            local_image_paths: self.attachments.local_image_paths(),
            remote_image_urls: self.attachments.remote_image_urls(),
            mention_bindings: self.snapshot_mention_bindings(),
            pending_pastes: self.draft.pending_pastes.clone(),
            cursor: self.current_cursor(),
        }
    }

    fn restore_draft(&mut self, draft: ComposerDraft) {
        let ComposerDraft {
            text,
            text_elements,
            local_image_paths,
            remote_image_urls,
            mention_bindings,
            pending_pastes,
            cursor,
        } = draft;
        self.set_remote_image_urls(remote_image_urls);
        self.set_text_content_with_mention_bindings(
            text,
            text_elements,
            local_image_paths,
            mention_bindings,
        );
        self.set_pending_pastes(pending_pastes);
        self.set_current_cursor(cursor);
        self.sync_popups();
    }

    /// 在不更改输入启用状态的情况下更新占位符文本。
    pub(crate) fn set_placeholder_text(&mut self, placeholder: String) {
        self.placeholder_text = placeholder;
    }

    pub(crate) fn set_parent_owned_thread(&mut self) {
        self.blocks_direct_input = true;
        self.placeholder_text = "Viewing sub-agent — direct input is disabled".to_string();
    }

    /// 将光标移动到当前文本缓冲区的末尾。
    pub(crate) fn move_cursor_to_end(&mut self) {
        self.draft
            .textarea
            .set_cursor(self.draft.textarea.text().len());
        self.sync_popups();
    }

    fn move_cursor_to_history_entry_end(&mut self) {
        let cursor = if self.draft.textarea.is_vim_normal_mode() {
            self.draft.textarea.vim_normal_end_cursor()
        } else {
            self.draft.textarea.text().len()
        };
        self.draft.textarea.set_cursor(cursor);
        self.sync_popups();
    }

    /// 将规范编辑器文本转换为文本区域的内部表示。
    ///
    /// Shell 模式将前导 `!` 存储为提示符状态而非可编辑文本，
    /// 因此整缓冲区导入必须在重建文本区域前吸收该前缀。
    fn imported_text_for_textarea(
        &mut self,
        text: String,
        text_elements: Vec<TextElement>,
    ) -> (String, Vec<TextElement>) {
        if let Some(stripped) = text.strip_prefix('!') {
            self.draft.is_bash_mode = true;
            (
                stripped.to_string(),
                text_elements
                    .into_iter()
                    .filter_map(|element| Self::shift_text_element(element, /*shift*/ -1))
                    .collect(),
            )
        } else {
            self.draft.is_bash_mode = false;
            (text, text_elements)
        }
    }

    pub(crate) fn clear_for_ctrl_c(&mut self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let previous = self.current_text();
        let text_elements = self.current_text_elements();
        let local_image_paths = self.attachments.local_image_paths();
        let pending_pastes = std::mem::take(&mut self.draft.pending_pastes);
        let remote_image_urls = self.attachments.remote_image_urls();
        let mention_bindings = self.snapshot_mention_bindings();
        self.set_text_content(String::new(), Vec::new(), Vec::new());
        self.attachments.clear_remote_image_urls();
        self.history.reset_navigation();
        self.history.record_local_submission(HistoryEntry {
            text: previous.clone(),
            text_elements,
            local_image_paths,
            remote_image_urls,
            mention_bindings,
            pending_pastes,
        });
        Some(previous)
    }

    /// 获取当前编辑器文本。
    pub(crate) fn current_text(&self) -> String {
        if self.draft.is_bash_mode {
            format!("!{}", self.draft.textarea.text())
        } else {
            self.draft.textarea.text().to_string()
        }
    }

    /// 将历史条目恢复进编辑器，并采用类似 shell 的光标放置。
    ///
    /// 此路径恢复文本、元素、图像、提及绑定和待处理粘贴负载，
    /// 然后将光标移动到活动模式的历史边界。如果调用方为历史召回
    /// 直接复用了 [`Self::set_text_content_with_mention_bindings`] 却忘记
    /// 最终的光标移动，重复的 Up/Down 将停止导航历史，因为光标门控
    /// 将内部位置视为普通编辑模式。
    fn apply_history_entry(&mut self, entry: HistoryEntry) {
        let HistoryEntry {
            text,
            text_elements,
            local_image_paths,
            remote_image_urls,
            mention_bindings,
            pending_pastes,
        } = entry;
        self.set_remote_image_urls(remote_image_urls);
        self.set_text_content_with_mention_bindings(
            text,
            text_elements,
            local_image_paths,
            mention_bindings,
        );
        self.set_pending_pastes(pending_pastes);
        self.move_cursor_to_history_entry_end();
    }

    pub(crate) fn text_elements(&self) -> Vec<TextElement> {
        self.current_text_elements()
    }

    pub(crate) fn draft_snapshot(&self) -> ComposerDraftSnapshot {
        ComposerDraftSnapshot {
            text: self.current_text(),
            text_elements: self.text_elements(),
            local_images: self.local_images(),
            remote_image_urls: self.remote_image_urls(),
            mention_bindings: self.mention_bindings(),
            pending_pastes: self.pending_pastes(),
        }
    }

    #[cfg(test)]
    pub(crate) fn local_image_paths(&self) -> Vec<PathBuf> {
        self.attachments.local_image_paths()
    }

    #[cfg(test)]
    pub(crate) fn status_line_text(&self) -> Option<String> {
        self.footer.status_line_text()
    }

    pub(crate) fn local_images(&self) -> Vec<LocalImageAttachment> {
        self.attachments.local_images()
    }

    pub(crate) fn mention_bindings(&self) -> Vec<MentionBinding> {
        self.snapshot_mention_bindings()
    }

    pub(crate) fn take_recent_submission_mention_bindings(&mut self) -> Vec<MentionBinding> {
        std::mem::take(&mut self.draft.recent_submission_mention_bindings)
    }

    /// 将暂存的斜杠命令草稿提交到本地上箭头召回。
    ///
    /// 在命令分派后调用。多次调用无害，因为待处理
    /// 槽位在第一次调用时就被消费。
    pub(crate) fn record_pending_slash_command_history(&mut self) {
        if let Some(entry) = self.pending_slash_command_history.take() {
            self.history.record_local_submission(entry);
        }
    }

    /// 插入附件占位符并跟踪它以便下次提交。
    pub fn attach_image(&mut self, path: PathBuf) {
        self.attachments
            .attach_image(&mut self.draft.textarea, path);
    }

    #[cfg(test)]
    pub fn take_recent_submission_images(&mut self) -> Vec<PathBuf> {
        self.attachments.take_recent_submission_images()
    }

    pub fn take_recent_submission_images_with_placeholders(&mut self) -> Vec<LocalImageAttachment> {
        self.attachments
            .take_recent_submission_images_with_placeholders()
    }

    /// 刷新任何到期的粘贴突发状态。
    ///
    /// 从 UI 滴答调用此方法，将粘贴突发瞬态转换为显式文本区域编辑：
    ///
    /// - 如果突发超时，通过 `handle_paste(String)` 刷新它。
    /// - 如果只持有第一个 ASCII 字符（闪烁抑制）且没有后续突发，则作为正常输入发出它。
    ///
    /// 这也允许单个"被持有"的 ASCII 字符即使原来不是
    /// 粘贴突发的一部分也能渲染。
    pub(crate) fn flush_paste_burst_if_due(&mut self) -> bool {
        self.handle_paste_burst_flush(Instant::now())
    }

    /// 返回编辑器当前是否处于任何与粘贴突发相关的瞬态状态。
    ///
    /// 这包括主动缓冲、具有非空突发缓冲区，或为闪烁抑制持有第一个
    /// ASCII 字符。
    pub(crate) fn is_in_paste_burst(&self) -> bool {
        self.draft.paste_burst.is_active()
    }

    /// 返回一个可靠超过粘贴突发计时阈值的延迟。
    ///
    /// 在测试中使用，以避免围绕 `PasteBurst` 超时的边界抖动。
    pub(crate) fn recommended_paste_flush_delay() -> Duration {
        PasteBurst::recommended_flush_delay()
    }

    /// 集成异步文件搜索的结果。
    pub(crate) fn on_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        // 仅当用户仍在编辑以 `query` 开头的令牌时应用。
        let current_opt = if self.mentions_v2_enabled {
            self.current_mentions_v2_token()
        } else {
            Self::current_at_token(&self.draft.textarea)
        };
        let Some(current_token) = current_opt else {
            return;
        };

        if !current_token.starts_with(&query) {
            return;
        }

        match &mut self.popups.active {
            ActivePopup::File(popup) => {
                popup.set_matches(&query, matches);
            }
            ActivePopup::MentionV2(popup) => {
                popup.set_file_matches(&query, matches);
            }
            _ => {}
        }
    }

    /// 为 `key` 显示瞬态"再次按下退出"提示。
    ///
    /// 所有者（`BottomPane`/`ChatWidget`）负责在 [`super::QUIT_SHORTCUT_TIMEOUT`]
    /// 之后调度重绘，以便即使 UI 空闲时提示也能消失。
    pub fn show_quit_shortcut_hint(&mut self, key: KeyBinding, has_focus: bool) {
        self.footer.quit_shortcut_expires_at = Instant::now()
            .checked_add(super::QUIT_SHORTCUT_TIMEOUT)
            .or_else(|| Some(Instant::now()));
        self.footer.quit_shortcut_key = key;
        self.footer.mode = FooterMode::QuitShortcutReminder;
        self.set_has_focus(has_focus);
    }

    /// 立即清除"再次按下退出"提示。
    pub fn clear_quit_shortcut_hint(&mut self, has_focus: bool) {
        self.footer.quit_shortcut_expires_at = None;
        self.footer.mode = reset_mode_after_activity(self.footer.mode);
        self.set_has_focus(has_focus);
    }

    /// 退出快捷方式提示当前是否应显示。
    ///
    /// 这是基于时间而非基于事件的：它可能在没有
    /// 任何额外用户输入的情况下变为 false，因此 UI 会在提示
    /// 过期时调度重绘。
    pub(crate) fn quit_shortcut_hint_visible(&self) -> bool {
        self.footer
            .quit_shortcut_expires_at
            .is_some_and(|expires_at| Instant::now() < expires_at)
    }

    fn next_large_paste_placeholder(&self, char_count: usize) -> String {
        let base = format!("[Pasted Content {char_count} chars]");
        let prefix = format!("{base} #");
        let mut max_suffix = 0usize;

        for (placeholder, _) in &self.draft.pending_pastes {
            if placeholder == &base {
                max_suffix = max_suffix.max(1);
                continue;
            }
            if let Some(suffix) = placeholder.strip_prefix(&prefix)
                && let Ok(value) = suffix.parse::<usize>()
            {
                max_suffix = max_suffix.max(value);
            }
        }

        if max_suffix == 0 {
            base
        } else {
            format!("{base} #{}", max_suffix + 1)
        }
    }

    pub(crate) fn insert_str(&mut self, text: &str) {
        self.draft.textarea.insert_str(text);
        self.sync_bash_mode_from_text();
        self.sync_popups();
    }

    /// 处理来自主 UI 的按键事件。
    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> (InputResult, bool) {
        if !self.draft.input_enabled {
            return (InputResult::None, false);
        }

        if matches!(key_event.kind, KeyEventKind::Release) {
            return (InputResult::None, false);
        }

        if self.history_search.is_some() {
            return self.handle_history_search_key(key_event);
        }

        if Self::is_history_search_key(&key_event, &self.history_search_previous_keys) {
            return self.begin_history_search();
        }

        let result = match &mut self.popups.active {
            ActivePopup::Command(_) => self.handle_key_event_with_slash_popup(key_event),
            ActivePopup::File(_) => self.handle_key_event_with_file_popup(key_event),
            ActivePopup::Skill(_) => self.handle_key_event_with_skill_popup(key_event),
            ActivePopup::MentionV2(_) => self.handle_key_event_with_mentions_v2_popup(key_event),
            ActivePopup::None => self.handle_key_event_without_popup(key_event),
        };
        self.reset_vim_mode_after_successful_dispatch(&result.0);
        // 处理按键后更新（或隐藏/显示）弹出窗口。
        self.sync_popups();
        result
    }

    /// 返回是否有任何弹出窗口或历史搜索处于活动状态。
    pub(crate) fn popup_active(&self) -> bool {
        self.history_search.is_some() || self.popups.active()
    }

    #[inline]
    fn clamp_to_char_boundary(text: &str, pos: usize) -> usize {
        let mut p = pos.min(text.len());
        if p < text.len() && !text.is_char_boundary(p) {
            p = text
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= p)
                .last()
                .unwrap_or(0);
        }
        p
    }

    /// 处理非 ASCII 字符输入（通常是 IME），同时仍支持粘贴突发检测。
    ///
    /// 此处理器存在是因为非 ASCII 输入通常来自 IME，其中的字符可以
    /// 合法地在短突发中到达，**不应**被视为粘贴。
    ///
    /// 与 ASCII 路径的关键区别：
    ///
    /// - 我们从不持有第一个字符（`PasteBurst::on_plain_char_no_hold`），因为持有
    ///   非 ASCII 字符会感觉像丢字。
    /// - 如果检测到突发，我们可能需要追溯性地移除光标前
    ///   已插入的文本并将其移入粘贴缓冲区（见 `PasteBurst::decide_begin_buffer`）。
    ///
    /// 因为此路径混合了"立即插入"与"可能稍后追溯抓取"，在切片 `textarea.text()`
    /// 前必须将光标固定到 UTF-8 字符边界。
    #[inline]
    fn handle_non_ascii_char(&mut self, input: KeyEvent, now: Instant) -> (InputResult, bool) {
        if self.draft.disable_paste_burst {
            // 禁用突发检测时，将 IME/非 ASCII 输入视为正常输入。
            // 特别是，不要追溯捕获或缓冲已插入的前缀文本。
            self.draft.textarea.input(input);
            let text_after = self.draft.textarea.text();
            self.draft
                .pending_pastes
                .retain(|(placeholder, _)| text_after.contains(placeholder));
            return (InputResult::None, true);
        }
        if let KeyEvent {
            code: KeyCode::Char(ch),
            ..
        } = input
        {
            if self.draft.paste_burst.try_append_char_if_active(ch, now) {
                return (InputResult::None, true);
            }
            // 非 ASCII 输入通常来自 IME，并且可能以快速突发到达。
            // 我们不想在此路径上持有第一个字符（闪烁抑制），但我们
            // 仍然想要检测类似粘贴的突发。在应用任何非 ASCII 输入之前，刷新
            // 任何现有的突发缓冲区（包括来自 ASCII 路径的待处理第一个字符），以便
            // 不把该瞬态状态带向前。
            if let Some(pasted) = self.draft.paste_burst.flush_before_modified_input() {
                self.handle_paste(pasted);
            }
            if let Some(decision) = self.draft.paste_burst.on_plain_char_no_hold(now) {
                match decision {
                    CharDecision::BufferAppend => {
                        self.draft.paste_burst.append_char_to_buffer(ch, now);
                        return (InputResult::None, true);
                    }
                    CharDecision::BeginBuffer { retro_chars } => {
                        // 对于非 ASCII，我们之前立即插入了字符，所以如果结果
                        // 是类似粘贴的，我们需要追溯性地抓取并移除
                        // 已从文本区域插入的前缀，然后再缓冲突发。
                        let cur = self.draft.textarea.cursor();
                        let txt = self.draft.textarea.text();
                        let safe_cur = Self::clamp_to_char_boundary(txt, cur);
                        let before = &txt[..safe_cur];
                        if let Some(grab) = self.draft.paste_burst.decide_begin_buffer(
                            now,
                            before,
                            retro_chars as usize,
                        ) {
                            if !grab.grabbed.is_empty() {
                                self.draft
                                    .textarea
                                    .replace_range(grab.start_byte..safe_cur, "");
                            }
                            // 用一切（抓取到的 + 新的）为粘贴突发缓冲区播种
                            self.draft.paste_burst.append_char_to_buffer(ch, now);
                            return (InputResult::None, true);
                        }
                        // 如果 decide_begin_buffer 选择不开始缓冲，
                        // 则落入下面的正常插入。
                    }
                    _ => unreachable!("on_plain_char_no_hold returned unexpected variant"),
                }
            }
        }
        if let Some(pasted) = self.draft.paste_burst.flush_before_modified_input() {
            self.handle_paste(pasted);
        }
        self.draft.textarea.input(input);

        let text_after = self.draft.textarea.text();
        self.draft
            .pending_pastes
            .retain(|(placeholder, _)| text_after.contains(placeholder));
        (InputResult::None, true)
    }

    /// 处理文件搜索弹出窗口可见时的按键事件。
    fn handle_key_event_with_file_popup(&mut self, key_event: KeyEvent) -> (InputResult, bool) {
        if self.handle_shortcut_overlay_key(&key_event) {
            return (InputResult::None, true);
        }
        if key_event.code == KeyCode::Esc {
            let next_mode = esc_hint_mode(self.footer.mode, self.is_task_running);
            if next_mode != self.footer.mode {
                self.footer.mode = next_mode;
                return (InputResult::None, true);
            }
        } else {
            self.footer.mode = reset_mode_after_activity(self.footer.mode);
        }
        let ActivePopup::File(popup) = &mut self.popups.active else {
            unreachable!();
        };

        match key_event {
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                popup.move_up();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                popup.move_down();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                if let Some((range, query)) = completion_target::current_prefixed_token_range(
                    &self.draft.textarea,
                    '@',
                    /*allow_empty*/ false,
                ) {
                    self.popups.dismissed_file_token =
                        Some(DismissedToken::new(&self.draft.textarea, range, query));
                }
                self.popups.active = ActivePopup::None;
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                let Some(sel) = popup.selected_match() else {
                    self.popups.active = ActivePopup::None;
                    return if key_event.code == KeyCode::Enter {
                        self.handle_key_event_without_popup(key_event)
                    } else {
                        (InputResult::None, true)
                    };
                };

                let sel_path = sel.to_string_lossy().to_string();
                if let Some((token_range, _)) =
                    self.current_editable_at_token_range_with_options(/*allow_empty*/ false)
                {
                    self.insert_selected_file_path(token_range, &sel_path);
                }
                self.popups.active = ActivePopup::None;
                (InputResult::None, true)
            }
            input => self.handle_input_basic(input),
        }
    }

    /// 处理遗留技能提及弹出窗口可见时的按键事件。
    fn handle_key_event_with_skill_popup(&mut self, key_event: KeyEvent) -> (InputResult, bool) {
        if self.handle_shortcut_overlay_key(&key_event) {
            return (InputResult::None, true);
        }
        self.footer.mode = reset_mode_after_activity(self.footer.mode);

        let ActivePopup::Skill(popup) = &mut self.popups.active else {
            unreachable!();
        };

        let mut selected_mention: Option<(String, Option<String>)> = None;
        let mut close_popup = false;

        let result = match key_event {
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                popup.move_up();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                popup.move_down();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                if let Some(target) = self.current_mention_target() {
                    self.popups.dismissed_mention_token = Some(DismissedToken::new(
                        &self.draft.textarea,
                        target.range,
                        target.query,
                    ));
                }
                self.popups.active = ActivePopup::None;
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if let Some(mention) = popup.selected_mention() {
                    selected_mention = Some((mention.insert_text.clone(), mention.path.clone()));
                }
                close_popup = true;
                (InputResult::None, true)
            }
            input => self.handle_input_basic(input),
        };

        if close_popup {
            if let Some((insert_text, path)) = selected_mention
                && let Some(target) = self.current_mention_target()
            {
                self.insert_selected_mention(target.range, &insert_text, path.as_deref());
            }
            self.popups.active = ActivePopup::None;
        }

        result
    }

    fn handle_key_event_with_mentions_v2_popup(
        &mut self,
        key_event: KeyEvent,
    ) -> (InputResult, bool) {
        if self.handle_shortcut_overlay_key(&key_event) {
            return (InputResult::None, true);
        }
        self.footer.mode = reset_mode_after_activity(self.footer.mode);
        let can_switch_search_mode = self.current_editable_at_token().is_some();

        let ActivePopup::MentionV2(popup) = &mut self.popups.active else {
            unreachable!();
        };

        let mut selected: Option<MentionV2Selection> = None;
        let mut close_popup = false;
        let mut submit_without_popup = false;

        let result = match key_event {
            KeyEvent {
                code: KeyCode::Up, ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                popup.move_up();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Down,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                popup.move_down();
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if can_switch_search_mode {
                    popup.previous_search_mode();
                    (InputResult::None, true)
                } else {
                    self.handle_input_basic(key_event)
                }
            }
            KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if can_switch_search_mode {
                    popup.next_search_mode();
                    (InputResult::None, true)
                } else {
                    self.handle_input_basic(key_event)
                }
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                if let Some((range, query)) = self.current_mentions_v2_token_range() {
                    self.popups.dismissed_mention_token =
                        Some(DismissedToken::new(&self.draft.textarea, range, query));
                }
                self.popups.active = ActivePopup::None;
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                selected = popup.selected();
                close_popup = true;
                (InputResult::None, true)
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                selected = popup.selected();
                close_popup = true;
                submit_without_popup = selected.is_none();
                (InputResult::None, true)
            }
            input => self.handle_input_basic(input),
        };

        if close_popup {
            let token_range = self
                .current_editable_at_token_range_with_options(/*allow_empty*/ true)
                .map(|(range, _)| range);
            if let (Some(selected), Some(token_range)) = (selected, token_range) {
                match selected {
                    MentionV2Selection::File(path) => {
                        self.insert_selected_file_path(
                            token_range,
                            path.to_string_lossy().as_ref(),
                        );
                    }
                    MentionV2Selection::Tool { insert_text, path } => {
                        self.insert_selected_mention(token_range, &insert_text, path.as_deref());
                    }
                }
            }
            self.popups.active = ActivePopup::None;
            if submit_without_popup {
                return self.handle_key_event_without_popup(key_event);
            }
        }

        result
    }

    fn is_image_path(path: &str) -> bool {
        let lower = path.to_ascii_lowercase();
        lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".gif")
            || lower.ends_with(".webp")
    }

    /// 在补全后将光标留在一个水平分隔符之后。
    ///
    /// 在任何非空白后缀之前保留另一个分隔符，以便后续输入
    /// 不会并入它。换行符从不作为分隔符复用；在它们之前插入
    /// 空格，以便后续输入保持在已补全令牌所在的行上。
    fn advance_past_completion_separator(&mut self) {
        let cursor = self.draft.textarea.cursor();
        let existing_separator_len = self.draft.textarea.text()[cursor..]
            .chars()
            .next()
            .filter(|c| {
                c.is_whitespace()
                    && !matches!(
                        *c,
                        '\n' | '\r'
                            | '\u{000B}'
                            | '\u{000C}'
                            | '\u{0085}'
                            | '\u{2028}'
                            | '\u{2029}'
                    )
            })
            .map(char::len_utf8);
        if let Some(separator_len) = existing_separator_len {
            let after_separator = cursor + separator_len;
            let separator_precedes_suffix = self.draft.textarea.text()[after_separator..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace());
            if separator_precedes_suffix {
                self.draft.textarea.insert_str(" ");
            } else {
                self.draft.textarea.set_cursor(after_separator);
            }
        } else {
            self.draft.textarea.insert_str(" ");
        }
    }

    /// 仅对刚插入的精确令牌出现关闭弹出同步。
    ///
    /// 同时匹配范围和文本可防止草稿中后面相同令牌
    /// 继承已补全令牌的关闭状态。
    fn dismiss_completed_prefixed_token(
        &mut self,
        prefix: char,
        inserted_range: Range<usize>,
        inserted_text: &str,
    ) {
        let Some(completed_token) = inserted_text.strip_prefix(prefix) else {
            return;
        };
        // 补全将光标留在分隔符空白上，否则正常的令牌亲和性会
        // 立即为带符号前缀的插入文本重新打开弹出窗口。
        let Some((current_range, current_token)) = completion_target::current_prefixed_token_range(
            &self.draft.textarea,
            prefix,
            /*allow_empty*/ true,
        ) else {
            return;
        };
        if current_range != inserted_range || current_token != completed_token {
            return;
        }

        if prefix == '@' && !self.mentions_v2_enabled {
            self.popups.dismissed_file_token = Some(DismissedToken::new(
                &self.draft.textarea,
                current_range,
                current_token,
            ));
        } else {
            self.popups.dismissed_mention_token = Some(DismissedToken::new(
                &self.draft.textarea,
                current_range,
                current_token,
            ));
        }
    }

    fn insert_selected_file_path(&mut self, token_range: Range<usize>, selected_path: &str) {
        let token_range = self.separate_completion_from_adjacent_element(token_range);
        if Self::is_image_path(selected_path) {
            let path_buf = PathBuf::from(selected_path);
            match image::image_dimensions(&path_buf) {
                Ok((width, height)) => {
                    tracing::debug!("selected image dimensions={}x{}", width, height);
                    let start_idx = token_range.start;
                    self.draft.textarea.replace_range(token_range, "");
                    self.draft.textarea.set_cursor(start_idx);
                    self.attach_image(path_buf);
                    self.advance_past_completion_separator();
                }
                Err(err) => {
                    tracing::trace!("image dimensions lookup failed: {err}");
                    self.insert_selected_path(token_range, selected_path);
                }
            }
        } else {
            self.insert_selected_path(token_range, selected_path);
        }
    }

    fn trim_text_elements(
        original: &str,
        trimmed: &str,
        elements: Vec<TextElement>,
    ) -> Vec<TextElement> {
        if trimmed.is_empty() || elements.is_empty() {
            return Vec::new();
        }
        let trimmed_start = original.len().saturating_sub(original.trim_start().len());
        let trimmed_end = trimmed_start.saturating_add(trimmed.len());

        elements
            .into_iter()
            .filter_map(|elem| {
                let start = elem.byte_range.start;
                let end = elem.byte_range.end;
                if end <= trimmed_start || start >= trimmed_end {
                    return None;
                }
                let new_start = start.saturating_sub(trimmed_start);
                let new_end = end.saturating_sub(trimmed_start).min(trimmed.len());
                if new_start >= new_end {
                    return None;
                }
                let placeholder = trimmed.get(new_start..new_end).map(str::to_string);
                Some(TextElement::new(
                    ByteRange {
                        start: new_start,
                        end: new_end,
                    },
                    placeholder,
                ))
            })
            .collect()
    }

    /// 使用元素范围展开大粘贴占位符并重建其他元素跨度。
    pub(crate) fn expand_pending_pastes(
        text: &str,
        mut elements: Vec<TextElement>,
        pending_pastes: &[(String, String)],
    ) -> (String, Vec<TextElement>) {
        if pending_pastes.is_empty() || elements.is_empty() {
            return (text.to_string(), elements);
        }

        // 阶段 1：按占位符为待处理的粘贴负载建立索引，以便确定性替换。
        let mut pending_by_placeholder: HashMap<&str, VecDeque<&str>> = HashMap::new();
        for (placeholder, actual) in pending_pastes {
            pending_by_placeholder
                .entry(placeholder.as_str())
                .or_default()
                .push_back(actual.as_str());
        }

        // 阶段 2：按顺序遍历元素，并在单次遍历中重建文本/跨度。
        elements.sort_by_key(|elem| elem.byte_range.start);

        let mut rebuilt = String::with_capacity(text.len());
        let mut rebuilt_elements = Vec::with_capacity(elements.len());
        let mut cursor = 0usize;

        for elem in elements {
            let start = elem.byte_range.start.min(text.len());
            let end = elem.byte_range.end.min(text.len());
            if start > end {
                continue;
            }
            if start > cursor {
                rebuilt.push_str(&text[cursor..start]);
            }
            let elem_text = &text[start..end];
            let placeholder = elem.placeholder(text).map(str::to_string);
            let replacement = placeholder
                .as_deref()
                .and_then(|ph| pending_by_placeholder.get_mut(ph))
                .and_then(VecDeque::pop_front);
            if let Some(actual) = replacement {
                // 阶段 3：内联实际粘贴负载并丢弃其占位符元素。
                rebuilt.push_str(actual);
            } else {
                // 阶段 4：保留非粘贴元素，为新文本更新其字节范围。
                let new_start = rebuilt.len();
                rebuilt.push_str(elem_text);
                let new_end = rebuilt.len();
                let placeholder = placeholder.or_else(|| Some(elem_text.to_string()));
                rebuilt_elements.push(TextElement::new(
                    ByteRange {
                        start: new_start,
                        end: new_end,
                    },
                    placeholder,
                ));
            }
            cursor = end;
        }

        // 阶段 5：追加跟随最后一个元素的任何尾随文本。
        if cursor < text.len() {
            rebuilt.push_str(&text[cursor..]);
        }

        (rebuilt, rebuilt_elements)
    }

    pub fn skills(&self) -> Option<&Vec<SkillMetadata>> {
        self.skills.as_ref()
    }

    pub fn plugins(&self) -> Option<&Vec<PluginCapabilitySummary>> {
        self.plugins.as_ref()
    }

    fn mentions_enabled(&self) -> bool {
        let skills_ready = self
            .skills
            .as_ref()
            .is_some_and(|skills| !skills.is_empty());
        let plugins_ready = self
            .plugins
            .as_ref()
            .is_some_and(|plugins| !plugins.is_empty());
        let connectors_ready = self.connectors_enabled
            && self
                .connectors_snapshot
                .as_ref()
                .is_some_and(|snapshot| !snapshot.connectors.is_empty());
        skills_ready || plugins_ready || connectors_ready
    }

    fn current_prefixed_token(
        textarea: &TextArea,
        prefix: char,
        allow_empty: bool,
    ) -> Option<String> {
        completion_target::current_prefixed_token_range(textarea, prefix, allow_empty)
            .map(|(_, token)| token)
    }

    /// 提取光标当前所在的 `@token`（如果有）。
    ///
    /// 返回的字符串**不**包含前导 `@`。
    fn current_at_token(textarea: &TextArea) -> Option<String> {
        Self::current_prefixed_token(textarea, '@', /*allow_empty*/ false)
    }

    /// 仅当活动的带前缀令牌的符号和名称保持为可编辑纯文本时返回它。
    ///
    /// 原子元素和其提及前缀已为原子的令牌被排除，因此已绑定的
    /// 提及不会再次被提供补全。
    fn current_editable_prefixed_token_range(
        &self,
        prefix: char,
        allow_empty: bool,
    ) -> Option<(Range<usize>, String)> {
        let (range, token) = completion_target::current_prefixed_token_range(
            &self.draft.textarea,
            prefix,
            allow_empty,
        )?;
        completion_target::prefixed_token_range_is_editable(
            &self.draft.textarea,
            prefix,
            &range,
            &token,
        )
        .then_some((range, token))
    }

    fn current_editable_at_token_range_with_options(
        &self,
        allow_empty: bool,
    ) -> Option<(Range<usize>, String)> {
        self.current_editable_prefixed_token_range('@', allow_empty)
    }

    fn current_editable_at_token_with_options(&self, allow_empty: bool) -> Option<String> {
        self.current_editable_at_token_range_with_options(allow_empty)
            .map(|(_, token)| token)
    }

    fn current_editable_at_token(&self) -> Option<String> {
        self.current_editable_at_token_with_options(/*allow_empty*/ false)
    }

    fn current_mentions_v2_token_range(&self) -> Option<(Range<usize>, String)> {
        if !self.mentions_v2_enabled {
            return None;
        }
        self.current_editable_at_token_range_with_options(/*allow_empty*/ true)
    }

    fn current_mentions_v2_token(&self) -> Option<String> {
        self.current_mentions_v2_token_range()
            .map(|(_, token)| token)
    }

    /// 在不急切克隆提及目录的情况下解析活动的遗留 `$` 目标。
    ///
    /// 明确的 shell 语法被拒绝。仅当可绑定的提及匹配时才接受
    /// 模糊的类 shell 查询。在该检查期间构建的与查询无关的目录
    /// 仅在所选目标也模糊时才向前携带。
    fn current_mention_target(&self) -> Option<MentionCompletionTarget> {
        if !self.mentions_enabled() {
            return None;
        }
        let mentions: OnceCell<Vec<MentionItem>> = OnceCell::new();
        let (range, query) = {
            let dollar_query_is_completable =
                |query: &str| match completion_target::dollar_query_kind(query) {
                    completion_target::DollarQueryKind::Completable => true,
                    completion_target::DollarQueryKind::AmbiguousShellParameter => mentions
                        .get_or_init(|| {
                            self.mention_items()
                                .into_iter()
                                .filter(|mention| {
                                    mention.path.is_some()
                                        && Self::mention_token_from_insert_text(
                                            &mention.insert_text,
                                        )
                                        .is_some()
                                })
                                .collect()
                        })
                        .iter()
                        .any(|mention| mention.fuzzy_match_query(query).is_some()),
                    completion_target::DollarQueryKind::ShellVariable
                    | completion_target::DollarQueryKind::DefiniteShellParameter
                    | completion_target::DollarQueryKind::Invalid => false,
                };
            let (range, query) =
                completion_target::current_prefixed_token_range_with_dollar_predicate(
                    &self.draft.textarea,
                    '$',
                    /*allow_empty*/ true,
                    dollar_query_is_completable,
                )?;
            if !completion_target::prefixed_token_range_is_editable(
                &self.draft.textarea,
                '$',
                &range,
                &query,
            ) || !dollar_query_is_completable(&query)
            {
                return None;
            }
            (range, query)
        };
        let prebuilt_mentions = if matches!(
            completion_target::dollar_query_kind(&query),
            completion_target::DollarQueryKind::AmbiguousShellParameter
        ) {
            mentions.into_inner()
        } else {
            None
        };

        Some(MentionCompletionTarget {
            range,
            query,
            prebuilt_mentions,
        })
    }

    /// 用 `path` 替换活动的 `@token`（光标下的那个）。
    fn insert_selected_path(&mut self, token_range: Range<usize>, path: &str) {
        // 如果路径包含空白，用双引号包裹它，以便
        // 本地提示参数解析器将其视为单个参数。避免在路径已包含引号时
        // 添加引号以保持行为简单。
        let needs_quotes = path.chars().any(char::is_whitespace);
        let inserted = if needs_quotes && !path.contains('"') {
            format!("\"{path}\"")
        } else {
            path.to_string()
        };

        // 仅替换活动的 `@token`，以便无关的文本元素（例如
        // 大粘贴占位符）保持原子性，仍可在提交时展开。
        let start_idx = token_range.start;
        self.draft.textarea.replace_range(token_range, &inserted);
        let inserted_range = start_idx..start_idx.saturating_add(inserted.len());
        self.draft.textarea.set_cursor(inserted_range.end);
        self.advance_past_completion_separator();
        self.dismiss_completed_prefixed_token('@', inserted_range, &inserted);
    }

    fn insert_selected_mention(
        &mut self,
        token_range: Range<usize>,
        insert_text: &str,
        path: Option<&str>,
    ) {
        let token_range = self.separate_completion_from_adjacent_element(token_range);
        // 移除活动令牌并将选中的提及作为原子元素插入。
        let start_idx = token_range.start;
        self.draft.textarea.replace_range(token_range, "");
        self.draft.textarea.set_cursor(start_idx);
        let id = self.draft.textarea.insert_element(insert_text);
        let inserted_range = start_idx..start_idx.saturating_add(insert_text.len());

        if let (Some(path), Some((sigil, mention))) =
            (path, Self::mention_token_from_insert_text(insert_text))
        {
            self.draft.mention_bindings.insert(
                id,
                ComposerMentionBinding {
                    sigil,
                    mention,
                    path: path.to_string(),
                },
            );
        }

        self.advance_past_completion_separator();
        if let Some(sigil) = insert_text.chars().next()
            && matches!(sigil, '$' | '@')
        {
            self.dismiss_completed_prefixed_token(sigil, inserted_range, insert_text);
        }
    }

    /// 当补全直接在原子元素之后开始时插入前导分隔符。
    ///
    /// 返回的范围被移位，以在插入后保持替换相同的可编辑令牌。
    fn separate_completion_from_adjacent_element(
        &mut self,
        mut token_range: Range<usize>,
    ) -> Range<usize> {
        let starts_after_element = self
            .draft
            .textarea
            .text_element_ranges()
            .any(|range| range.end == token_range.start);
        if starts_after_element {
            self.draft
                .textarea
                .replace_range(token_range.start..token_range.start, " ");
            token_range.start += 1;
            token_range.end += 1;
        }
        token_range
    }

    fn mention_token_from_insert_text(insert_text: &str) -> Option<(char, String)> {
        let sigil = insert_text.chars().next()?;
        if !matches!(sigil, '$' | '@') {
            return None;
        }
        let name = &insert_text[sigil.len_utf8()..];
        if name.is_empty() {
            return None;
        }
        if name
            .as_bytes()
            .iter()
            .all(|byte| is_mention_name_char(*byte))
        {
            Some((sigil, name.to_string()))
        } else {
            None
        }
    }

    fn current_mention_elements(&self) -> Vec<(u64, char, String)> {
        self.draft
            .textarea
            .text_element_snapshots()
            .into_iter()
            .filter_map(|snapshot| {
                Self::mention_token_from_insert_text(snapshot.text.as_str())
                    .map(|(sigil, mention)| (snapshot.id, sigil, mention))
            })
            .collect()
    }

    fn snapshot_mention_bindings(&self) -> Vec<MentionBinding> {
        let mut ordered = Vec::new();
        for (id, sigil, mention) in self.current_mention_elements() {
            if let Some(binding) = self.draft.mention_bindings.get(&id)
                && binding.sigil == sigil
                && binding.mention == mention
            {
                ordered.push(MentionBinding {
                    sigil: binding.sigil,
                    mention: binding.mention.clone(),
                    path: binding.path.clone(),
                });
            }
        }
        ordered
    }

    fn bind_mentions_from_snapshot(&mut self, mention_bindings: Vec<MentionBinding>) {
        self.draft.mention_bindings.clear();
        if mention_bindings.is_empty() {
            return;
        }

        let text = self.draft.textarea.text().to_string();
        let mut scan_from = 0usize;
        for binding in mention_bindings {
            let token = format!("{}{}", binding.sigil, binding.mention);
            let Some(range) =
                find_next_mention_token_range(text.as_str(), token.as_str(), scan_from)
            else {
                continue;
            };

            let id = if let Some(id) = self.draft.textarea.add_element_range(range.clone()) {
                Some(id)
            } else {
                self.draft
                    .textarea
                    .element_id_for_exact_range(range.clone())
            };

            if let Some(id) = id {
                self.draft.mention_bindings.insert(
                    id,
                    ComposerMentionBinding {
                        sigil: binding.sigil,
                        mention: binding.mention,
                        path: binding.path,
                    },
                );
                scan_from = range.end;
            }
        }
    }

    fn plugin_at_mention_highlights(&self) -> Vec<(Range<usize>, Style)> {
        self.draft
            .textarea
            .text_element_snapshots()
            .into_iter()
            .filter_map(|snapshot| {
                let binding = self.draft.mention_bindings.get(&snapshot.id)?;
                if !binding.path.starts_with("plugin://") || !snapshot.text.starts_with('@') {
                    return None;
                }
                Some((snapshot.range, Style::default().fg(Color::Magenta)))
            })
            .collect()
    }

    /// 为提交/排队准备文本。如果提交应被抑制则返回 None。
    /// 成功时清除待处理的粘贴负载，因为占位符已被展开。
    ///
    /// 当 `record_history` 为 true 时，最终提交被存储用于 ↑/↓ 召回。
    fn prepare_submission_text(
        &mut self,
        record_history: bool,
    ) -> Option<(String, Vec<TextElement>)> {
        self.prepare_submission_text_with_options(
            record_history,
            SlashValidation::Immediate,
            PendingPasteHandling::Expand,
        )
    }

    fn prepare_submission_text_with_options(
        &mut self,
        record_history: bool,
        slash_validation: SlashValidation,
        pending_paste_handling: PendingPasteHandling,
    ) -> Option<(String, Vec<TextElement>)> {
        let mut text = self.current_text();
        let original_input = text.clone();
        let original_text_elements = self.current_text_elements();
        let original_mention_bindings = self.snapshot_mention_bindings();
        let original_local_image_paths = self.attachments.local_image_paths();
        let original_pending_pastes = self.draft.pending_pastes.clone();
        let mut text_elements = original_text_elements.clone();
        let input_starts_with_space = original_input.starts_with(' ');
        self.draft.recent_submission_mention_bindings.clear();
        self.draft.textarea.set_text_clearing_elements("");
        self.draft.is_bash_mode = false;

        if pending_paste_handling == PendingPasteHandling::Expand
            && !self.draft.pending_pastes.is_empty()
        {
            // 展开占位符，使元素字节范围保持对齐。
            let (expanded, expanded_elements) =
                Self::expand_pending_pastes(&text, text_elements, &self.draft.pending_pastes);
            text = expanded;
            text_elements = expanded_elements;
        }

        let expanded_input = text.clone();

        // 如果既没有文本也没有附件，则完全抑制提交。
        text = text.trim().to_string();
        text_elements = Self::trim_text_elements(&expanded_input, &text, text_elements);

        if slash_validation == SlashValidation::Immediate
            && let SubmissionValidation::UnknownCommand(name) = self
                .slash_input()
                .validate_submission(&text, input_starts_with_space)
        {
            let message = format!(
                r#"Unrecognized command '/{name}'. Type "/" for a list of supported commands."#
            );
            self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                history_cell::new_info_event(message, /*hint*/ None),
            )));
            self.set_text_content_with_mention_bindings(
                original_input.clone(),
                original_text_elements,
                original_local_image_paths,
                original_mention_bindings,
            );
            self.draft
                .pending_pastes
                .clone_from(&original_pending_pastes);
            self.draft.textarea.set_cursor(original_input.len());
            return None;
        }

        let actual_chars = text.chars().count();
        if actual_chars > MAX_USER_INPUT_TEXT_CHARS {
            let message = user_input_too_large_message(actual_chars);
            self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
                history_cell::new_error_event(message),
            )));
            self.set_text_content_with_mention_bindings(
                original_input.clone(),
                original_text_elements,
                original_local_image_paths,
                original_mention_bindings,
            );
            self.draft
                .pending_pastes
                .clone_from(&original_pending_pastes);
            self.draft.textarea.set_cursor(original_input.len());
            return None;
        }
        self.attachments
            .prune_local_images_for_submission(&text, &text_elements);
        if text.is_empty() && self.attachments.is_empty() {
            return None;
        }
        self.draft.recent_submission_mention_bindings = original_mention_bindings.clone();
        if record_history && (!text.is_empty() || !self.attachments.is_empty()) {
            self.history.record_local_submission(HistoryEntry {
                text: text.clone(),
                text_elements: text_elements.clone(),
                local_image_paths: self.attachments.local_image_paths(),
                remote_image_urls: self.attachments.remote_image_urls(),
                mention_bindings: original_mention_bindings,
                pending_pastes: if pending_paste_handling == PendingPasteHandling::Preserve {
                    original_pending_pastes.clone()
                } else {
                    Vec::new()
                },
            });
        }
        self.draft.pending_pastes.clear();
        Some((text, text_elements))
    }

    /// 处理消息提交/排队的通用逻辑。
    /// 根据 `should_queue` 返回适当的 InputResult。
    fn handle_submission(&mut self, should_queue: bool) -> (InputResult, bool) {
        let result = self.handle_submission_with_time(should_queue, Instant::now());
        self.reset_vim_mode_after_successful_dispatch(&result.0);
        result
    }

    fn reset_vim_mode_after_successful_dispatch(&mut self, result: &InputResult) {
        if matches!(
            result,
            InputResult::Submitted { .. }
                | InputResult::Queued { .. }
                | InputResult::Command(_)
                | InputResult::ServiceTierCommand(_)
                | InputResult::CommandWithArgs(_, _, _)
        ) {
            self.draft.textarea.enter_vim_normal_mode();
        }
    }

    fn handle_submission_with_time(
        &mut self,
        should_queue: bool,
        now: Instant,
    ) -> (InputResult, bool) {
        // 在应用父拥有提交策略之前保留属于粘贴的换行符。
        // 排队的启动输入仍然将突发刷新到其排队的消息中，如下。
        let in_slash_context = self.slash_commands_enabled()
            && !self.draft.is_bash_mode
            && (matches!(self.popups.active, ActivePopup::Command(_))
                || self
                    .draft
                    .textarea
                    .text()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .starts_with('/'));
        if !should_queue
            && !self.draft.disable_paste_burst
            && self.draft.paste_burst.is_active()
            && !in_slash_context
            && self.draft.paste_burst.append_newline_if_active(now)
        {
            return (InputResult::None, true);
        }
        if !should_queue
            && !in_slash_context
            && !self.draft.disable_paste_burst
            && self
                .draft
                .paste_burst
                .newline_should_insert_instead_of_submit(now)
        {
            self.draft.textarea.insert_str("\n");
            self.draft.paste_burst.extend_window(now);
            return (InputResult::None, true);
        }

        if let Some(result) = self.handle_parent_owned_submission() {
            return result;
        }
        if should_queue {
            if let Some(pasted) = self.draft.paste_burst.flush_before_modified_input() {
                self.handle_paste(pasted);
            }
            let raw_text = self.draft.textarea.text();
            let defer_slash_validation = self.slash_input().should_parse_on_dequeue(raw_text);
            let preserve_pending_pastes = defer_slash_validation
                && !self.draft.pending_pastes.is_empty()
                && parse_slash_name(raw_text)
                    .is_some_and(|(name, _, _)| name == SlashCommand::Goal.command());
            let pending_pastes = if preserve_pending_pastes {
                self.draft.pending_pastes.clone()
            } else {
                Vec::new()
            };
            if let Some((text, text_elements)) = self.prepare_submission_text_with_options(
                /*record_history*/ true,
                if defer_slash_validation {
                    SlashValidation::Deferred
                } else {
                    SlashValidation::Immediate
                },
                if preserve_pending_pastes {
                    PendingPasteHandling::Preserve
                } else {
                    PendingPasteHandling::Expand
                },
            ) {
                let action = slash_input::queued_input_action(&text, defer_slash_validation);
                return (
                    InputResult::Queued {
                        text,
                        text_elements,
                        action,
                        pending_pastes,
                    },
                    true,
                );
            }
            return (InputResult::None, true);
        }

        // 如果第一行是裸内置斜杠命令（无参数），
        // 即使斜杠弹出窗口不可见也分派它。这保留了
        // 工作流：输入前缀（"/di"），按 Tab 补全为
        // "/diff "，然后按 Enter/Ctrl+Shift+Q 运行它。Tab 将光标移到
        // '/name' 令牌之后，我们基于光标的启发式隐藏弹出窗口，
        // 但 Enter/Ctrl+Shift+Q 仍应分派命令而不是提交
        // 字面文本。
        if let Some(result) = self.try_dispatch_bare_slash_command() {
            return (result, true);
        }

        let original_input = self.current_text();
        let original_text_elements = self.current_text_elements();
        let original_mention_bindings = self.snapshot_mention_bindings();
        let original_local_image_paths = self.attachments.local_image_paths();
        let original_pending_pastes = self.draft.pending_pastes.clone();
        if let Some(result) = self.try_dispatch_slash_command_with_args() {
            // 直接键入并提交的 inline-arg 命令(如 `/plan <task>` `/review <args>`):
            // 命令已被消费,清空 textarea 并退出 bash 模式,与 bare 命令路径对齐。
            // (注意:弹窗补全路径 `handle_key_event_with_slash_popup` 调用同一方法时
            //  需保留 draft tail,故清空放在本「提交」调用点,而非方法内部。)
            self.draft.textarea.set_text_clearing_elements("");
            self.draft.is_bash_mode = false;
            return (result, true);
        }

        if let Some((text, text_elements)) =
            self.prepare_submission_text(/*record_history*/ true)
        {
            if should_queue {
                (
                    InputResult::Queued {
                        text,
                        text_elements,
                        action: QueuedInputAction::Plain,
                        pending_pastes: Vec::new(),
                    },
                    true,
                )
            } else {
                // 此处不在此清除本地附件；ChatWidget 通过
                // take_recent_submission_images() 排空它们。
                (
                    InputResult::Submitted {
                        text,
                        text_elements,
                    },
                    true,
                )
            }
        } else {
            // 如果提交被抑制则恢复文本。
            self.set_text_content_with_mention_bindings(
                original_input,
                original_text_elements,
                original_local_image_paths,
                original_mention_bindings,
            );
            self.draft.pending_pastes = original_pending_pastes;
            (InputResult::None, true)
        }
    }

    fn handle_parent_owned_submission(&mut self) -> Option<(InputResult, bool)> {
        if !self.blocks_direct_input {
            return None;
        }

        let text = self.current_text();
        let allowed_slash_command = parse_slash_name(&text).is_some_and(|(name, args, _)| {
            matches!(
                self.slash_input().command(name),
                Some(SlashCommandItem::Builtin(command))
                    if name == command.command()
                        && parent_owned_command_is_allowed(command, args)
            )
        });
        if text.starts_with('/') && allowed_slash_command {
            return None;
        }

        Some((InputResult::ParentOwnedInputBlocked, true))
    }

    /// 检查第一行是否为裸斜杠命令（无参数）并分派它。
    /// 如果分派了命令则返回 Some(InputResult)，否则返回 None。
    fn try_dispatch_bare_slash_command(&mut self) -> Option<InputResult> {
        let command = self
            .slash_input()
            .bare_command(self.draft.textarea.text())?;
        if self.reject_slash_command_if_unavailable(&command) {
            self.stage_slash_command_history(&command);
            self.record_pending_slash_command_history();
            return Some(InputResult::None);
        }
        self.stage_slash_command_history(&command);
        self.draft.textarea.set_text_clearing_elements("");
        self.draft.is_bash_mode = false;
        Some(match command {
            SlashCommandItem::Builtin(cmd) => InputResult::Command(cmd),
            SlashCommandItem::ServiceTier(command) => InputResult::ServiceTierCommand(command),
        })
    }

    /// 检查输入是否为带参数的斜杠命令（例如 /review args）并分派它。
    /// 如果分派了命令则返回 Some(InputResult)，否则返回 None。
    fn try_dispatch_slash_command_with_args(&mut self) -> Option<InputResult> {
        let text = self.draft.textarea.text().to_string();
        let inline_command = self.slash_input().inline_command(&text)?;
        let command = inline_command.command;
        if self.reject_slash_command_if_unavailable(&command) {
            self.stage_slash_command_history(&command);
            self.record_pending_slash_command_history();
            return Some(InputResult::None);
        }

        self.stage_slash_command_history(&command);

        let mut args_elements = slash_input::args_elements(
            inline_command.rest,
            inline_command.rest_offset,
            &self.draft.textarea.text_elements(),
        );
        let trimmed_rest = inline_command.rest.trim();
        args_elements = Self::trim_text_elements(inline_command.rest, trimmed_rest, args_elements);
        let SlashCommandItem::Builtin(cmd) = command else {
            return None;
        };
        Some(InputResult::CommandWithArgs(
            cmd,
            trimmed_rest.to_string(),
            args_elements,
        ))
    }

    /// 展开待处理占位符并提取规范化的内联命令参数。
    ///
    /// 内联参数命令最初使用原始草稿分派，因此命令拒绝不会
    /// 消耗用户输入。一旦命令需要其参数，此辅助方法执行通常的
    /// 提交准备（粘贴展开、元素修剪），并将元素范围从
    /// 全文偏移量重建基线到命令参数偏移量。
    ///
    /// 已经暂存斜杠命令历史的调用方通常应为
    /// `record_history` 传 `false`；否则像 `/plan investigate` 这样的命令会通过
    /// 斜杠命令路径和消息提交路径同时进入
    /// 本地召回。
    pub(crate) fn prepare_inline_args_submission(
        &mut self,
        record_history: bool,
    ) -> Option<(String, Vec<TextElement>)> {
        let (prepared_text, prepared_elements) = self.prepare_submission_text(record_history)?;
        let (prepared_rest, prepared_rest_offset) = slash_input::prepared_args(&prepared_text)?;
        let mut args_elements =
            slash_input::args_elements(prepared_rest, prepared_rest_offset, &prepared_elements);
        let trimmed_rest = prepared_rest.trim();
        args_elements = Self::trim_text_elements(prepared_rest, trimmed_rest, args_elements);
        Some((trimmed_rest.to_string(), args_elements))
    }

    fn reject_slash_command_if_unavailable(&self, command: &SlashCommandItem) -> bool {
        if !self.is_task_running || command.available_during_task() {
            return false;
        }
        let message = format!(
            "'/{}' is disabled while a task is in progress.",
            command.command()
        );
        self.app_event_tx.send(AppEvent::InsertHistoryCell(Box::new(
            history_cell::new_error_event(message),
        )));
        true
    }

    /// 为后续本地召回暂存当前斜杠命令文本。
    ///
    /// 暂存会在文本区域被清除前快照丰富的编辑器状态。`ChatWidget`
    /// 在分派后提交暂存的条目，以便命令召回跟随提交的文本，而非
    /// 命令结果。
    fn stage_slash_command_history(&mut self, command: &SlashCommandItem) {
        if matches!(command, SlashCommandItem::Builtin(SlashCommand::Clear)) {
            return;
        }
        self.stage_slash_command_history_text(self.draft.textarea.text().trim().to_string());
    }

    /// 使用其规范命令文本暂存弹出窗口选中的命令。
    ///
    /// 弹出窗口过滤文本可能不完整，因此记录选中的命令可避免在用户实际
    /// 接受 `/diff` 后召回 `/di`。
    fn stage_selected_slash_command_history(&mut self, command: &CommandItem) {
        if matches!(command, CommandItem::Builtin(SlashCommand::Clear)) {
            return;
        }
        self.stage_slash_command_history_text(format!("/{}", command.command()));
    }

    /// 在待处理槽位中存储提供的命令文本和当前编辑器装饰。
    ///
    /// 待处理条目刻意与其它本地历史条目形状相同，以便召回
    /// 可以恢复附件、提及绑定和待处理粘贴占位符（如果命令
    /// 工作流未来开始携带这些）。
    fn stage_slash_command_history_text(&mut self, text: String) {
        self.pending_slash_command_history = Some(HistoryEntry {
            text,
            text_elements: self.draft.textarea.text_elements(),
            local_image_paths: self.attachments.local_image_paths(),
            remote_image_urls: self.attachments.remote_image_urls(),
            mention_bindings: self.snapshot_mention_bindings(),
            pending_pastes: self.draft.pending_pastes.clone(),
        });
    }

    fn handle_remote_image_selection_key(
        &mut self,
        key_event: &KeyEvent,
    ) -> Option<(InputResult, bool)> {
        self.attachments
            .handle_remote_image_selection_key(key_event, &mut self.draft.textarea)
    }

    /// 无弹出窗口可见时处理按键事件。
    fn handle_key_event_without_popup(&mut self, key_event: KeyEvent) -> (InputResult, bool) {
        if let Some((result, redraw)) = self.handle_remote_image_selection_key(&key_event) {
            return (result, redraw);
        }
        if self.attachments.selected_remote_image_index.is_some() {
            self.attachments.clear_remote_image_selection();
        }
        if self.handle_shortcut_overlay_key(&key_event) {
            return (InputResult::None, true);
        }
        if self.draft.is_bash_mode && key_event.code == KeyCode::Esc {
            if let Some(pasted) = self.draft.paste_burst.flush_before_modified_input() {
                self.handle_paste(pasted);
            }
            if self.draft.textarea.is_empty() {
                self.draft.is_bash_mode = false;
                return (InputResult::None, true);
            }
        }
        if self.should_handle_vim_insert_escape(key_event) {
            return self.handle_input_basic(key_event);
        }
        if self.draft.textarea.is_vim_normal_mode() && self.draft.textarea.is_vim_operator_pending()
        {
            return self.handle_input_basic(key_event);
        }
        if self.draft.textarea.is_vim_normal_mode()
            && self.is_empty()
            && matches!(
                key_event,
                KeyEvent {
                    code: KeyCode::Char('/'),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }
            )
        {
            self.footer.mode = reset_mode_after_activity(self.footer.mode);
            self.draft.textarea.set_text_clearing_elements("/");
            self.draft
                .textarea
                .set_cursor(self.draft.textarea.text().len());
            self.draft.textarea.enter_vim_insert_mode();
            return (InputResult::None, true);
        }
        if self.draft.textarea.is_vim_normal_mode()
            && self.is_empty()
            && matches!(
                key_event,
                KeyEvent {
                    code: KeyCode::Char('!'),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }
            )
        {
            self.footer.mode = reset_mode_after_activity(self.footer.mode);
            self.draft.is_bash_mode = true;
            self.draft.textarea.enter_vim_insert_mode();
            return (InputResult::None, true);
        }
        if key_event.code == KeyCode::Esc {
            if self.is_empty() {
                let next_mode = esc_hint_mode(self.footer.mode, self.is_task_running);
                if next_mode != self.footer.mode {
                    self.footer.mode = next_mode;
                    return (InputResult::None, true);
                }
            }
        } else {
            self.footer.mode = reset_mode_after_activity(self.footer.mode);
        }
        if self.queue_keys.is_pressed(key_event)
            && (self.is_task_running || self.queue_submissions || !self.is_bang_shell_command())
        {
            return self.handle_submission(self.is_task_running || self.queue_submissions);
        }

        if self.submit_keys.is_pressed(key_event) {
            return self.handle_submission(self.queue_submissions);
        }

        if let KeyEvent {
            code: KeyCode::Char('d'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            ..
        } = key_event
            && self.is_empty()
        {
            return (InputResult::None, false);
        }

        let (history_up_pressed, history_down_pressed) = if self.draft.textarea.is_vim_normal_mode()
        {
            if self.draft.textarea.is_vim_operator_pending() {
                (false, false)
            } else {
                (
                    self.vim_normal_keymap.move_up.is_pressed(key_event),
                    self.vim_normal_keymap.move_down.is_pressed(key_event),
                )
            }
        } else {
            (
                self.editor_keymap.move_up.is_pressed(key_event),
                self.editor_keymap.move_down.is_pressed(key_event),
            )
        };
        if history_up_pressed || history_down_pressed {
            if self
                .history
                .should_handle_navigation(&self.current_text(), self.history_navigation_cursor())
            {
                let replace_entry = if history_up_pressed {
                    self.history.navigate_up(&self.app_event_tx)
                } else {
                    self.history.navigate_down(&self.app_event_tx)
                };
                if let Some(entry) = replace_entry {
                    self.apply_history_entry(entry);
                    return (InputResult::None, true);
                }
            }
            return self.handle_input_basic(key_event);
        }

        self.handle_input_basic(key_event)
    }

    fn is_bang_shell_command(&self) -> bool {
        self.current_text().trim_start().starts_with('!')
    }

    fn shell_mode_footer_line(&self) -> Option<Line<'static>> {
        self.is_bang_shell_command()
            .then_some(())
            .map(|_| Line::from(vec![Span::from("Shell mode").light_red()]))
    }

    /// 在时刻 `now` 应用任何到期的 `PasteBurst` 刷新。
    ///
    /// 将 [`PasteBurst::flush_if_due`] 的结果转换为具体的文本区域修改。
    ///
    /// 调用方：
    ///
    /// - 通过 [`ChatComposer::flush_paste_burst_if_due`] 的 UI 滴答，以便被持有的首字符可以渲染。
    /// - 通过 [`ChatComposer::handle_input_basic`] 的输入处理，以便到期的突发不会滞后。
    fn handle_paste_burst_flush(&mut self, now: Instant) -> bool {
        match self.draft.paste_burst.flush_if_due(now) {
            FlushResult::Paste(pasted) => {
                self.handle_paste(pasted);
                true
            }
            FlushResult::Typed(ch) => {
                self.insert_str(ch.to_string().as_str());
                true
            }
            FlushResult::None => false,
        }
    }

    /// 处理修改文本区域的按键，包括粘贴突发检测。
    ///
    /// 作为修改文本区域的按键的最低层键路径。对于无法可靠提供
    /// 括号粘贴的终端，这里也是将纯字符流
    /// 转换为显式粘贴操作的地方。
    ///
    /// 顺序很重要：
    ///
    /// - 总是先刷新任何*到期*的粘贴突发，使缓冲文本不会滞后于无关
    ///   编辑。
    /// - 然后处理传入的按键，仅拦截"纯"（无 Ctrl/Alt）字符输入。
    /// - 对于非纯按键，在应用按键前通过 `flush_before_modified_input()` 刷新；
    ///   否则 `clear_window_after_non_char()` 可能让缓冲文本在没有
    ///   可超时的时间戳的情况下一直等待。
    fn handle_input_basic(&mut self, input: KeyEvent) -> (InputResult, bool) {
        // 在此忽略按键释放，避免将其视为额外输入
        // （例如通过粘贴突发逻辑将相同的字符附加两次）。
        if !matches!(input.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return (InputResult::None, false);
        }

        self.handle_input_basic_with_time(input, Instant::now())
    }

    fn handle_input_basic_with_time(
        &mut self,
        input: KeyEvent,
        now: Instant,
    ) -> (InputResult, bool) {
        // 如果我们有缓冲的无括号粘贴突发，并且距最后一个字符已过去足够
        // 时间，在处理新输入前刷新它。
        self.handle_paste_burst_flush(now);

        if !matches!(input.code, KeyCode::Esc) {
            self.footer.mode = reset_mode_after_activity(self.footer.mode);
        }

        // 如果我们正在捕获突发并收到 Enter，将其累积而不是插入。
        if matches!(input.code, KeyCode::Enter)
            && !self.draft.disable_paste_burst
            && self.draft.paste_burst.is_active()
            && self.draft.paste_burst.append_newline_if_active(now)
        {
            return (InputResult::None, true);
        }

        // 拦截纯 Char 输入以可选地累积到突发缓冲区中。
        //
        // 这刻意仅限于"纯"（无 Ctrl/Alt）字符，以便快捷键保持其
        // 正常语义，并且当按下非字符
        // 键时我们可以积极地刷新/清除任何突发状态。
        if let KeyEvent {
            code: KeyCode::Char(ch),
            modifiers,
            ..
        } = input
        {
            let has_ctrl_or_alt = has_ctrl_or_alt(modifiers);
            if !has_ctrl_or_alt
                && !self.draft.disable_paste_burst
                && self.draft.textarea.allows_paste_burst()
            {
                // 非 ASCII 字符（例如来自 IME）可以快速突发到达，因此避免
                // 持有首字符，同时仍然允许粘贴输入的突发检测。
                if !ch.is_ascii() {
                    return self.handle_non_ascii_char(input, now);
                }

                match self.draft.paste_burst.on_plain_char(ch, now) {
                    CharDecision::BufferAppend => {
                        self.draft.paste_burst.append_char_to_buffer(ch, now);
                        return (InputResult::None, true);
                    }
                    CharDecision::BeginBuffer { retro_chars } => {
                        let cur = self.draft.textarea.cursor();
                        let txt = self.draft.textarea.text();
                        let safe_cur = Self::clamp_to_char_boundary(txt, cur);
                        let before = &txt[..safe_cur];
                        if let Some(grab) = self.draft.paste_burst.decide_begin_buffer(
                            now,
                            before,
                            retro_chars as usize,
                        ) {
                            if !grab.grabbed.is_empty() {
                                self.draft
                                    .textarea
                                    .replace_range(grab.start_byte..safe_cur, "");
                            }
                            self.draft.paste_burst.append_char_to_buffer(ch, now);
                            return (InputResult::None, true);
                        }
                        // 如果 decide_begin_buffer 选择不开始缓冲，
                        // 则落入下面的正常插入。
                    }
                    CharDecision::BeginBufferFromPending => {
                        // 首字符被持有；现在追加当前字符。
                        self.draft.paste_burst.append_char_to_buffer(ch, now);
                        return (InputResult::None, true);
                    }
                    CharDecision::RetainFirstChar => {
                        // 暂时保持第一个快速字符待处理。
                        return (InputResult::None, true);
                    }
                }
            }
            if let Some(pasted) = self.draft.paste_burst.flush_before_modified_input() {
                self.handle_paste(pasted);
            }
        }

        // 在应用非字符输入（方向键等）前刷新任何缓冲的突发。
        //
        // `clear_window_after_non_char()` 清除 `last_plain_char_time`。如果缓冲区非空时清除它，
        // `flush_if_due()` 就会失去可
        // 超时的时间戳，缓冲的粘贴可能一直卡住，直到另一个纯字符
        // 到来。
        if !matches!(input.code, KeyCode::Char(_) | KeyCode::Enter)
            && let Some(pasted) = self.draft.paste_burst.flush_before_modified_input()
        {
            self.handle_paste(pasted);
        }
        // 对于非字符输入（或刷新后），正常处理。
        // 跟踪元素删除，以便我们可以丢弃任何相应的占位符而无需扫描
        // 全文。（占位符是原子元素；删除时元素消失。）
        let elements_before = if self.draft.pending_pastes.is_empty() && self.attachments.is_empty()
        {
            None
        } else {
            Some(self.draft.textarea.element_payloads())
        };

        if self.draft.is_bash_mode
            && matches!(input.code, KeyCode::Backspace)
            && self.draft.textarea.cursor() == 0
        {
            self.draft.is_bash_mode = false;
            return (InputResult::None, true);
        }

        self.draft.textarea.input(input);
        self.sync_bash_mode_from_text();

        if let Some(elements_before) = elements_before {
            self.reconcile_deleted_elements(elements_before);
        }

        // 为纯 Char（无 Ctrl/Alt）事件更新粘贴突发启发式。
        let crossterm::event::KeyEvent {
            code, modifiers, ..
        } = input;
        match code {
            KeyCode::Char(_) => {
                let has_ctrl_or_alt = has_ctrl_or_alt(modifiers);
                if has_ctrl_or_alt {
                    self.draft.paste_burst.clear_window_after_non_char();
                }
            }
            KeyCode::Enter => {
                // 保持突发窗口存活（支持粘贴中的空行）。
            }
            _ => {
                // 其他键：清除突发窗口（缓冲区应已在上方按需刷新）。
                self.draft.paste_burst.clear_window_after_non_char();
            }
        }

        (InputResult::None, true)
    }

    fn sync_bash_mode_from_text(&mut self) {
        if !self.draft.is_bash_mode && self.draft.textarea.text().starts_with('!') {
            self.draft.textarea.replace_range(0..1, "");
            self.draft.is_bash_mode = true;
        }
    }

    fn reconcile_deleted_elements(&mut self, elements_before: Vec<String>) {
        let elements_after: HashSet<String> =
            self.draft.textarea.element_payloads().into_iter().collect();

        let removed_payloads = elements_before
            .into_iter()
            .filter(|payload| !elements_after.contains(payload))
            .collect::<Vec<_>>();
        for removed in &removed_payloads {
            self.draft.pending_pastes.retain(|(ph, _)| ph != removed);
        }
        self.attachments
            .remove_deleted_local_placeholders(&removed_payloads, &mut self.draft.textarea);
    }

    /// 处理专用的快捷方式覆盖切换键。
    ///
    /// 此方法仅在编辑器为空且没有粘贴突发进行中时切换，
    /// 以便输入/粘贴 `?` 仍插入文本而非打开
    /// 帮助。绑定的键列表有意支持终端变体
    /// 修饰符报告（例如 `?` 与 `shift-?`）。
    fn handle_shortcut_overlay_key(&mut self, key_event: &KeyEvent) -> bool {
        if key_event.kind != KeyEventKind::Press {
            return false;
        }

        let toggles = self.toggle_shortcuts_keys.is_pressed(*key_event)
            && self.is_empty()
            && !self.is_in_paste_burst();

        if !toggles {
            return false;
        }

        let next = toggle_shortcut_mode(
            self.footer.mode,
            self.quit_shortcut_hint_visible(),
            self.is_empty(),
        );
        let changed = next != self.footer.mode;
        self.footer.mode = next;
        changed
    }

    fn footer_props(&self) -> FooterProps {
        let mode = self.footer_mode();
        let is_wsl = {
            #[cfg(target_os = "linux")]
            {
                mode == FooterMode::ShortcutOverlay
                    && crate::tui_core::clipboard_paste::is_probably_wsl()
            }
            #[cfg(not(target_os = "linux"))]
            {
                false
            }
        };

        FooterProps {
            mode,
            esc_backtrack_hint: self.footer.esc_backtrack_hint,
            use_shift_enter_hint: self.footer.use_shift_enter_hint,
            is_task_running: self.is_task_running,
            queue_submissions: self.queue_submissions,
            quit_shortcut_key: self.footer.quit_shortcut_key,
            collaboration_modes_enabled: self.collaboration_modes_enabled,
            is_wsl,
            status_line_value: self.footer.status_line_value.clone(),
            status_line_enabled: self.footer.status_line_enabled,
            key_hints: FooterKeyHints {
                toggle_shortcuts: self.footer.toggle_shortcuts_key,
                queue: self.footer.queue_key,
                insert_newline: self.footer.insert_newline_key,
                external_editor: self.footer.external_editor_key,
                edit_previous: Some(key_hint::plain(KeyCode::Esc)),
                show_transcript: self.footer.show_transcript_key,
                history_search: self.footer.history_search_key,
                reasoning_down: self.footer.reasoning_down_key,
                reasoning_up: self.footer.reasoning_up_key,
            },
            active_agent_label: self.footer.active_agent_label.clone(),
        }
    }

    /// 通过小型优先级瀑布流解析有效底部模式。
    ///
    /// 基础模式仅由编辑器是否为空派生：
    /// 为空时为 `ComposerEmpty`，否则为 `ComposerHasDraft`。瞬态
    /// 模式（Esc 提示、覆盖、退出提醒）可以在其
    /// 条件激活时覆盖该基础模式。
    fn footer_mode(&self) -> FooterMode {
        if self.history_search.is_some() {
            return FooterMode::HistorySearch;
        }

        let base_mode = if self.is_empty() {
            FooterMode::ComposerEmpty
        } else {
            FooterMode::ComposerHasDraft
        };

        match self.footer.mode {
            FooterMode::HistorySearch => FooterMode::HistorySearch,
            FooterMode::EscHint => FooterMode::EscHint,
            FooterMode::ShortcutOverlay => FooterMode::ShortcutOverlay,
            FooterMode::QuitShortcutReminder if self.quit_shortcut_hint_visible() => {
                FooterMode::QuitShortcutReminder
            }
            FooterMode::ComposerEmpty | FooterMode::ComposerHasDraft
                if self.quit_shortcut_hint_visible() =>
            {
                FooterMode::QuitShortcutReminder
            }
            FooterMode::QuitShortcutReminder => base_mode,
            FooterMode::ComposerEmpty | FooterMode::ComposerHasDraft => base_mode,
        }
    }

    fn custom_footer_height(&self) -> Option<u16> {
        if self.footer.flash_visible() {
            return Some(1);
        }
        self.footer
            .hint_override
            .as_ref()
            .map(|items| if items.is_empty() { 0 } else { 1 })
    }

    pub(crate) fn sync_popups(&mut self) {
        self.sync_slash_command_elements();
        if self.history_search.is_some() {
            if self.popups.current_file_query.is_some() {
                self.app_event_tx
                    .send(AppEvent::StartFileSearch(String::new()));
                self.popups.current_file_query = None;
            }
            self.popups.active = ActivePopup::None;
            self.popups.dismissed_file_token = None;
            self.popups.dismissed_mention_token = None;
            return;
        }
        if !self.popups_enabled() {
            self.popups.active = ActivePopup::None;
            return;
        }
        let mut mentions_v2_token = self.current_mentions_v2_token_range();
        let mut file_token = if self.mentions_v2_enabled {
            None
        } else {
            self.current_editable_at_token_range_with_options(/*allow_empty*/ false)
        };
        let browsing_history = self
            .history
            .should_handle_navigation(&self.current_text(), self.history_navigation_cursor());
        // 浏览输入历史（shell 风格的 Up/Down 召回）时，跳过所有弹出
        // 同步，以免任何内容抢走持续历史导航的焦点。
        if browsing_history {
            if self.popups.current_file_query.is_some() {
                self.app_event_tx
                    .send(AppEvent::StartFileSearch(String::new()));
                self.popups.current_file_query = None;
            }
            self.popups.active = ActivePopup::None;
            return;
        }
        let mut mention_target = self.current_mention_target();
        let at_token_start = mentions_v2_token
            .as_ref()
            .or(file_token.as_ref())
            .map(|(range, _)| range.start);
        let mention_token_start = mention_target.as_ref().map(|target| target.range.start);
        if let (Some(at_token_start), Some(mention_token_start)) =
            (at_token_start, mention_token_start)
        {
            if at_token_start > mention_token_start {
                mention_target = None;
            } else if mention_token_start > at_token_start {
                mentions_v2_token = None;
                file_token = None;
            }
        }

        let allow_command_popup = self.slash_commands_enabled()
            && !self.draft.is_bash_mode
            && file_token.is_none()
            && mentions_v2_token.is_none()
            && mention_target.is_none();
        self.sync_command_popup(allow_command_popup);

        if matches!(self.popups.active, ActivePopup::Command(_)) {
            if self.popups.current_file_query.is_some() {
                self.app_event_tx
                    .send(AppEvent::StartFileSearch(String::new()));
                self.popups.current_file_query = None;
            }
            self.popups.dismissed_file_token = None;
            self.popups.dismissed_mention_token = None;
            return;
        }

        if let Some((range, token)) = mentions_v2_token {
            self.sync_mentions_v2_popup(range, token);
            return;
        }

        if let Some(target) = mention_target {
            if self.popups.current_file_query.is_some() {
                self.app_event_tx
                    .send(AppEvent::StartFileSearch(String::new()));
                self.popups.current_file_query = None;
            }
            self.sync_mention_popup(target);
            return;
        }
        self.popups.dismissed_mention_token = None;

        if let Some((range, token)) = file_token {
            self.sync_file_search_popup(range, token);
            return;
        }

        if self.popups.current_file_query.is_some() {
            self.app_event_tx
                .send(AppEvent::StartFileSearch(String::new()));
            self.popups.current_file_query = None;
        }
        self.popups.dismissed_file_token = None;
        if matches!(
            self.popups.active,
            ActivePopup::File(_) | ActivePopup::Skill(_) | ActivePopup::MentionV2(_)
        ) {
            self.popups.active = ActivePopup::None;
        }
    }

    /// 使用文本区域中的当前文本同步 `self.command_popup`。
    /// 每次可能改变文本的修改后都必须调用此方法，以便
    /// 弹出窗口相应地显示/更新/隐藏。
    fn sync_command_popup(&mut self, allow: bool) {
        let text = self.draft.textarea.text();
        let first_line_end = text.find('\n').unwrap_or(text.len());
        let first_line = &text[..first_line_end];
        // 保持显式关闭的弹出窗口关闭，直到命令令牌改变。
        let command_token = slash_input::command_popup_filter_text(first_line, /*cursor*/ 0);
        if let Some(command_token) = command_token.as_deref()
            && self.popups.dismissed_command_token.as_deref() == Some(command_token)
        {
            return;
        }
        self.popups.dismissed_command_token = None;

        if !allow {
            if matches!(self.popups.active, ActivePopup::Command(_)) {
                self.popups.active = ActivePopup::None;
            }
            return;
        }
        // 判断插入符是否位于第一行的初始 '/name' 令牌内。
        let cursor = self.draft.textarea.cursor();
        let caret_on_first_line = cursor <= first_line_end;

        let is_editing_slash_command_name = caret_on_first_line
            && self
                .slash_input()
                .is_editing_command_name(first_line, cursor);
        let command_filter_text = caret_on_first_line
            .then(|| slash_input::command_popup_filter_text(first_line, cursor))
            .flatten();

        // 如果光标当前位于 `@token` 内，优先使用
        // 文件搜索弹出窗口而非斜杠弹出窗口，以便用户可以插入文件路径
        // 作为命令的参数（例如 "/review @docs/..."）。
        if Self::current_at_token(&self.draft.textarea).is_some() {
            if matches!(self.popups.active, ActivePopup::Command(_)) {
                self.popups.active = ActivePopup::None;
            }
            return;
        }
        match &mut self.popups.active {
            ActivePopup::Command(popup) => {
                if is_editing_slash_command_name {
                    if let Some(command_filter_text) = command_filter_text.as_deref() {
                        popup.on_composer_text_change(command_filter_text.to_string());
                    }
                } else {
                    self.popups.active = ActivePopup::None;
                }
            }
            _ => {
                if is_editing_slash_command_name
                    && let Some(command_filter_text) = command_filter_text.as_deref()
                {
                    let command_popup = self.slash_input().command_popup(command_filter_text);
                    self.popups.active = ActivePopup::Command(command_popup);
                }
            }
        }
    }

    /// 使用当前的 `@` 令牌同步遗留文件搜索弹出窗口。
    fn sync_file_search_popup(&mut self, range: Range<usize>, query: String) {
        if self
            .popups
            .dismissed_file_token
            .as_ref()
            .is_some_and(|dismissed| dismissed.matches(&self.draft.textarea, &range, &query))
        {
            return;
        }

        if query.is_empty() {
            self.app_event_tx
                .send(AppEvent::StartFileSearch(String::new()));
        } else {
            self.app_event_tx
                .send(AppEvent::StartFileSearch(query.clone()));
        }

        match &mut self.popups.active {
            ActivePopup::File(popup) => {
                if query.is_empty() {
                    popup.set_empty_prompt();
                } else {
                    popup.set_query(&query);
                }
            }
            _ => {
                let mut popup = FileSearchPopup::new();
                if query.is_empty() {
                    popup.set_empty_prompt();
                } else {
                    popup.set_query(&query);
                }
                self.popups.active = ActivePopup::File(popup);
            }
        }

        if query.is_empty() {
            self.popups.current_file_query = None;
        } else {
            self.popups.current_file_query = Some(query);
        }
        self.popups.dismissed_file_token = None;
    }

    fn sync_mention_popup(&mut self, target: MentionCompletionTarget) {
        let MentionCompletionTarget {
            range,
            query,
            prebuilt_mentions,
        } = target;
        if self
            .popups
            .dismissed_mention_token
            .as_ref()
            .is_some_and(|dismissed| dismissed.matches(&self.draft.textarea, &range, &query))
        {
            return;
        }

        let mentions = prebuilt_mentions.unwrap_or_else(|| self.mention_items());
        if mentions.is_empty() {
            self.popups.active = ActivePopup::None;
            return;
        }

        match &mut self.popups.active {
            ActivePopup::Skill(popup) => {
                popup.set_query(&query);
                popup.set_mentions(mentions);
            }
            _ => {
                let mut popup = SkillPopup::new(mentions);
                popup.set_query(&query);
                self.popups.active = ActivePopup::Skill(popup);
            }
        }
    }

    fn sync_mentions_v2_popup(&mut self, range: Range<usize>, query: String) {
        if self
            .popups
            .dismissed_mention_token
            .as_ref()
            .is_some_and(|dismissed| dismissed.matches(&self.draft.textarea, &range, &query))
        {
            return;
        }

        if query.is_empty() {
            self.app_event_tx
                .send(AppEvent::StartFileSearch(String::new()));
            self.popups.current_file_query = None;
        } else {
            self.app_event_tx
                .send(AppEvent::StartFileSearch(query.clone()));
            self.popups.current_file_query = Some(query.clone());
        }

        let candidates = super::mentions_v2::build_search_catalog(
            self.skills.as_deref(),
            self.plugins.as_deref(),
        );

        match &mut self.popups.active {
            ActivePopup::MentionV2(popup) => {
                popup.set_query(&query);
                popup.set_candidates(candidates);
            }
            _ => {
                let mut popup = MentionV2Popup::new(candidates);
                popup.set_query(&query);
                self.popups.active = ActivePopup::MentionV2(popup);
            }
        }

        self.popups.dismissed_mention_token = None;
    }

    fn mention_items(&self) -> Vec<MentionItem> {
        let mut mentions = Vec::new();
        if let Some(skills) = self.skills.as_ref() {
            for skill in skills {
                let display_name = skill_display_name(skill);
                let description = skill_description(skill);
                let skill_name = skill.name.clone();
                let search_terms = if display_name == skill.name {
                    vec![skill_name.clone()]
                } else {
                    vec![skill_name.clone(), display_name.clone()]
                };
                mentions.push(MentionItem {
                    display_name,
                    description,
                    insert_text: format!("${skill_name}"),
                    search_terms,
                    path: Some(skill.path.to_string_lossy().into_owned()),
                    category_tag: Some("[Skill]".to_string()),
                    sort_rank: 1,
                });
            }
        }

        if let Some(plugins) = self.plugins.as_ref() {
            for plugin in plugins {
                let (plugin_name, marketplace_name) = plugin
                    .config_name
                    .split_once('@')
                    .unwrap_or((plugin.config_name.as_str(), ""));
                let mut capability_labels = Vec::new();
                if plugin.has_skills {
                    capability_labels.push("skills".to_string());
                }
                if !plugin.mcp_server_names.is_empty() {
                    let mcp_server_count = plugin.mcp_server_names.len();
                    capability_labels.push(if mcp_server_count == 1 {
                        "1 MCP server".to_string()
                    } else {
                        format!("{mcp_server_count} MCP servers")
                    });
                }
                if !plugin.app_connector_ids.is_empty() {
                    let app_count = plugin.app_connector_ids.len();
                    capability_labels.push(if app_count == 1 {
                        "1 app".to_string()
                    } else {
                        format!("{app_count} apps")
                    });
                }
                let description = plugin.description.clone().or_else(|| {
                    Some(if capability_labels.is_empty() {
                        "Plugin".to_string()
                    } else {
                        format!("Plugin · {}", capability_labels.join(" · "))
                    })
                });
                let mut search_terms = vec![plugin_name.to_string(), plugin.config_name.clone()];
                if plugin.display_name != plugin_name {
                    search_terms.push(plugin.display_name.clone());
                }
                if !marketplace_name.is_empty() {
                    search_terms.push(marketplace_name.to_string());
                }
                mentions.push(MentionItem {
                    display_name: plugin.display_name.clone(),
                    description,
                    insert_text: format!("${plugin_name}"),
                    search_terms,
                    path: Some(format!("plugin://{}", plugin.config_name)),
                    category_tag: Some("[Plugin]".to_string()),
                    sort_rank: 0,
                });
            }
        }

        if self.connectors_enabled
            && let Some(snapshot) = self.connectors_snapshot.as_ref()
        {
            for connector in &snapshot.connectors {
                if !connector.is_accessible || !connector.is_enabled {
                    continue;
                }
                let display_name = crate::connectors::metadata::connector_display_label(connector);
                let description = Some(Self::connector_brief_description(connector));
                let slug = crate::connectors::metadata::connector_mention_slug(connector);
                let search_terms = vec![display_name.clone(), connector.id.clone(), slug.clone()];
                let connector_id = connector.id.as_str();
                mentions.push(MentionItem {
                    display_name: display_name.clone(),
                    description,
                    insert_text: format!("${slug}"),
                    search_terms,
                    path: Some(format!("app://{connector_id}")),
                    category_tag: Some("[App]".to_string()),
                    sort_rank: 1,
                });
            }
        }

        mentions
    }

    fn connector_brief_description(connector: &AppInfo) -> String {
        Self::connector_description(connector).unwrap_or_default()
    }

    fn connector_description(connector: &AppInfo) -> Option<String> {
        connector
            .description
            .as_deref()
            .map(str::trim)
            .filter(|description| !description.is_empty())
            .map(str::to_string)
    }

    fn set_has_focus(&mut self, has_focus: bool) {
        self.has_focus = has_focus;
    }

    #[allow(dead_code)]
    pub(crate) fn set_input_enabled(&mut self, enabled: bool, placeholder: Option<String>) {
        self.draft.input_enabled = enabled;
        self.draft.input_disabled_placeholder = if enabled { None } else { placeholder };

        // 避免在输入被阻塞时留下交互式弹出窗口打开。
        if !enabled && self.popups.active() {
            self.popups.active = ActivePopup::None;
        }
    }

    pub(crate) fn show_shutdown_in_progress(&mut self) {
        self.set_input_enabled(/*enabled*/ false, Some("Shutting down...".to_string()));
        self.footer.quit_shortcut_expires_at = None;
        self.footer.mode = FooterMode::ComposerEmpty;
        self.footer.hint_override = Some(Vec::new());
        self.footer.plan_mode_nudge_visible = false;
        self.footer.flash = None;
    }

    pub fn set_task_running(&mut self, running: bool) {
        self.is_task_running = running;
    }

    pub(crate) fn set_queue_submissions(&mut self, queue_submissions: bool) {
        self.queue_submissions = queue_submissions;
    }

    pub(crate) fn set_context_window(&mut self, percent: Option<i64>, used_tokens: Option<i64>) {
        if self.footer.context_window_percent == percent
            && self.footer.context_window_used_tokens == used_tokens
        {
            return;
        }
        self.footer.context_window_percent = percent;
        self.footer.context_window_used_tokens = used_tokens;
    }

    pub(crate) fn set_esc_backtrack_hint(&mut self, show: bool) {
        self.footer.esc_backtrack_hint = show;
        if show {
            self.footer.mode = esc_hint_mode(self.footer.mode, self.is_task_running);
        } else {
            self.footer.mode = reset_mode_after_activity(self.footer.mode);
        }
    }

    pub(crate) fn set_status_line(&mut self, status_line: Option<Line<'static>>) -> bool {
        if self.footer.status_line_value == status_line {
            return false;
        }
        self.footer.status_line_value = status_line;
        true
    }

    pub(crate) fn set_status_line_hyperlink(&mut self, url: Option<String>) -> bool {
        if self.footer.status_line_hyperlink_url == url {
            return false;
        }
        self.footer.status_line_hyperlink_url = url;
        true
    }

    pub(crate) fn set_status_line_enabled(&mut self, enabled: bool) -> bool {
        if self.footer.status_line_enabled == enabled {
            return false;
        }
        self.footer.status_line_enabled = enabled;
        true
    }

    pub(crate) fn set_side_conversation_context_label(&mut self, label: Option<String>) -> bool {
        if self.footer.side_conversation_context_label == label {
            return false;
        }
        self.footer.side_conversation_context_label = label;
        true
    }

    /// 替换当前查看代理的上下文底部标签。
    ///
    /// 返回 `false` 表示值未改变，因此调用方可以跳过重绘工作。此
    /// 字段刻意仅作为缓存的展示状态；`ChatComposer` 不会自行推断哪个
    /// 线程处于活动状态。
    pub(crate) fn set_active_agent_label(&mut self, active_agent_label: Option<String>) -> bool {
        if self.footer.active_agent_label == active_agent_label {
            return false;
        }
        self.footer.active_agent_label = active_agent_label;
        true
    }
}

impl Renderable for ChatComposer {
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        self.cursor_pos_with_textarea_right_reserve(area, /*textarea_right_reserve*/ 0)
    }

    fn cursor_style(&self, _area: Rect) -> crossterm::cursor::SetCursorStyle {
        if self.draft.textarea.uses_vim_insert_cursor() {
            crossterm::cursor::SetCursorStyle::SteadyBar
        } else {
            crossterm::cursor::SetCursorStyle::DefaultUserShape
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.desired_height_with_textarea_right_reserve(width, /*textarea_right_reserve*/ 0)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.render_with_mask(area, buf, /*mask_char*/ None);
    }
}

impl ChatComposer {
    pub(crate) fn desired_height_with_textarea_right_reserve(
        &self,
        width: u16,
        textarea_right_reserve: u16,
    ) -> u16 {
        let footer_props = self.footer_props();
        let footer_hint_height = self
            .custom_footer_height()
            .unwrap_or_else(|| footer_height(&footer_props));
        let footer_spacing = Self::footer_spacing(footer_hint_height);
        let footer_total_height = footer_hint_height + footer_spacing;
        const COLS_WITH_MARGIN: u16 = LIVE_PREFIX_COLS + 1;
        let inner_width =
            width.saturating_sub(COLS_WITH_MARGIN.saturating_add(textarea_right_reserve));
        let remote_images_height: u16 = self
            .attachments
            .remote_image_lines()
            .len()
            .try_into()
            .unwrap_or(u16::MAX);
        let remote_images_separator = u16::from(remote_images_height > 0);
        self.draft.textarea.desired_height(inner_width)
            + remote_images_height
            + remote_images_separator
            + 2
            + match &self.popups.active {
                ActivePopup::None => footer_total_height,
                ActivePopup::Command(c) => c.calculate_required_height(width),
                ActivePopup::File(c) => c.calculate_required_height(),
                ActivePopup::Skill(c) => c.calculate_required_height(width),
                ActivePopup::MentionV2(c) => c.calculate_required_height(width),
            }
    }
}

impl ChatComposer {
    pub(crate) fn render_with_mask(&self, area: Rect, buf: &mut Buffer, mask_char: Option<char>) {
        self.render_with_mask_and_textarea_right_reserve(
            area, buf, mask_char, /*textarea_right_reserve*/ 0,
        );
    }

    pub(crate) fn render_with_mask_and_textarea_right_reserve(
        &self,
        area: Rect,
        buf: &mut Buffer,
        mask_char: Option<char>,
        textarea_right_reserve: u16,
    ) {
        let [composer_rect, remote_images_rect, textarea_rect, popup_rect] =
            self.layout_areas_with_textarea_right_reserve(area, textarea_right_reserve);
        match &self.popups.active {
            ActivePopup::Command(popup) => {
                popup.render_ref(popup_rect, buf);
            }
            ActivePopup::File(popup) => {
                popup.render_ref(popup_rect, buf);
            }
            ActivePopup::Skill(popup) => {
                popup.render_ref(popup_rect, buf);
            }
            ActivePopup::MentionV2(popup) => {
                popup.render_ref(popup_rect, buf);
            }
            ActivePopup::None => {
                let footer_props = self.footer_props();
                let show_cycle_hint = !footer_props.is_task_running
                    && self.footer.collaboration_mode_indicator.is_some();
                let show_shortcuts_hint = match footer_props.mode {
                    FooterMode::ComposerEmpty => !self.is_in_paste_burst(),
                    FooterMode::ComposerHasDraft => false,
                    FooterMode::HistorySearch
                    | FooterMode::QuitShortcutReminder
                    | FooterMode::ShortcutOverlay
                    | FooterMode::EscHint => false,
                };
                let show_queue_hint = match footer_props.mode {
                    FooterMode::ComposerHasDraft => footer_props.is_task_running,
                    FooterMode::HistorySearch
                    | FooterMode::QuitShortcutReminder
                    | FooterMode::ComposerEmpty
                    | FooterMode::ShortcutOverlay
                    | FooterMode::EscHint => false,
                };
                let custom_height = self.custom_footer_height();
                let footer_hint_height =
                    custom_height.unwrap_or_else(|| footer_height(&footer_props));
                let footer_spacing = Self::footer_spacing(footer_hint_height);
                let hint_rect = if footer_spacing > 0 && footer_hint_height > 0 {
                    let [_, hint_rect] = Layout::vertical([
                        Constraint::Length(footer_spacing),
                        Constraint::Length(footer_hint_height),
                    ])
                    .areas(popup_rect);
                    hint_rect
                } else {
                    popup_rect
                };
                if let Some(line) = self.history_search_footer_line() {
                    render_footer_line(hint_rect, buf, line);
                } else if self.footer.plan_mode_nudge_visible {
                    let available_width =
                        hint_rect.width.saturating_sub(FOOTER_INDENT_COLS as u16) as usize;
                    render_footer_line(
                        hint_rect,
                        buf,
                        truncate_line_with_ellipsis_if_overflow(
                            plan_mode_nudge_line(),
                            available_width,
                        ),
                    );
                } else {
                    let available_width =
                        hint_rect.width.saturating_sub(FOOTER_INDENT_COLS as u16) as usize;
                    let status_line_active = uses_passive_footer_status_layout(&footer_props);
                    let combined_status_line = if status_line_active {
                        passive_footer_status_line(&footer_props)
                    } else {
                        None
                    };
                    let transition_visible = status_line_active
                        && !self.footer.flash_visible()
                        && self.footer.hint_override.is_none();
                    let transition_active = transition_visible
                        && self
                            .effort_status_line_transition
                            .as_ref()
                            .is_some_and(|transition| !transition.is_finished());
                    let combined_status_line = if transition_visible
                        && let Some(transition) = &self.effort_status_line_transition
                        && !transition.is_finished()
                    {
                        transition.render_line(
                            combined_status_line.as_ref(),
                            hint_rect.width.saturating_sub(FOOTER_INDENT_COLS as u16),
                        )
                    } else {
                        combined_status_line
                    };
                    let mut truncated_status_line = if status_line_active {
                        combined_status_line.as_ref().map(|line| {
                            truncate_line_with_ellipsis_if_overflow(line.clone(), available_width)
                        })
                    } else {
                        None
                    };
                    let left_mode_indicator = if status_line_active {
                        None
                    } else {
                        self.footer.collaboration_mode_indicator
                    };
                    let active_footer_hint_override = self.footer.hint_override.as_ref();
                    let mut left_width = if self.footer.flash_visible() {
                        self.footer
                            .flash
                            .as_ref()
                            .map(|flash| flash.line.width() as u16)
                            .unwrap_or(0)
                    } else if let Some(items) = active_footer_hint_override {
                        footer_hint_items_width(items)
                    } else if status_line_active {
                        truncated_status_line
                            .as_ref()
                            .map(|line| line.width() as u16)
                            .unwrap_or(0)
                    } else {
                        footer_line_width(
                            &footer_props,
                            left_mode_indicator,
                            show_cycle_hint,
                            show_shortcuts_hint,
                            show_queue_hint,
                        )
                    };
                    let right_line =
                        if let Some(label) = self.footer.side_conversation_context_label.as_ref() {
                            Some(side_conversation_context_line(label))
                        } else if let Some(line) = self.shell_mode_footer_line() {
                            Some(line)
                        } else if transition_active {
                            None
                        } else if status_line_active {
                            let full = self.mode_indicator_line(show_cycle_hint);
                            let compact = self.mode_indicator_line(/*show_cycle_hint*/ false);
                            let full_width = full.as_ref().map(|l| l.width() as u16).unwrap_or(0);
                            if can_show_left_with_context(hint_rect, left_width, full_width) {
                                full
                            } else {
                                compact
                            }
                        } else {
                            Some(self.right_footer_line_with_context())
                        };
                    let right_width = right_line.as_ref().map(|l| l.width() as u16).unwrap_or(0);
                    if status_line_active
                        && let Some(max_left) = max_left_width_for_right(hint_rect, right_width)
                        && left_width > max_left
                        && let Some(line) = combined_status_line.as_ref().map(|line| {
                            truncate_line_with_ellipsis_if_overflow(line.clone(), max_left as usize)
                        })
                    {
                        left_width = line.width() as u16;
                        truncated_status_line = Some(line);
                    }
                    let can_show_left_and_context =
                        can_show_left_with_context(hint_rect, left_width, right_width);
                    let has_override =
                        self.footer.flash_visible() || active_footer_hint_override.is_some();
                    let single_line_layout = if has_override || status_line_active {
                        None
                    } else {
                        match footer_props.mode {
                            FooterMode::ComposerEmpty | FooterMode::ComposerHasDraft => {
                                // 这两种模式都渲染单行底部样式（使用
                                // 快捷方式提示或可选队列提示）。我们仍然
                                // 想要单行折叠规则，以便模式标签可以在窄宽度上
                                // 胜过上下文指示器。
                                Some(single_line_footer_layout(
                                    hint_rect,
                                    right_width,
                                    left_mode_indicator,
                                    show_cycle_hint,
                                    show_shortcuts_hint,
                                    show_queue_hint,
                                    footer_props.key_hints,
                                ))
                            }
                            FooterMode::EscHint
                            | FooterMode::HistorySearch
                            | FooterMode::QuitShortcutReminder
                            | FooterMode::ShortcutOverlay => None,
                        }
                    };
                    let show_right = if matches!(
                        footer_props.mode,
                        FooterMode::EscHint
                            | FooterMode::HistorySearch
                            | FooterMode::QuitShortcutReminder
                            | FooterMode::ShortcutOverlay
                    ) {
                        false
                    } else {
                        single_line_layout
                            .as_ref()
                            .map(|(_, show_context)| *show_context)
                            .unwrap_or(can_show_left_and_context)
                    };

                    if let Some((summary_left, _)) = single_line_layout {
                        match summary_left {
                            SummaryLeft::Default => {
                                if status_line_active {
                                    if let Some(line) = truncated_status_line.clone() {
                                        render_footer_line(hint_rect, buf, line);
                                    } else {
                                        render_footer_from_props(
                                            hint_rect,
                                            buf,
                                            &footer_props,
                                            left_mode_indicator,
                                            show_cycle_hint,
                                            show_shortcuts_hint,
                                            show_queue_hint,
                                        );
                                    }
                                } else {
                                    render_footer_from_props(
                                        hint_rect,
                                        buf,
                                        &footer_props,
                                        left_mode_indicator,
                                        show_cycle_hint,
                                        show_shortcuts_hint,
                                        show_queue_hint,
                                    );
                                }
                            }
                            SummaryLeft::Custom(line) => {
                                render_footer_line(hint_rect, buf, line);
                            }
                            SummaryLeft::None => {}
                        }
                    } else if self.footer.flash_visible() {
                        if let Some(flash) = self.footer.flash.as_ref() {
                            flash.line.render(inset_footer_hint_area(hint_rect), buf);
                        }
                    } else if let Some(items) = active_footer_hint_override {
                        render_footer_hint_items(hint_rect, buf, items);
                    } else if status_line_active {
                        if let Some(line) = truncated_status_line {
                            render_footer_line(hint_rect, buf, line);
                        }
                    } else {
                        render_footer_from_props(
                            hint_rect,
                            buf,
                            &footer_props,
                            self.footer.collaboration_mode_indicator,
                            show_cycle_hint,
                            show_shortcuts_hint,
                            show_queue_hint,
                        );
                    }
                    if show_right && let Some(line) = &right_line {
                        render_context_right(hint_rect, buf, line);
                    }
                    if status_line_active
                        && let Some(url) = self.footer.status_line_hyperlink_url.as_deref()
                    {
                        mark_underlined_hyperlink(buf, hint_rect, url);
                    }
                    if transition_visible
                        && let Some(transition) = &self.effort_status_line_transition
                        && !transition.is_finished()
                        && let Some(frame_requester) = &self.frame_requester
                    {
                        frame_requester.schedule_frame_in(EFFORT_STATUS_LINE_FRAME_TICK);
                    }
                }
            }
        }
        let style = user_message_style();
        Block::default().style(style).render_ref(composer_rect, buf);
        if !remote_images_rect.is_empty() {
            Paragraph::new(self.attachments.remote_image_lines())
                .style(style)
                .render_ref(remote_images_rect, buf);
        }
        if !textarea_rect.is_empty() {
            let prompt = if self.draft.input_enabled {
                if self.draft.is_bash_mode {
                    Span::from("!").light_red().bold()
                } else if let Some(tier) = self.effort_tier {
                    let charge = self
                        .effort_ignition
                        .as_ref()
                        .map(EffortIgnition::charge_alpha)
                        .unwrap_or(1.0);
                    tier.prompt(charge)
                } else {
                    "›".bold()
                }
            } else {
                "›".dim()
            };
            buf.set_span(
                textarea_rect.x - LIVE_PREFIX_COLS,
                textarea_rect.y,
                &prompt,
                textarea_rect.width,
            );
        }

        let mut state = self.draft.textarea_state.borrow_mut();
        let textarea_is_empty = self.draft.textarea.text().is_empty() && !self.draft.is_bash_mode;
        if self.draft.input_enabled {
            if let Some(mask_char) = mask_char {
                self.draft
                    .textarea
                    .render_ref_masked(textarea_rect, buf, &mut state, mask_char);
            } else {
                let mut highlights = self.plugin_at_mention_highlights();
                let search_highlight_style =
                    Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD);
                highlights.extend(
                    self.history_search_highlight_ranges()
                        .into_iter()
                        .map(|range| (range, search_highlight_style)),
                );
                // 正文必须带可见前景色：fork ratatui 下 Style::default() 可能不可见。
                let base = if crate::tui_core::terminal_palette::default_fg().is_some() {
                    Style::default()
                } else {
                    Style::default().fg(ratatui::style::Color::White)
                };
                if highlights.is_empty() {
                    self.draft.textarea.render_ref_styled_with_highlights(
                        textarea_rect,
                        buf,
                        &mut state,
                        base,
                        &[],
                    );
                } else {
                    self.draft.textarea.render_ref_styled_with_highlights(
                        textarea_rect,
                        buf,
                        &mut state,
                        base,
                        &highlights,
                    );
                }
            }
        }
        if !self.draft.input_enabled || textarea_is_empty {
            let text = if self.draft.input_enabled {
                self.placeholder_text.as_str().to_string()
            } else {
                self.draft
                    .input_disabled_placeholder
                    .as_deref()
                    .unwrap_or("Input disabled.")
                    .to_string()
            };
            if !textarea_rect.is_empty() {
                let placeholder = Span::from(text).dim();
                Line::from(vec![placeholder])
                    .render_ref(textarea_rect.inner(Margin::new(0, 0)), buf);
            }
        }
        if matches!(self.popups.active, ActivePopup::None)
            && let Some(ignition) = &self.effort_ignition
            && !ignition.is_finished()
        {
            let protected_top = if remote_images_rect.is_empty() {
                textarea_rect.y
            } else {
                remote_images_rect.y
            };
            let protected = Rect::new(
                composer_rect.x,
                protected_top,
                composer_rect.width,
                textarea_rect.bottom().saturating_sub(protected_top),
            );
            if ignition.render(composer_rect, protected, buf)
                && let Some(frame_requester) = &self.frame_requester
            {
                frame_requester.schedule_frame_in(IGNITION_FRAME_TICK);
            }
        }
    }
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
#[path = "chat_composer_effort_tests.rs"]
mod effort_tests;

#[cfg(all(test, feature = "tui-upstream-tests"))]
#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
