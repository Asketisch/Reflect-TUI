//! 底栏提示行渲染辅助：footer_from_props_lines 及其衍生函数
//! （passive/status line、shortcuts overlay、context window 等）。
//! 从 footer.rs 抽出，依赖父模块 import 与 shortcuts 子模块的类型。

use super::*;

/// 将 `FooterProps` 映射为页脚行，不进行基于宽度的折叠。
///
/// 这是规范的 FooterMode 到文本的映射。它驱动瞬时的指导性状态
/// （快捷键覆盖层、Esc 提示、退出提醒），也用于在未应用折叠时
/// （或 `single_line_footer_layout` 返回 `SummaryLeft::Default` 时）渲染基础状态的默认内容。
/// 折叠与回退决策位于 `single_line_footer_layout`；本函数仅格式化已选定/默认的内容。
pub(super) fn footer_from_props_lines(
    props: &FooterProps,
    collaboration_mode_indicator: Option<CollaborationModeIndicator>,
    show_cycle_hint: bool,
    show_shortcuts_hint: bool,
    show_queue_hint: bool,
) -> Vec<Line<'static>> {
    let key_hints = props.key_hints;
    // 被动页脚上下文可来自可配置的状态行、活跃 agent 标签，或两者结合。
    if let Some(status_line) = passive_footer_status_line(props) {
        return vec![status_line];
    }
    match props.mode {
        FooterMode::QuitShortcutReminder => {
            vec![quit_shortcut_reminder_line(props.quit_shortcut_key)]
        }
        FooterMode::HistorySearch => vec![Line::from("reverse-i-search: ").dim()],
        FooterMode::ComposerEmpty => {
            let state = LeftSideState {
                hint: if show_shortcuts_hint {
                    SummaryHintKind::Shortcuts
                } else {
                    SummaryHintKind::None
                },
                show_cycle_hint,
            };
            vec![left_side_line(
                collaboration_mode_indicator,
                state,
                key_hints,
            )]
        }
        FooterMode::ShortcutOverlay => {
            let state = ShortcutsState {
                use_shift_enter_hint: props.use_shift_enter_hint,
                esc_backtrack_hint: props.esc_backtrack_hint,
                is_task_running: props.is_task_running,
                queue_submissions: props.queue_submissions,
                is_wsl: props.is_wsl,
                collaboration_modes_enabled: props.collaboration_modes_enabled,
                key_hints,
            };
            shortcut_overlay_lines(state)
        }
        FooterMode::EscHint => vec![esc_hint_line(props.esc_backtrack_hint)],
        FooterMode::ComposerHasDraft => {
            let state = LeftSideState {
                hint: if show_queue_hint {
                    SummaryHintKind::QueueMessage
                } else if show_shortcuts_hint {
                    SummaryHintKind::Shortcuts
                } else {
                    SummaryHintKind::None
                },
                show_cycle_hint,
            };
            vec![left_side_line(
                collaboration_mode_indicator,
                state,
                key_hints,
            )]
        }
    }
}

/// 当页脚未忙于显示指导性提示时，返回上下文页脚行。
///
/// 返回的行可能包含配置的状态行、当前查看的 agent 标签，或两者结合。
/// 退出提醒、快捷键覆盖层和排队提示等活跃的指导性状态会刻意返回 `None`，
/// 以便这些行动提示保持可见。
pub(crate) fn passive_footer_status_line(props: &FooterProps) -> Option<Line<'static>> {
    if !shows_passive_footer_line(props) {
        return None;
    }

    let mut line = if props.status_line_enabled {
        props.status_line_value.clone()
    } else {
        None
    };

    if let Some(active_agent_label) = props.active_agent_label.as_ref() {
        if let Some(existing) = line.as_mut() {
            existing.spans.push(" · ".dim());
            existing.spans.push(active_agent_label.clone().dim());
        } else {
            line = Some(Line::from(active_agent_label.clone()).dim());
        }
    }

    line
}

/// 当前页脚模式是否允许用上下文信息替换指导性提示。
///
/// 实际上这意味着编写器处于空闲状态，或有草稿但未在运行任务，
/// 因此页脚可以将该行用于环境上下文，而非「接下来做什么」的文本。
pub(crate) fn shows_passive_footer_line(props: &FooterProps) -> bool {
    match props.mode {
        FooterMode::ComposerEmpty => true,
        FooterMode::ComposerHasDraft => !props.is_task_running,
        FooterMode::HistorySearch
        | FooterMode::QuitShortcutReminder
        | FooterMode::ShortcutOverlay
        | FooterMode::EscHint => false,
    }
}

/// 调用方是否应为上下文页脚行预留专用的状态行布局。
///
/// 该专用布局用于可配置的 `/statusline` 行。单独的 agent 标签可由标准页脚流程渲染，
/// 因此仅当状态行功能已启用且当前模式允许上下文页脚内容时，此函数才返回 `true`。
pub(crate) fn uses_passive_footer_status_layout(props: &FooterProps) -> bool {
    props.status_line_enabled && shows_passive_footer_line(props)
}

pub(crate) fn footer_line_width(
    props: &FooterProps,
    collaboration_mode_indicator: Option<CollaborationModeIndicator>,
    show_cycle_hint: bool,
    show_shortcuts_hint: bool,
    show_queue_hint: bool,
) -> u16 {
    footer_from_props_lines(
        props,
        collaboration_mode_indicator,
        show_cycle_hint,
        show_shortcuts_hint,
        show_queue_hint,
    )
    .last()
    .map(|line| line.width() as u16)
    .unwrap_or(0)
}

