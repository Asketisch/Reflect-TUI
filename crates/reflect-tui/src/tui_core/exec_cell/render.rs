use std::time::Instant;

use super::model::CommandOutput;
use super::model::ExecCall;
use super::model::ExecCell;
use crate::ansi_escape::ansi_escape_line;
use crate::app_server_protocol::CommandExecutionSource as ExecCommandSource;
use crate::protocol_compat::parse_command::ParsedCommand;
use crate::shell_command::bash::extract_bash_command;
use crate::tui_core::exec_command::strip_bash_lc_and_escape;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::history_cell::plain_lines;
use crate::tui_core::motion::MotionMode;
use crate::tui_core::motion::ReducedMotionIndicator;
use crate::tui_core::motion::activity_indicator;
use crate::tui_core::render::highlight::highlight_bash_to_lines;
use crate::tui_core::render::line_utils::prefix_lines;
use crate::tui_core::render::line_utils::push_owned_lines;
use crate::tui_core::ui_consts::TRANSCRIPT_HINT;
use crate::tui_core::wrapping::RtOptions;
use crate::tui_core::wrapping::adaptive_wrap_line;
use crate::tui_core::wrapping::adaptive_wrap_lines;
use crate::utils_elapsed::format_duration;
use itertools::Itertools;
use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::style::Stylize;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use textwrap::WordSplitter;
use unicode_width::UnicodeWidthStr;

pub(crate) const TOOL_CALL_MAX_LINES: usize = 5;
const USER_SHELL_TOOL_CALL_MAX_LINES: usize = 50;
const MAX_INTERACTION_PREVIEW_CHARS: usize = 80;

pub(crate) struct OutputLinesParams {
    pub(crate) line_limit: usize,
    pub(crate) only_err: bool,
    pub(crate) include_angle_pipe: bool,
    pub(crate) include_prefix: bool,
}

pub(crate) fn new_active_exec_command(
    call_id: String,
    command: Vec<String>,
    parsed: Vec<ParsedCommand>,
    source: ExecCommandSource,
    interaction_input: Option<String>,
    animations_enabled: bool,
) -> ExecCell {
    ExecCell::new(
        ExecCall {
            call_id,
            command,
            parsed,
            output: None,
            source,
            start_time: Some(Instant::now()),
            duration: None,
            interaction_input,
        },
        animations_enabled,
    )
}

fn format_unified_exec_interaction(command: &[String], input: Option<&str>) -> String {
    let command_display = if let Some(script) = extract_bash_command(command) {
        script
    } else {
        command.join(" ")
    };
    match input {
        Some(data) if !data.is_empty() => {
            let preview = summarize_interaction_input(data);
            format!("Interacted with `{command_display}`, sent `{preview}`")
        }
        _ => format!("Waited for `{command_display}`"),
    }
}

fn summarize_interaction_input(input: &str) -> String {
    let single_line = input.replace('\n', "\\n");
    let sanitized = single_line.replace('`', "\\`");
    if sanitized.chars().count() <= MAX_INTERACTION_PREVIEW_CHARS {
        return sanitized;
    }

    let mut preview = String::new();
    for ch in sanitized.chars().take(MAX_INTERACTION_PREVIEW_CHARS) {
        preview.push(ch);
    }
    preview.push_str("...");
    preview
}

#[derive(Clone)]
pub(crate) struct OutputLines {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) omitted: Option<usize>,
}

pub(crate) fn output_lines(
    output: Option<&CommandOutput>,
    params: OutputLinesParams,
) -> OutputLines {
    let OutputLinesParams {
        line_limit,
        only_err,
        include_angle_pipe,
        include_prefix,
    } = params;
    let output = match output {
        Some(output) if only_err && output.exit_code == 0 => {
            return OutputLines {
                lines: Vec::new(),
                omitted: None,
            };
        }
        Some(output) => output,
        None => {
            return OutputLines {
                lines: Vec::new(),
                omitted: None,
            };
        }
    };

    let (total, retained) = output.line_counts();
    let mut out: Vec<Line<'static>> = Vec::new();

    let head_end = total.min(line_limit).min(retained);
    for (i, raw) in output.lines().take(head_end).enumerate() {
        let mut line = ansi_escape_line(raw.as_ref());
        let prefix = if !include_prefix {
            ""
        } else if i == 0 && include_angle_pipe {
            "  └ "
        } else {
            "    "
        };
        line.spans.insert(0, prefix.into());
        line.spans.iter_mut().for_each(|span| {
            span.style = span.style.add_modifier(Modifier::DIM);
        });
        out.push(line);
    }

    let tail_len = total
        .saturating_sub(head_end)
        .min(line_limit)
        .min(retained.saturating_sub(head_end));
    let omitted = total.saturating_sub(head_end + tail_len);
    let omitted = (omitted > 0).then_some(omitted);
    if let Some(omitted) = omitted {
        out.push(ExecCell::output_ellipsis_line(omitted));
    }

    let tail = output.lines().rev().take(tail_len).collect_vec();
    for raw in tail.into_iter().rev() {
        let mut line = ansi_escape_line(raw.as_ref());
        if include_prefix {
            line.spans.insert(0, "    ".into());
        }
        line.spans.iter_mut().for_each(|span| {
            span.style = span.style.add_modifier(Modifier::DIM);
        });
        out.push(line);
    }

    OutputLines {
        lines: out,
        omitted,
    }
}

