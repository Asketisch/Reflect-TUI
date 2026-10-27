//! 执行单元渲染（exec_cell/render） 的测试集。
//!
//! 从 exec_cell/render.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::app_server_protocol::CommandExecutionSource as ExecCommandSource;
use pretty_assertions::assert_eq;

fn render_line_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

#[test]
fn user_shell_output_is_limited_by_screen_lines() {
    let long_url_like = format!(
        "https://example.test/api/v1/projects/alpha-team/releases/2026-02-17/builds/1234567890/{}",
        "very-long-segment-".repeat(120),
    );
    let aggregated_output = format!("{long_url_like}\n{long_url_like}\n");

    // 基准：若仅对所有逻辑行做换行而不做任何截断，会得到多少屏幕行？
    let output = CommandOutput::new(/*exit_code*/ 0, aggregated_output);
    let width = 20;
    let layout = EXEC_DISPLAY_LAYOUT;
    let raw_output = output_lines(
        Some(&output),
        OutputLinesParams {
            // 足够大以包含所有逻辑行，且不会触发 `output_lines` 中的省略号。
            line_limit: 100,
            only_err: false,
            include_angle_pipe: false,
            include_prefix: false,
        },
    );
    let output_wrap_width = layout.output_block.wrap_width(width);
    let output_opts = RtOptions::new(output_wrap_width).word_splitter(WordSplitter::NoHyphenation);
    let mut full_wrapped_output: Vec<Line<'static>> = Vec::new();
    for line in &raw_output.lines {
        push_owned_lines(
            &adaptive_wrap_line(line, output_opts.clone()),
            &mut full_wrapped_output,
        );
    }
    let full_prefixed_output = prefix_lines(
        full_wrapped_output,
        Span::from(layout.output_block.initial_prefix).dim(),
        Span::from(layout.output_block.subsequent_prefix),
    );
    let full_screen_lines = Paragraph::new(Text::from(full_prefixed_output))
        .wrap(Wrap { trim: false })
        .line_count(width);

    // 健全性检查：在不应用任何截断时，这个场景应当产生多于用户 shell 单次调用上限的屏幕行。
    // 如果该检查失败，说明该测试已无法覆盖相应的回归问题。
    assert!(
        full_screen_lines > USER_SHELL_TOOL_CALL_MAX_LINES,
        "expected unbounded wrapping to produce more than {USER_SHELL_TOOL_CALL_MAX_LINES} screen lines, got {full_screen_lines}",
    );

    let call = ExecCall {
        call_id: "call-id".to_string(),
        command: vec!["bash".into(), "-lc".into(), "echo long".into()],
        parsed: Vec::new(),
        output: Some(output),
        source: ExecCommandSource::UserShell,
        start_time: None,
        duration: None,
        interaction_input: None,
    };

    let cell = ExecCell::new(call, /*animations_enabled*/ false);

    // 使用较窄的宽度，使每个逻辑行都被换行成多行屏幕行。
    let lines = cell.command_display_lines(width);
    let rendered_rows = Paragraph::new(Text::from(lines.clone()))
        .wrap(Wrap { trim: false })
        .line_count(width);
    let header_rows = Paragraph::new(Text::from(vec![lines[0].clone()]))
        .wrap(Wrap { trim: false })
        .line_count(width);
    let output_screen_rows = rendered_rows.saturating_sub(header_rows);

    let contains_ellipsis = lines
        .iter()
        .any(|line| line.spans.iter().any(|span| span.content.contains("… +")));

    // 回归保护：在此之前，由于截断发生在最终视口换行之前，这种场景可能渲染出数百行
    // 被换行后的行。按行数感知的截断现在可以限制可见的输出行数。
    assert!(
        output_screen_rows <= USER_SHELL_TOOL_CALL_MAX_LINES,
        "expected at most {USER_SHELL_TOOL_CALL_MAX_LINES} output rows, got {output_screen_rows} (total rows: {rendered_rows})",
    );
    assert!(
        contains_ellipsis,
        "expected truncated output to include an ellipsis line"
    );
    let normalized = lines
        .iter()
        .map(render_line_text)
        .join(" ")
        .split_whitespace()
        .join(" ");
    assert!(
        normalized.contains(TRANSCRIPT_HINT),
        "expected truncated output to advertise transcript shortcut, got {normalized}"
    );
}

#[test]
fn truncate_lines_middle_keeps_omitted_count_in_line_units() {
    let lines = vec![
        Line::from("  └ short"),
        Line::from("    this-is-a-very-long-token-that-wraps-many-rows"),
        Line::from(format!(
            "    {}",
            ExecCell::output_ellipsis_text(/*omitted*/ 4)
        )),
        Line::from("    tail"),
    ];

    let truncated = ExecCell::truncate_lines_middle(
        &lines,
        /*max_rows*/ 2,
        /*width*/ 80,
        Some(4),
        Some(Line::from("    ".dim())),
    );
    let rendered: Vec<String> = truncated.iter().map(render_line_text).collect();

    assert!(
        rendered
            .iter()
            .any(|line| line.contains("… +6 lines (ctrl + t to view transcript)")),
        "expected omitted hint to count hidden lines (not wrapped rows), got: {rendered:?}"
    );
}