pub(crate) fn footer_hint_items_width(items: &[(String, String)]) -> u16 {
    if items.is_empty() {
        return 0;
    }
    footer_hint_items_line(items).width() as u16
}

pub(super) fn footer_hint_items_line(items: &[(String, String)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(items.len() * 4);
    for (idx, (key, label)) in items.iter().enumerate() {
        spans.push(" ".into());
        spans.push(key.clone().bold());
        spans.push(format!(" {label}").into());
        if idx + 1 != items.len() {
            spans.push("   ".into());
        }
    }
    Line::from(spans)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ShortcutsState {
    pub(super) use_shift_enter_hint: bool,
    pub(super) esc_backtrack_hint: bool,
    pub(super) is_task_running: bool,
    pub(super) queue_submissions: bool,
    pub(super) is_wsl: bool,
    pub(super) collaboration_modes_enabled: bool,
    pub(super) key_hints: FooterKeyHints,
}

pub(super) fn quit_shortcut_reminder_line(key: KeyBinding) -> Line<'static> {
    Line::from(vec![key.into(), " again to quit".into()]).dim()
}

pub(super) fn esc_hint_line(esc_backtrack_hint: bool) -> Line<'static> {
    let esc = key_hint::plain(KeyCode::Esc);
    if esc_backtrack_hint {
        Line::from(vec![esc.into(), " again to edit previous message".into()]).dim()
    } else {
        Line::from(vec![
            esc.into(),
            " ".into(),
            esc.into(),
            " to edit previous message".into(),
        ])
        .dim()
    }
}

pub(super) fn shortcut_overlay_lines(state: ShortcutsState) -> Vec<Line<'static>> {
    let mut commands = Line::from("");
    let mut shell_commands = Line::from("");
    let mut newline = Line::from("");
    let mut queue_message_tab = Line::from("");
    let mut file_paths = Line::from("");
    let mut paste_image = Line::from("");
    let mut external_editor = Line::from("");
    let mut edit_previous = Line::from("");
    let mut history_search = Line::from("");
    let mut quit = Line::from("");
    let mut show_transcript = Line::from("");
    let mut change_mode = Line::from("");
    let mut reasoning_down = Line::from("");
    let mut reasoning_up = Line::from("");

    for descriptor in SHORTCUTS {
        if let Some(text) = descriptor.overlay_entry(state) {
            match descriptor.id {
                ShortcutId::Commands => commands = text,
                ShortcutId::ShellCommands => shell_commands = text,
                ShortcutId::InsertNewline => newline = text,
                ShortcutId::QueueMessageTab => queue_message_tab = text,
                ShortcutId::FilePaths => file_paths = text,
                ShortcutId::PasteImage => paste_image = text,
                ShortcutId::ExternalEditor => external_editor = text,
                ShortcutId::EditPrevious => edit_previous = text,
                ShortcutId::HistorySearch => history_search = text,
                ShortcutId::Quit => quit = text,
                ShortcutId::ShowTranscript => show_transcript = text,
                ShortcutId::ChangeMode => change_mode = text,
                ShortcutId::ReasoningDown => reasoning_down = text,
                ShortcutId::ReasoningUp => reasoning_up = text,
            }
        }
    }

    let mut ordered = vec![
        commands,
        shell_commands,
        newline,
        queue_message_tab,
        file_paths,
        paste_image,
        external_editor,
        edit_previous,
        history_search,
        quit,
        reasoning_down,
        reasoning_up,
    ];
    if change_mode.width() > 0 {
        ordered.push(change_mode);
    }
    ordered.push(show_transcript);

    let mut lines = build_columns(ordered);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        "customize shortcuts with ".into(),
        "/keymap".cyan(),
    ]));
    lines
}

pub(super) fn build_columns(entries: Vec<Line<'static>>) -> Vec<Line<'static>> {
    if entries.is_empty() {
        return Vec::new();
    }

    const COLUMNS: usize = 2;
    const COLUMN_PADDING: [usize; COLUMNS] = [4, 4];
    const COLUMN_GAP: usize = 4;

    let rows = entries.len().div_ceil(COLUMNS);
    let target_len = rows * COLUMNS;
    let mut entries = entries;
    if entries.len() < target_len {
        entries.extend(std::iter::repeat_n(
            Line::from(""),
            target_len - entries.len(),
        ));
    }

    let mut column_widths = [0usize; COLUMNS];

    for (idx, entry) in entries.iter().enumerate() {
        let column = idx % COLUMNS;
        column_widths[column] = column_widths[column].max(entry.width());
    }

    for (idx, width) in column_widths.iter_mut().enumerate() {
        *width += COLUMN_PADDING[idx];
    }

    entries
        .chunks(COLUMNS)
        .map(|chunk| {
            let mut line = Line::from("");
            for (col, entry) in chunk.iter().enumerate() {
                line.extend(entry.spans.clone());
                if col < COLUMNS - 1 {
                    let target_width = column_widths[col];
                    let padding = target_width.saturating_sub(entry.width()) + COLUMN_GAP;
                    line.push_span(Span::from(" ".repeat(padding)));
                }
            }
            line.dim()
        })
        .collect()
}

pub(crate) fn context_window_line(percent: Option<i64>, used_tokens: Option<i64>) -> Line<'static> {
    if let Some(percent) = percent {
        let percent = percent.clamp(0, 100);
        return Line::from(vec![Span::from(format!("{percent}% context left")).dim()]);
    }

    if let Some(tokens) = used_tokens {
        let used_fmt = format_tokens_compact(tokens);
        return Line::from(vec![Span::from(format!("{used_fmt} used")).dim()]);
    }

    Line::from(vec![Span::from("100% context left").dim()])
}