fn activity_marker(start_time: Option<Instant>, animations_enabled: bool) -> Span<'static> {
    activity_indicator(
        start_time,
        MotionMode::from_animations_enabled(animations_enabled),
        ReducedMotionIndicator::StaticBullet,
    )
    .unwrap_or_else(|| "•".dim())
}

impl HistoryCell for ExecCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        if self.is_exploring_cell() {
            self.exploring_display_lines(width)
        } else {
            self.command_display_lines(width)
        }
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = vec![];
        for (i, call) in self.iter_calls().enumerate() {
            if i > 0 {
                lines.push("".into());
            }
            let script = strip_bash_lc_and_escape(&call.command);
            let highlighted_script = highlight_bash_to_lines(&script);
            let cmd_display = adaptive_wrap_lines(
                &highlighted_script,
                RtOptions::new(width as usize)
                    .initial_indent("$ ".magenta().into())
                    .subsequent_indent("    ".into()),
            );
            lines.extend(cmd_display);

            if let Some(output) = call.output.as_ref() {
                if !call.is_unified_exec_interaction() {
                    let wrap_width = width.max(1) as usize;
                    let wrap_opts = RtOptions::new(wrap_width);
                    for unwrapped in output
                        .transcript_lines()
                        .map(|line| ansi_escape_line(line.as_ref()))
                    {
                        let wrapped = adaptive_wrap_line(&unwrapped, wrap_opts.clone());
                        push_owned_lines(&wrapped, &mut lines);
                    }
                }
                if let Some(duration) = call.duration {
                    let duration = format_duration(duration);
                    let mut result: Line = if output.exit_code == 0 {
                        Line::from("✓".green().bold())
                    } else {
                        Line::from(vec![
                            "✗".red().bold(),
                            format!(" ({})", output.exit_code).into(),
                        ])
                    };
                    result.push_span(format!(" • {duration}").dim());
                    lines.push(result);
                }
            }
        }
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(self.transcript_lines(u16::MAX))
    }
}

impl ExecCell {
    fn output_ellipsis_text(omitted: usize) -> String {
        format!("… +{omitted} lines ({TRANSCRIPT_HINT})")
    }

