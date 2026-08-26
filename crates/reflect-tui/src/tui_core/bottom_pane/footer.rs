//! 底部面板页脚渲染临时提示和上下文指示器。
//!
//! 页脚为纯渲染：它将 `FooterProps` 格式化为 `Line`，不改变任何状态。
//! 它有意不决定「应显示哪个」页脚内容；这由 `ChatComposer`（选择 `FooterMode`）
//! 和更高层状态机（如 `ChatWidget`，决定何时允许退出/中断）拥有。
//!
//! 部分页脚内容基于时间而非事件，例如「再次按以退出」提示。拥有控件调度重绘，
//! 使基于时间的提示即使在 UI 空闲时也能过期。
//!
//! 本模块使用的术语：
//! - 「状态行」指由 `/statusline` 项构建的可配置上下文行，如模型、git 分支和上下文用量。
//! - 「指导性页脚」指告知用户下一步操作的行，如退出确认、快捷键帮助或排队提示。
//! - 「上下文页脚」指页脚可显示环境上下文而非指令。该状态下，页脚可渲染配置的
//!   状态行、活跃 agent 标签、侧边对话状态，或它们的某种组合。
//!
//! 单行折叠概览：
//! 1. Composer 决定当前 `FooterMode` 和提示标志，然后调用 `single_line_footer_layout`
//!    处理基础单行模式。
//! 2. `single_line_footer_layout` 应用基于宽度的回退规则：
//!    （若此描述难以理解，只需调整终端宽度尝试；这些规则基于试错构建。）
//!    - 从最完整的左侧提示加右侧上下文开始。
//!    - 排队提示活跃时，优先保留排队提示可见，即使这意味着更早丢弃右侧上下文；
//!      排队提示在移除前也可能被缩短。
//!    - 排队提示不活跃但模式循环提示适用时，先丢弃「? 查看快捷键」再丢弃
//!      「(shift+tab 循环)」。
//!    - 若「(shift+tab 循环)」无法放入，也隐藏右侧上下文以避免短时间内状态转换过多。
//!    - 最后尝试仅模式行（含/不含上下文），若什么都放不下则回退到无左侧页脚。
//! 3. 折叠选择特定行时，调用方通过 `render_footer_line` 渲染它。否则，调用方
//!    通过 `render_footer_from_props` 渲染直接的模式到文本映射。
//!
//! 简言之：`single_line_footer_layout` 选择「什么」最合适，两个渲染辅助函数
//! 选择是否绘制所选行或默认 `FooterProps` 映射。
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crate::tui_core::render::line_utils::prefix_lines;
use crate::tui_core::status::format_tokens_compact;
use crate::tui_core::ui_consts::FOOTER_INDENT_COLS;
use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