#[test]
fn output_lines_ellipsis_includes_transcript_hint() {
    let output = CommandOutput::new(
        /*exit_code*/ 0,
        (1..=7).map(|n| n.to_string()).join("\n"),
    );

    let rendered: Vec<String> = output_lines(
        Some(&output),
        OutputLinesParams {
            line_limit: 2,
            only_err: false,
            include_angle_pipe: false,
            include_prefix: false,
        },
    )
    .lines
    .iter()
    .map(render_line_text)
    .collect();

    assert_eq!(
        rendered,
        vec![
            "1",
            "2",
            "… +3 lines (ctrl + t to view transcript)",
            "6",
            "7",
        ]
    );
}

#[test]
fn output_lines_handles_newline_dense_output_without_materializing_every_line() {
    let output = CommandOutput::new(/*exit_code*/ 0, "\n".repeat(100_000));

    let rendered = output_lines(
        Some(&output),
        OutputLinesParams {
            line_limit: 5,
            only_err: false,
            include_angle_pipe: false,
            include_prefix: false,
        },
    );

    assert_eq!(rendered.lines.len(), 11);
    assert_eq!(rendered.omitted, Some(99_990));
}

#[test]
fn streamed_output_renders_head_tail_previews() {
    let mut cell = new_active_exec_command(
        "call-id".to_string(),
        vec!["bash".into(), "-lc".into(), "echo output".into()],
        Vec::new(),
        ExecCommandSource::Agent,
        /*interaction_input*/ None,
        /*animations_enabled*/ false,
    );
    for line in 1..=160 {
        assert!(cell.append_output("call-id", &format!("line {line}\n")));
    }
    let output = cell.calls[0].output.as_ref().expect("streamed output");

    let agent = output_lines(
        Some(output),
        OutputLinesParams {
            line_limit: TOOL_CALL_MAX_LINES,
            only_err: false,
            include_angle_pipe: false,
            include_prefix: false,
        },
    );
    assert_eq!(agent.lines.len(), 11);
    assert_eq!(agent.omitted, Some(150));
    assert_eq!(render_line_text(&agent.lines[0]), "line 1");
    assert_eq!(render_line_text(&agent.lines[10]), "line 160");

    let user_shell = output_lines(
        Some(output),
        OutputLinesParams {
            line_limit: USER_SHELL_TOOL_CALL_MAX_LINES,
            only_err: false,
            include_angle_pipe: false,
            include_prefix: false,
        },
    );
    assert_eq!(user_shell.lines.len(), 101);
    assert_eq!(user_shell.omitted, Some(60));
    assert_eq!(render_line_text(&user_shell.lines[0]), "line 1");
    assert_eq!(render_line_text(&user_shell.lines[100]), "line 160");
}

#[test]
fn truncated_live_output_preview_and_transcript_snapshot() {
    let mut cell = new_active_exec_command(
        "call-id".to_string(),
        vec!["bash".into(), "-lc".into(), "echo output".into()],
        Vec::new(),
        ExecCommandSource::Agent,
        /*interaction_input*/ None,
        /*animations_enabled*/ false,
    );
    let hidden = "\x1b[2m".repeat(300_000);
    let output = format!(
        "\x1b[31mhead error that wraps onto the next row\x1b[0m{hidden}\x1b[32mtail output that also wraps\x1b[0m"
    );
    assert!(cell.append_output("call-id", &output));

    let preview = cell.display_lines(/*width*/ 60);
    cell.calls[0].start_time = None;
    cell.mark_failed();
    let transcript = cell.transcript_lines(/*width*/ 60);

    insta::assert_debug_snapshot!(
        "truncated_live_output_preview_and_transcript",
        (preview, transcript)
    );
}

#[test]
fn command_truncation_ellipsis_does_not_include_transcript_hint() {
    let truncated = ExecCell::limit_lines_from_start(
        &[
            Line::from("first"),
            Line::from("second"),
            Line::from("third"),
        ],
        /*keep*/ 2,
    );
    let rendered: Vec<String> = truncated.iter().map(render_line_text).collect();

    assert_eq!(
        rendered,
        vec![
            "first".to_string(),
            "second".to_string(),
            "… +1 lines".to_string(),
        ]
    );
}

#[test]
fn truncate_lines_middle_does_not_truncate_blank_prefixed_output_lines() {
    let mut lines = vec![Line::from("  └ start")];
    lines.extend(std::iter::repeat_n(Line::from("    "), 26));
    lines.push(Line::from("    end"));

    let truncated = ExecCell::truncate_lines_middle(
        &lines, /*max_rows*/ 28, /*width*/ 80, /*omitted_hint*/ None,
        /*ellipsis_prefix*/ None,
    );

    assert_eq!(truncated, lines);
}