    fn output_ellipsis_line(omitted: usize) -> Line<'static> {
        Line::from(vec![Self::output_ellipsis_text(omitted).dim()])
    }

    fn exploring_display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut out: Vec<Line<'static>> = Vec::new();
        out.push(Line::from(vec![
            if self.is_active() {
                activity_marker(self.active_start_time(), self.animations_enabled())
            } else {
                "•".dim()
            },
            " ".into(),
            if self.is_active() {
                "Exploring".bold()
            } else {
                "Explored".bold()
            },
        ]));

        let mut calls = self.calls.as_slice();
        let mut out_indented = Vec::new();
        while let Some((call, remaining)) = calls.split_first() {
            let reads_only = call
                .parsed
                .iter()
                .all(|parsed| matches!(parsed, ParsedCommand::Read { .. }));
            let group_len = if reads_only {
                1 + remaining
                    .iter()
                    .take_while(|next| {
                        next.parsed
                            .iter()
                            .all(|parsed| matches!(parsed, ParsedCommand::Read { .. }))
                    })
                    .count()
            } else {
                1
            };
            let (group, remaining) = calls.split_at(group_len);
            calls = remaining;

            let call_lines: Vec<(&str, Vec<Span<'static>>)> = if reads_only {
                let names = group
                    .iter()
                    .flat_map(|call| &call.parsed)
                    .map(|parsed| match parsed {
                        ParsedCommand::Read { name, .. } => name.clone(),
                        _ => unreachable!(),
                    })
                    .unique();
                vec![(
                    "Read",
                    Itertools::intersperse(names.into_iter().map(Into::into), ", ".dim()).collect(),
                )]
            } else {
                let mut lines = Vec::new();
                for parsed in &call.parsed {
                    match parsed {
                        ParsedCommand::Read { name, .. } => {
                            lines.push(("Read", vec![name.clone().into()]));
                        }
                        ParsedCommand::ListFiles { cmd, path } => {
                            lines.push(("List", vec![path.clone().unwrap_or(cmd.clone()).into()]));
                        }
                        ParsedCommand::Search { cmd, query, path } => {
                            let spans = match (query, path) {
                                (Some(q), Some(p)) => {
                                    vec![q.clone().into(), " in ".dim(), p.clone().into()]
                                }
                                (Some(q), None) => vec![q.clone().into()],
                                _ => vec![cmd.clone().into()],
                            };
                            lines.push(("Search", spans));
                        }
                        ParsedCommand::Unknown { cmd } => {
                            lines.push(("Run", vec![cmd.clone().into()]));
                        }
                    }
                }
                lines
            };

            for (title, line) in call_lines {
                let line = Line::from(line);
                let initial_indent = Line::from(vec![title.cyan(), " ".into()]);
                let subsequent_indent = " ".repeat(initial_indent.width()).into();
                let wrapped = adaptive_wrap_line(
                    &line,
                    RtOptions::new(width as usize)
                        .initial_indent(initial_indent)
                        .subsequent_indent(subsequent_indent),
                );
                push_owned_lines(&wrapped, &mut out_indented);
            }
        }

        out.extend(prefix_lines(out_indented, "  └ ".dim(), "    ".into()));
        out
    }

    fn command_display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let [call] = &self.calls.as_slice() else {
            panic!("Expected exactly one call in a command display cell");
        };
        let layout = EXEC_DISPLAY_LAYOUT;
        let success = call
            .duration
            .and_then(|_| call.output.as_ref().map(|o| o.exit_code == 0));
        let bullet = match success {
            Some(true) => "•".green().bold(),
            Some(false) => "•".red().bold(),
            None => activity_marker(call.start_time, self.animations_enabled()),
        };
        let is_interaction = call.is_unified_exec_interaction();
        let title = if is_interaction {
            ""
        } else if self.is_active() {
            "Running"
        } else if call.is_user_shell_command() {
            "You ran"
        } else {
            "Ran"
        };

        let mut header_line = if is_interaction {
            Line::from(vec![bullet.clone(), " ".into()])
        } else {
            Line::from(vec![bullet.clone(), " ".into(), title.bold(), " ".into()])
        };
        let header_prefix_width = header_line.width();

        let cmd_display = if call.is_unified_exec_interaction() {
            format_unified_exec_interaction(&call.command, call.interaction_input.as_deref())
        } else {
            strip_bash_lc_and_escape(&call.command)
        };
        let highlighted_lines = highlight_bash_to_lines(&cmd_display);

        let continuation_wrap_width = layout.command_continuation.wrap_width(width);
        let continuation_opts =
            RtOptions::new(continuation_wrap_width).word_splitter(WordSplitter::NoHyphenation);

        let mut continuation_lines: Vec<Line<'static>> = Vec::new();

        if let Some((first, rest)) = highlighted_lines.split_first() {
            let available_first_width = (width as usize).saturating_sub(header_prefix_width).max(1);
            let first_opts =
                RtOptions::new(available_first_width).word_splitter(WordSplitter::NoHyphenation);

            let mut first_wrapped: Vec<Line<'static>> = Vec::new();
            push_owned_lines(&adaptive_wrap_line(first, first_opts), &mut first_wrapped);
            let mut first_wrapped_iter = first_wrapped.into_iter();
            if let Some(first_segment) = first_wrapped_iter.next() {
                header_line.extend(first_segment);
            }
            continuation_lines.extend(first_wrapped_iter);

            for line in rest {
                push_owned_lines(
                    &adaptive_wrap_line(line, continuation_opts.clone()),
                    &mut continuation_lines,
                );
            }
        }

        let mut lines: Vec<Line<'static>> = vec![header_line];

        let continuation_lines = Self::limit_lines_from_start(
            &continuation_lines,
            layout.command_continuation_max_lines,
        );
        if !continuation_lines.is_empty() {
            lines.extend(prefix_lines(
                continuation_lines,
                Span::from(layout.command_continuation.initial_prefix).dim(),
                Span::from(layout.command_continuation.subsequent_prefix).dim(),
            ));
        }

        if let Some(output) = call.output.as_ref() {
            let line_limit = if call.is_user_shell_command() {
                USER_SHELL_TOOL_CALL_MAX_LINES
            } else {
                TOOL_CALL_MAX_LINES
            };
            let raw_output = output_lines(
                Some(output),
                OutputLinesParams {
                    line_limit,
                    only_err: false,
                    include_angle_pipe: false,
                    include_prefix: false,
                },
            );
            let display_limit = if call.is_user_shell_command() {
                USER_SHELL_TOOL_CALL_MAX_LINES
            } else {
                layout.output_max_lines
            };

            if raw_output.lines.is_empty() {
                if !call.is_unified_exec_interaction() {
                    lines.extend(prefix_lines(
                        vec![Line::from("(no output)".dim())],
                        Span::from(layout.output_block.initial_prefix).dim(),
                        Span::from(layout.output_block.subsequent_prefix),
                    ));
                }
            } else {
                // 先进行换行，让截断作用于屏幕上实际呈现的行，而不是逻辑行。
                // 这样可以避免少数几行极长的输出淹没整个视口。
                let mut wrapped_output: Vec<Line<'static>> = Vec::new();
                let output_wrap_width = layout.output_block.wrap_width(width);
                let output_opts =
                    RtOptions::new(output_wrap_width).word_splitter(WordSplitter::NoHyphenation);
                for line in &raw_output.lines {
                    push_owned_lines(
                        &adaptive_wrap_line(line, output_opts.clone()),
                        &mut wrapped_output,
                    );
                }

                let prefixed_output = prefix_lines(
                    wrapped_output,
                    Span::from(layout.output_block.initial_prefix).dim(),
                    Span::from(layout.output_block.subsequent_prefix),
                );
                let trimmed_output = Self::truncate_lines_middle(
                    &prefixed_output,
                    display_limit,
                    width,
                    raw_output.omitted,
                    Some(Line::from(
                        Span::from(layout.output_block.subsequent_prefix).dim(),
                    )),
                );

                if !trimmed_output.is_empty() {
                    lines.extend(trimmed_output);
                }
            }
        }

        lines
    }

    fn limit_lines_from_start(lines: &[Line<'static>], keep: usize) -> Vec<Line<'static>> {
        if lines.len() <= keep {
            return lines.to_vec();
        }
        if keep == 0 {
            return vec![Self::ellipsis_line(lines.len())];
        }

        let mut out: Vec<Line<'static>> = lines[..keep].to_vec();
        out.push(Self::ellipsis_line(lines.len() - keep));
        out
    }

    /// 将一组行截断以适配 `max_rows` 个视口行，保留首尾两段并在中间插入省略行。
    ///
    /// `max_rows` 以视口行为单位衡量（即 `Paragraph::wrap` 之后一行真正占用的空间），
    /// 而不是逻辑行。每行的行数开销通过 `Paragraph::line_count` 在给定的 `width` 下计算得出。
    /// 这样可以正确处理包含长 URL（会换行成多行视口行）的单个逻辑行。
    ///
    /// 省略号信息报告被省略的“逻辑行”数（而不是视口行数），这样在不同终端宽度下计数
    /// 能够保持稳定。`omitted_hint` 会沿用上游截断已经报告过的省略数量；`ellipsis_prefix`
    /// 会在省略行前面加上输出侧栏的前缀。
    fn truncate_lines_middle(
        lines: &[Line<'static>],
        max_rows: usize,
        width: u16,
        omitted_hint: Option<usize>,
        ellipsis_prefix: Option<Line<'static>>,
    ) -> Vec<Line<'static>> {
        let width = width.max(1);
        if max_rows == 0 {
            return Vec::new();
        }
        let line_rows: Vec<usize> = lines
            .iter()
            .map(|line| {
                let is_whitespace_only = line
                    .spans
                    .iter()
                    .all(|span| span.content.chars().all(char::is_whitespace));
                if is_whitespace_only {
                    line.width().div_ceil(usize::from(width)).max(1)
                } else {
                    Paragraph::new(Text::from(vec![line.clone()]))
                        .wrap(Wrap { trim: false })
                        .line_count(width)
                        .max(1)
                }
            })
            .collect();
        let total_rows: usize = line_rows.iter().sum();
        if total_rows <= max_rows {
            return lines.to_vec();
        }
        // 为转录提示本身预留空间，使得返回的输出在窄终端上仍然遵守行数预算。
        let estimated_omitted = omitted_hint.unwrap_or(0)
            + lines
                .len()
                .saturating_sub(usize::from(omitted_hint.is_some()));
        let ellipsis_rows =
            Self::output_ellipsis_row_count(estimated_omitted, width, ellipsis_prefix.as_ref());
        if ellipsis_rows >= max_rows {
            return vec![Self::output_ellipsis_line_with_prefix(
                estimated_omitted,
                ellipsis_prefix.as_ref(),
            )];
        }

        let available_rows = max_rows - ellipsis_rows;
        let head_budget = available_rows / 2;
        let tail_budget = available_rows - head_budget;
        let mut head_lines: Vec<Line<'static>> = Vec::new();
        let mut head_rows = 0usize;
        let mut head_end = 0usize;
        while head_end < lines.len() {
            let line_row_count = line_rows[head_end];
            if head_rows + line_row_count > head_budget {
                break;
            }
            head_rows += line_row_count;
            head_lines.push(lines[head_end].clone());
            head_end += 1;
        }

        let mut tail_lines_reversed: Vec<Line<'static>> = Vec::new();
        let mut tail_rows = 0usize;
        let mut tail_start = lines.len();
        while tail_start > head_end {
            let idx = tail_start - 1;
            let line_row_count = line_rows[idx];
            if tail_rows + line_row_count > tail_budget {
                break;
            }
            tail_rows += line_row_count;
            tail_lines_reversed.push(lines[idx].clone());
            tail_start -= 1;
        }

        let mut out = head_lines;
        let base = omitted_hint.unwrap_or(0);
        let additional = lines
            .len()
            .saturating_sub(out.len() + tail_lines_reversed.len())
            .saturating_sub(usize::from(omitted_hint.is_some()));
        out.push(Self::output_ellipsis_line_with_prefix(
            base + additional,
            ellipsis_prefix.as_ref(),
        ));

        out.extend(tail_lines_reversed.into_iter().rev());

        out
    }

    fn ellipsis_line(omitted: usize) -> Line<'static> {
        Line::from(vec![format!("… +{omitted} lines").dim()])
    }

    fn output_ellipsis_row_count(
        omitted: usize,
        width: u16,
        prefix: Option<&Line<'static>>,
    ) -> usize {
        Paragraph::new(Text::from(vec![Self::output_ellipsis_line_with_prefix(
            omitted, prefix,
        )]))
        .wrap(Wrap { trim: false })
        .line_count(width)
        .max(1)
    }

    /// 构造一行输出省略号（`… +N lines (ctrl + t to view transcript)`），
    /// 可选地在前面加上前缀，使省略号与输出侧栏对齐。
    fn output_ellipsis_line_with_prefix(
        omitted: usize,
        prefix: Option<&Line<'static>>,
    ) -> Line<'static> {
        let mut line = prefix.cloned().unwrap_or_default();
        line.push_span(Self::output_ellipsis_text(omitted).dim());
        line
    }
}