/// 编写器下方页脚区域的渲染输入。
///
/// 调用方应基于更高级别的状态（`ChatComposer`、`BottomPane` 和 `ChatWidget`）构造
/// `FooterProps`，并将其传递给页脚渲染辅助函数（`render_footer_from_props` 或
/// 单行折叠逻辑）。页脚将这些值视为权威数据，不会尝试推断缺失的状态
/// （例如，它不会查询任务是否正在运行）。
#[derive(Clone, Debug)]
pub(crate) struct FooterProps {
    pub(crate) mode: FooterMode,
    pub(crate) esc_backtrack_hint: bool,
    pub(crate) use_shift_enter_hint: bool,
    pub(crate) is_task_running: bool,
    pub(crate) queue_submissions: bool,
    pub(crate) collaboration_modes_enabled: bool,
    pub(crate) is_wsl: bool,
    /// 用户必须再次按下的退出按键。
    ///
    /// 当 `mode` 为 `FooterMode::QuitShortcutReminder` 时渲染该提示。
    pub(crate) quit_shortcut_key: KeyBinding,
    pub(crate) status_line_value: Option<Line<'static>>,
    pub(crate) status_line_enabled: bool,
    pub(crate) key_hints: FooterKeyHints,
    /// 当页脚渲染上下文信息而非指导性提示时显示的活跃 agent 标签。
    ///
    /// 当该标签与配置的状态行同时可用时，它们渲染在同一行上，以 ` · ` 分隔。
    pub(crate) active_agent_label: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CollaborationModeIndicator {
    Plan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GoalStatusIndicator {
    Active { usage: Option<String> },
    Paused,
    Blocked,
    UsageLimited,
    BudgetLimited { usage: Option<String> },
    Complete { usage: Option<String> },
}

const MODE_CYCLE_HINT: &str = "shift+tab to cycle";
const FOOTER_CONTEXT_GAP_COLS: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FooterKeyHints {
    pub(crate) toggle_shortcuts: Option<KeyBinding>,
    pub(crate) queue: Option<KeyBinding>,
    pub(crate) insert_newline: Option<KeyBinding>,
    pub(crate) external_editor: Option<KeyBinding>,
    pub(crate) edit_previous: Option<KeyBinding>,
    pub(crate) show_transcript: Option<KeyBinding>,
    pub(crate) history_search: Option<KeyBinding>,
    pub(crate) reasoning_down: Option<KeyBinding>,
    pub(crate) reasoning_up: Option<KeyBinding>,
}

impl FooterKeyHints {
    #[cfg(test)]
    pub(crate) fn default_bindings() -> Self {
        Self {
            toggle_shortcuts: Some(key_hint::plain(KeyCode::Char('?'))),
            queue: Some(key_hint::plain(KeyCode::Tab)),
            insert_newline: Some(key_hint::ctrl(KeyCode::Char('j'))),
            external_editor: Some(key_hint::ctrl(KeyCode::Char('g'))),
            edit_previous: Some(key_hint::plain(KeyCode::Esc)),
            show_transcript: Some(key_hint::ctrl(KeyCode::Char('t'))),
            history_search: Some(key_hint::ctrl(KeyCode::Char('r'))),
            reasoning_down: Some(key_hint::alt(KeyCode::Char(','))),
            reasoning_up: Some(key_hint::alt(KeyCode::Char('.'))),
        }
    }
}

impl CollaborationModeIndicator {
    fn label(self, show_cycle_hint: bool) -> String {
        let suffix = if show_cycle_hint {
            format!(" ({MODE_CYCLE_HINT})")
        } else {
            String::new()
        };
        match self {
            CollaborationModeIndicator::Plan => format!("Plan mode{suffix}"),
        }
    }

    fn styled_span(self, show_cycle_hint: bool) -> Span<'static> {
        let label = self.label(show_cycle_hint);
        match self {
            CollaborationModeIndicator::Plan => Span::from(label).magenta(),
        }
    }
}

/// 选择要渲染的页脚内容。
///
/// 当前模式由 `ChatComposer` 持有，它会基于瞬时状态覆盖该模式
/// （例如，仅在其计时器激活期间显示 `QuitShortcutReminder`）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FooterMode {
    /// 在 Ctrl+R 搜索激活期间显示的单行增量历史搜索提示。
    HistorySearch,
    /// 「再次按以退出」的瞬时提醒（Ctrl+C/Ctrl+D）。
    QuitShortcutReminder,
    /// 按下 `?` 后显示的多行快捷键覆盖层。
    ShortcutOverlay,
    /// 空闲状态下第一次按 Esc 后显示的「再次按 Esc」瞬时提示。
    EscHint,
    /// 编写器为空时的基础单行页脚。
    ComposerEmpty,
    /// 编写器包含草稿时的基础单行页脚。
    ///
    /// 此处抑制快捷键提示；当任务正在运行时，该模式可改为显示排队提示。
    ComposerHasDraft,
}

pub(crate) fn toggle_shortcut_mode(
    current: FooterMode,
    ctrl_c_hint: bool,
    is_empty: bool,
) -> FooterMode {
    if ctrl_c_hint && matches!(current, FooterMode::QuitShortcutReminder) {
        return current;
    }

    let base_mode = if is_empty {
        FooterMode::ComposerEmpty
    } else {
        FooterMode::ComposerHasDraft
    };

    match current {
        FooterMode::ShortcutOverlay | FooterMode::QuitShortcutReminder => base_mode,
        _ => FooterMode::ShortcutOverlay,
    }
}