#[test]
fn command_display_does_not_split_long_url_token() {
    let url = "http://example.com/long-url-with-dashes-wider-than-terminal-window/blah-blah-blah-text/more-gibberish-text";

    let call = ExecCall {
        call_id: "call-id".to_string(),
        command: vec!["bash".into(), "-lc".into(), format!("echo {url}")],
        parsed: Vec::new(),
        output: None,
        source: ExecCommandSource::UserShell,
        start_time: None,
        duration: None,
        interaction_input: None,
    };

    let cell = ExecCell::new(call, /*animations_enabled*/ false);
    let rendered: Vec<String> = cell
        .command_display_lines(/*width*/ 36)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();

    assert_eq!(
        rendered.iter().filter(|line| line.contains(url)).count(),
        1,
        "expected full URL in one rendered line, got: {rendered:?}"
    );
}

#[test]
fn active_command_without_animations_is_stable() {
    let call = ExecCall {
        call_id: "call-id".to_string(),
        command: vec!["bash".into(), "-lc".into(), "echo done".into()],
        parsed: Vec::new(),
        output: None,
        source: ExecCommandSource::Agent,
        start_time: Some(Instant::now()),
        duration: None,
        interaction_input: None,
    };

    let cell = ExecCell::new(call, /*animations_enabled*/ false);
    let first: Vec<String> = cell
        .command_display_lines(/*width*/ 80)
        .iter()
        .map(render_line_text)
        .collect();
    let second: Vec<String> = cell
        .command_display_lines(/*width*/ 80)
        .iter()
        .map(render_line_text)
        .collect();

    assert_eq!(first, second);
    assert_eq!(first, vec!["• Running echo done".to_string()]);
}

#[test]
fn exploring_display_does_not_split_long_url_like_search_query() {
    let url_like = "example.test/api/v1/projects/alpha-team/releases/2026-02-17/builds/1234567890/artifacts/reports/performance/summary/detail/with/a/very/long/path";
    let call = ExecCall {
        call_id: "call-id".to_string(),
        command: vec!["bash".into(), "-lc".into(), "rg foo".into()],
        parsed: vec![ParsedCommand::Search {
            cmd: format!("rg {url_like}"),
            query: Some(url_like.to_string()),
            path: None,
        }],
        output: None,
        source: ExecCommandSource::Agent,
        start_time: None,
        duration: None,
        interaction_input: None,
    };

    let cell = ExecCell::new(call, /*animations_enabled*/ false);
    let rendered: Vec<String> = cell
        .display_lines(/*width*/ 36)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();

    assert_eq!(
        rendered
            .iter()
            .filter(|line| line.contains(url_like))
            .count(),
        1,
        "expected full URL-like query in one rendered line, got: {rendered:?}"
    );
}

#[test]
fn output_display_does_not_split_long_url_like_token_without_scheme() {
    let url = "example.test/api/v1/projects/alpha-team/releases/2026-02-17/builds/1234567890/artifacts/reports/performance/summary/detail/session_id=abc123def456ghi789jkl012mno345pqr678";

    let call = ExecCall {
        call_id: "call-id".to_string(),
        command: vec!["bash".into(), "-lc".into(), "echo done".into()],
        parsed: Vec::new(),
        output: Some(CommandOutput::new(/*exit_code*/ 0, url.to_string())),
        source: ExecCommandSource::UserShell,
        start_time: None,
        duration: None,
        interaction_input: None,
    };

    let cell = ExecCell::new(call, /*animations_enabled*/ false);
    let rendered: Vec<String> = cell
        .command_display_lines(/*width*/ 36)
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();

    assert_eq!(
        rendered.iter().filter(|line| line.contains(url)).count(),
        1,
        "expected full URL-like token in one rendered line, got: {rendered:?}"
    );
}

#[test]
fn desired_transcript_height_accounts_for_wrapped_url_like_rows() {
    let url = "https://example.test/api/v1/projects/alpha-team/releases/2026-02-17/builds/1234567890/artifacts/reports/performance/summary/detail/with/a/very/long/path/that/keeps/going/for/testing/purposes";
    let call = ExecCall {
        call_id: "call-id".to_string(),
        command: vec!["bash".into(), "-lc".into(), "echo done".into()],
        parsed: Vec::new(),
        output: Some(CommandOutput::new(/*exit_code*/ 0, url.to_string())),
        source: ExecCommandSource::Agent,
        start_time: None,
        duration: None,
        interaction_input: None,
    };

    let cell = ExecCell::new(call, /*animations_enabled*/ false);
    let width: u16 = 36;
    let logical_height = cell.transcript_lines(width).len() as u16;
    let wrapped_height = cell.desired_transcript_height(width);

    assert!(
        wrapped_height > logical_height,
        "expected transcript height to account for wrapped URL-like rows, logical_height={logical_height}, wrapped_height={wrapped_height}"
    );
}