#[derive(Clone, Copy)]
struct PrefixedBlock {
    initial_prefix: &'static str,
    subsequent_prefix: &'static str,
}

impl PrefixedBlock {
    const fn new(initial_prefix: &'static str, subsequent_prefix: &'static str) -> Self {
        Self {
            initial_prefix,
            subsequent_prefix,
        }
    }

    fn wrap_width(self, total_width: u16) -> usize {
        let prefix_width = UnicodeWidthStr::width(self.initial_prefix)
            .max(UnicodeWidthStr::width(self.subsequent_prefix));
        usize::from(total_width).saturating_sub(prefix_width).max(1)
    }
}

#[derive(Clone, Copy)]
struct ExecDisplayLayout {
    command_continuation: PrefixedBlock,
    command_continuation_max_lines: usize,
    output_block: PrefixedBlock,
    output_max_lines: usize,
}

impl ExecDisplayLayout {
    const fn new(
        command_continuation: PrefixedBlock,
        command_continuation_max_lines: usize,
        output_block: PrefixedBlock,
        output_max_lines: usize,
    ) -> Self {
        Self {
            command_continuation,
            command_continuation_max_lines,
            output_block,
            output_max_lines,
        }
    }
}

const EXEC_DISPLAY_LAYOUT: ExecDisplayLayout = ExecDisplayLayout::new(
    PrefixedBlock::new("  │ ", "  │ "),
    /*command_continuation_max_lines*/ 2,
    PrefixedBlock::new("  └ ", "    "),
    /*output_max_lines*/ 5,
);

#[cfg(test)]
#[cfg(test)]
mod tests;