pub(crate) fn esc_hint_mode(current: FooterMode, is_task_running: bool) -> FooterMode {
    if is_task_running {
        current
    } else {
        FooterMode::EscHint
    }
}

pub(crate) fn reset_mode_after_activity(current: FooterMode) -> FooterMode {
    match current {
        FooterMode::EscHint
        | FooterMode::ShortcutOverlay
        | FooterMode::QuitShortcutReminder
        | FooterMode::HistorySearch
        | FooterMode::ComposerHasDraft => FooterMode::ComposerEmpty,
        other => other,
    }
}

pub(crate) fn footer_height(props: &FooterProps) -> u16 {
    let show_shortcuts_hint = match props.mode {
        FooterMode::ComposerEmpty => true,
        FooterMode::ComposerHasDraft => false,
        FooterMode::HistorySearch
        | FooterMode::QuitShortcutReminder
        | FooterMode::ShortcutOverlay
        | FooterMode::EscHint => false,
    };
    let show_queue_hint = match props.mode {
        FooterMode::ComposerHasDraft => props.is_task_running,
        FooterMode::QuitShortcutReminder
        | FooterMode::HistorySearch
        | FooterMode::ComposerEmpty
        | FooterMode::ShortcutOverlay
        | FooterMode::EscHint => false,
    };
    footer_from_props_lines(
        props,
        /*collaboration_mode_indicator*/ None,
        /*show_cycle_hint*/ false,
        show_shortcuts_hint,
        show_queue_hint,
    )
    .len() as u16
}

/// 渲染一条预计算好的页脚行。
pub(crate) fn render_footer_line(area: Rect, buf: &mut Buffer, line: Line<'static>) {
    Paragraph::new(prefix_lines(
        vec![line],
        " ".repeat(FOOTER_INDENT_COLS).into(),
        " ".repeat(FOOTER_INDENT_COLS).into(),
    ))
    .render(area, buf);
}

/// 直接从 `FooterProps` 渲染页脚内容。
///
/// 该函数有意不参与基于宽度的折叠/回退逻辑。瞬时的指导性状态
/// （快捷键覆盖层、Esc 提示、退出提醒）优先显示「接下来做什么」的指令，
/// 并当前完全抑制协作模式标签。当折叠逻辑已选定特定的单行时，
/// 优先使用 `render_footer_line`。
pub(crate) fn render_footer_from_props(
    area: Rect,
    buf: &mut Buffer,
    props: &FooterProps,
    collaboration_mode_indicator: Option<CollaborationModeIndicator>,
    show_cycle_hint: bool,
    show_shortcuts_hint: bool,
    show_queue_hint: bool,
) {
    Paragraph::new(prefix_lines(
        footer_from_props_lines(
            props,
            collaboration_mode_indicator,
            show_cycle_hint,
            show_shortcuts_hint,
            show_queue_hint,
        ),
        " ".repeat(FOOTER_INDENT_COLS).into(),
        " ".repeat(FOOTER_INDENT_COLS).into(),
    ))
    .render(area, buf);
}

pub(crate) fn left_fits(area: Rect, left_width: u16) -> bool {
    let max_width = area.width.saturating_sub(FOOTER_INDENT_COLS as u16);
    left_width <= max_width
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SummaryHintKind {
    None,
    Shortcuts,
    QueueMessage,
    QueueShort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LeftSideState {
    hint: SummaryHintKind,
    show_cycle_hint: bool,
}

fn left_side_line(
    collaboration_mode_indicator: Option<CollaborationModeIndicator>,
    state: LeftSideState,
    key_hints: FooterKeyHints,
) -> Line<'static> {
    let mut line = Line::from("");
    match state.hint {
        SummaryHintKind::None => {}
        SummaryHintKind::Shortcuts => {
            if let Some(key) = key_hints.toggle_shortcuts {
                line.push_span(key);
                line.push_span(" for shortcuts".dim());
            }
        }
        SummaryHintKind::QueueMessage => {
            if let Some(key) = key_hints.queue {
                line.push_span(key);
                line.push_span(" to queue message".dim());
            }
        }
        SummaryHintKind::QueueShort => {
            if let Some(key) = key_hints.queue {
                line.push_span(key);
                line.push_span(" to queue".dim());
            }
        }
    };

    if let Some(collaboration_mode_indicator) = collaboration_mode_indicator {
        if !matches!(state.hint, SummaryHintKind::None) {
            line.push_span(" · ".dim());
        }
        line.push_span(collaboration_mode_indicator.styled_span(state.show_cycle_hint));
    }

    line
}

pub(crate) enum SummaryLeft {
    Default,
    Custom(Line<'static>),
    None,
}

/// 计算单行页脚布局，以及右侧上下文指示器能否与其同屏显示。
pub(crate) fn single_line_footer_layout(
    area: Rect,
    context_width: u16,
    collaboration_mode_indicator: Option<CollaborationModeIndicator>,
    show_cycle_hint: bool,
    show_shortcuts_hint: bool,
    show_queue_hint: bool,
    key_hints: FooterKeyHints,
) -> (SummaryLeft, bool) {
    let hint_kind = if show_queue_hint {
        SummaryHintKind::QueueMessage
    } else if show_shortcuts_hint {
        SummaryHintKind::Shortcuts
    } else {
        SummaryHintKind::None
    };
    let default_state = LeftSideState {
        hint: hint_kind,
        show_cycle_hint,
    };
    let default_line = left_side_line(collaboration_mode_indicator, default_state, key_hints);
    let default_width = default_line.width() as u16;
    if default_width > 0 && can_show_left_with_context(area, default_width, context_width) {
        return (SummaryLeft::Default, true);
    }

    let state_line = |state: LeftSideState| -> Line<'static> {
        if state == default_state {
            default_line.clone()
        } else {
            left_side_line(collaboration_mode_indicator, state, key_hints)
        }
    };
    let state_width = |state: LeftSideState| -> u16 { state_line(state).width() as u16 };
    // 当模式循环提示适用时（空闲、非排队模式），仅当
    // 「(shift+tab to cycle)」变体也能放下时，才显示右侧上下文指示器。
    let context_requires_cycle_hint = show_cycle_hint && !show_queue_hint;

    if show_queue_hint {
        // 排队模式下，优先丢弃上下文而不是丢弃排队提示。
        let queue_states = [
            default_state,
            LeftSideState {
                hint: SummaryHintKind::QueueMessage,
                show_cycle_hint: false,
            },
            LeftSideState {
                hint: SummaryHintKind::QueueShort,
                show_cycle_hint: false,
            },
        ];

        // 第 1 轮：只要任一排队变体与右侧上下文指示器能同时放下，
        // 就保留该指示器。我们跳过相邻的重复项，因为
        // `default_state` 可能已经是无循环提示的排队变体。
        let mut previous_state: Option<LeftSideState> = None;
        for state in queue_states {
            if previous_state == Some(state) {
                continue;
            }
            previous_state = Some(state);
            let width = state_width(state);
            if width > 0 && can_show_left_with_context(area, width, context_width) {
                if state == default_state {
                    return (SummaryLeft::Default, true);
                }
                return (SummaryLeft::Custom(state_line(state)), true);
            }
        }

        // 第 2 轮：若上下文放不下，先丢弃上下文再丢弃排队提示。
        // 复用相同的去重逻辑，避免对等价的两种状态重复尝试。
        let mut previous_state: Option<LeftSideState> = None;
        for state in queue_states {
            if previous_state == Some(state) {
                continue;
            }
            previous_state = Some(state);
            let width = state_width(state);
            if width > 0 && left_fits(area, width) {
                if state == default_state {
                    return (SummaryLeft::Default, false);
                }
                return (SummaryLeft::Custom(state_line(state)), false);
            }
        }
    } else if collaboration_mode_indicator.is_some() {
        if show_cycle_hint {
            // 第一个回退：丢弃快捷键提示，但在能放下时保留模式标签上的
            // 循环提示。
            let cycle_state = LeftSideState {
                hint: SummaryHintKind::None,
                show_cycle_hint: true,
            };
            let cycle_width = state_width(cycle_state);
            if cycle_width > 0 && can_show_left_with_context(area, cycle_width, context_width) {
                return (SummaryLeft::Custom(state_line(cycle_state)), true);
            }
            if cycle_width > 0 && left_fits(area, cycle_width) {
                return (SummaryLeft::Custom(state_line(cycle_state)), false);
            }
        }

        // 下一个回退：仅模式标签。若循环提示适用但放不下，
        // 我们也隐藏上下文，以免右侧内容比左侧的
        // 「(shift+tab to cycle)」存在得更久。
        let mode_only_state = LeftSideState {
            hint: SummaryHintKind::None,
            show_cycle_hint: false,
        };
        let mode_only_width = state_width(mode_only_state);
        if !context_requires_cycle_hint
            && mode_only_width > 0
            && can_show_left_with_context(area, mode_only_width, context_width)
        {
            return (
                SummaryLeft::Custom(state_line(mode_only_state)),
                true, // 是否同时显示右侧上下文
            );
        }
        if mode_only_width > 0 && left_fits(area, mode_only_width) {
            return (
                SummaryLeft::Custom(state_line(mode_only_state)),
                false, // 是否同时显示右侧上下文
            );
        }
    }

    // 最终回退：若排队变体（或前面的其他状态）完全放不下，
    // 则丢弃所有提示，仅尝试显示模式标签。
    if let Some(collaboration_mode_indicator) = collaboration_mode_indicator {
        let mode_only_state = LeftSideState {
            hint: SummaryHintKind::None,
            show_cycle_hint: false,
        };
        // 不经过 `state_line` 直接计算宽度，以避免依赖
        // `default_state`（它可能仍是排队变体）。
        let mode_only_width = left_side_line(
            Some(collaboration_mode_indicator),
            mode_only_state,
            key_hints,
        )
        .width() as u16;
        if !context_requires_cycle_hint
            && can_show_left_with_context(area, mode_only_width, context_width)
        {
            return (
                SummaryLeft::Custom(left_side_line(
                    Some(collaboration_mode_indicator),
                    mode_only_state,
                    key_hints,
                )),
                true, // 是否同时显示右侧上下文
            );
        }
        if left_fits(area, mode_only_width) {
            return (
                SummaryLeft::Custom(left_side_line(
                    Some(collaboration_mode_indicator),
                    mode_only_state,
                    key_hints,
                )),
                false, // 是否同时显示右侧上下文
            );
        }
    }

    (SummaryLeft::None, true)
}

pub(crate) fn mode_indicator_line(
    indicator: Option<CollaborationModeIndicator>,
    show_cycle_hint: bool,
) -> Option<Line<'static>> {
    indicator.map(|indicator| Line::from(vec![indicator.styled_span(show_cycle_hint)]))
}

pub(crate) fn goal_status_indicator_line(
    indicator: Option<&GoalStatusIndicator>,
) -> Option<Line<'static>> {
    let indicator = indicator?;
    let label = match indicator {
        GoalStatusIndicator::Active { usage } => {
            if let Some(usage) = usage {
                format!("Pursuing goal ({usage})")
            } else {
                "Pursuing goal".to_string()
            }
        }
        GoalStatusIndicator::Paused => "Goal paused (/goal resume)".to_string(),
        GoalStatusIndicator::Blocked => "Goal blocked (/goal resume)".to_string(),
        GoalStatusIndicator::UsageLimited => "Goal hit usage limits (/goal resume)".to_string(),
        GoalStatusIndicator::BudgetLimited { usage } => {
            if let Some(usage) = usage {
                format!("Goal unmet ({usage})")
            } else {
                "Goal abandoned".to_string()
            }
        }
        GoalStatusIndicator::Complete { usage } => {
            if let Some(usage) = usage {
                format!("Goal achieved ({usage})")
            } else {
                "Goal achieved".to_string()
            }
        }
    };

    Some(Line::from(vec![Span::from(label).magenta()]))
}

pub(crate) fn status_line_right_indicator_line(
    collaboration_mode_indicator: Option<CollaborationModeIndicator>,
    goal_status_indicator: Option<&GoalStatusIndicator>,
    ide_context_active: bool,
    show_cycle_hint: bool,
) -> Option<Line<'static>> {
    let primary_indicator = mode_indicator_line(collaboration_mode_indicator, show_cycle_hint)
        .or_else(|| goal_status_indicator_line(goal_status_indicator));
    let ide_context_indicator = ide_context_active.then(|| Line::from(vec!["IDE context".cyan()]));
    let mut line: Option<Line<'static>> = None;

    for indicator in [primary_indicator, ide_context_indicator]
        .into_iter()
        .flatten()
    {
        if let Some(line) = line.as_mut() {
            line.push_span(" · ".dim());
            for span in indicator.spans {
                line.push_span(span);
            }
        } else {
            line = Some(indicator);
        }
    }

    line
}

pub(crate) fn side_conversation_context_line(label: &str) -> Line<'static> {
    if let Some(rest) = label.strip_prefix("Side ") {
        Line::from(vec!["Side".magenta().bold(), format!(" {rest}").magenta()])
    } else {
        Line::from(label.to_string()).magenta()
    }
}

fn right_aligned_x(area: Rect, content_width: u16) -> Option<u16> {
    if area.is_empty() {
        return None;
    }

    let right_padding = FOOTER_INDENT_COLS as u16;
    let max_width = area.width.saturating_sub(right_padding);
    if content_width == 0 || max_width == 0 {
        return None;
    }

    if content_width >= max_width {
        return Some(area.x.saturating_add(right_padding));
    }

    Some(
        area.x
            .saturating_add(area.width)
            .saturating_sub(content_width)
            .saturating_sub(right_padding),
    )
}

pub(crate) fn max_left_width_for_right(area: Rect, right_width: u16) -> Option<u16> {
    let context_x = right_aligned_x(area, right_width)?;
    let left_start = area.x + FOOTER_INDENT_COLS as u16;

    // 左右之间至少保留一列间隔
    let gap = FOOTER_CONTEXT_GAP_COLS;

    if context_x <= left_start + gap {
        return Some(0);
    }

    Some(context_x.saturating_sub(left_start + gap))
}

pub(crate) fn can_show_left_with_context(area: Rect, left_width: u16, context_width: u16) -> bool {
    let Some(context_x) = right_aligned_x(area, context_width) else {
        return true;
    };
    if left_width == 0 {
        return true;
    }
    let left_extent = FOOTER_INDENT_COLS as u16 + left_width + FOOTER_CONTEXT_GAP_COLS;
    left_extent <= context_x.saturating_sub(area.x)
}

pub(crate) fn render_context_right(area: Rect, buf: &mut Buffer, line: &Line<'static>) {
    if area.is_empty() {
        return;
    }

    let context_width = line.width() as u16;
    let Some(mut x) = right_aligned_x(area, context_width) else {
        return;
    };
    let y = area.y + area.height.saturating_sub(1);
    let max_x = area.x.saturating_add(area.width);

    for span in &line.spans {
        if x >= max_x {
            break;
        }
        let span_width = span.width() as u16;
        if span_width == 0 {
            continue;
        }
        let remaining = max_x.saturating_sub(x);
        let draw_width = span_width.min(remaining);
        buf.set_span(x, y, span, draw_width);
        x = x.saturating_add(span_width);
    }
}

pub(crate) fn inset_footer_hint_area(mut area: Rect) -> Rect {
    if area.width > 2 {
        area.x += 2;
        area.width = area.width.saturating_sub(2);
    }
    area
}

pub(crate) fn render_footer_hint_items(area: Rect, buf: &mut Buffer, items: &[(String, String)]) {
    if items.is_empty() {
        return;
    }

    footer_hint_items_line(items).render(inset_footer_hint_area(area), buf);
}

// ── 提示行渲染辅助与快捷键目录（外移子模块） ──
mod lines;
pub(crate) use lines::*;
mod shortcuts;
use shortcuts::*;

#[cfg(test)]
mod tests;
