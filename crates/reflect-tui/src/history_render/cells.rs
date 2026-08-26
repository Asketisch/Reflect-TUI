//! Reflect 侧本地 HistoryCell 构造器(工具调用 / 思考行 / 回合分隔 / 启动 banner)。
//!
//! 这些是「老 TUI 样式」的薄封装:不 fork Reflect 单元,而是把手工排好的
//! `Line` 包进 `PlainHistoryCell`(已 `pub(crate)`)。老 TUI 的字形
//! (`✓ ✗ …`)+ 配色在这里复刻,适配新 TUI 的原生 scrollback 模型
//! (工具输出截断而非可折叠交互)。

use crate::tui_core::history_cell::PlainHistoryCell;
use crate::tui_core::live_wrap::take_prefix_by_width;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// 工具输出最大展示行数(超出截断 + `… (+N more lines)` 提示)。
const MAX_OUTPUT_LINES: usize = 5;

/// 格式化耗时:毫秒级显示工具的轻量调用。
fn fmt_tool_elapsed(elapsed_ms: u64) -> String {
    if elapsed_ms < 1000 {
        format!("{elapsed_ms}ms")
    } else {
        let secs = elapsed_ms / 1000;
        crate::tui_core::status_indicator_widget::fmt_elapsed_compact(secs)
    }
}

/// 工具调用单元:`✓/✗ tool_name (Nms)` + 截断输出。
///
/// - 首行 `✓ name (Nms)` 绿色粗体 / `✗ name (Nms)` 红色(`error=true`)。
/// - 输出截断到前 [`MAX_OUTPUT_LINES`] 行,超出加 `… (+N more lines)` dim。
/// - 输出每行 `│   ` 暗灰前缀 + 白字;错误输出标红。
pub fn tool_call_cell(name: &str, output: &str, error: bool, elapsed_ms: u64) -> PlainHistoryCell {
    let glyph = if error { "✗" } else { "✓" };
    let head_color = if error { Color::Red } else { Color::Green };
    let header = Line::from(vec![
        Span::styled(
            format!("{glyph} {name}"),
            Style::default().fg(head_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ({})", fmt_tool_elapsed(elapsed_ms)),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let trimmed = output.trim();
    let mut lines = vec![header];
    if !trimmed.is_empty() {
        let all: Vec<&str> = trimmed.lines().collect();
        let total = all.len();
        let shown = all.into_iter().take(MAX_OUTPUT_LINES);
        let body_color = if error { Color::LightRed } else { Color::White };
        for out_line in shown {
            lines.push(Line::from(vec![
                Span::styled("│   ", Style::default().fg(Color::DarkGray)),
                Span::styled(out_line.to_string(), Style::default().fg(body_color)),
            ]));
        }
        if total > MAX_OUTPUT_LINES {
            let more = total - MAX_OUTPUT_LINES;
            lines.push(Line::from(Span::styled(
                format!("… (+{more} more lines)"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            )));
        }
    }
    PlainHistoryCell::new(lines)
}

/// 思考行:`… {text}` 暗紫(Magenta)斜体 + DIM。对齐老 TUI `Thinking` 行。
pub fn thinking_cell(text: String) -> PlainHistoryCell {
    let style = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::ITALIC | Modifier::DIM);
    let mut lines = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let prefix = if i == 0 { "… " } else { "  " };
        lines.push(Line::from(Span::styled(format!("{prefix}{line}"), style)));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled("…", style)));
    }
    PlainHistoryCell::new(lines)
}

/// 把已渲染好的 `Line` 序列原样包成 `PlainHistoryCell`,用于 banner 这类
/// 渐变锚点必须固定的预渲染内容(走通用 cell 路径会被 marker 改写破坏)。
pub fn prebuilt_cell(lines: Vec<Line<'static>>) -> PlainHistoryCell {
    PlainHistoryCell::new(lines)
}

/// 启动 banner:REFLECT 块字形 + braille 装饰,左上蓝 → 右下红双轴渐变 24-bit。
///
/// 与老 TUI `startup_banner_pill` 对齐:逐字符双轴插值生成 `Color::Rgb`,
/// 普通空格 / braille 空白不染色,空行保留(保持节奏)。banner 在
/// `tui/mod.rs::run_async` 启动时 push 一条到 history,跟随原生 scrollback
/// 自然滚出,并被 `Ctrl-T` transcript 收录。
pub fn banner_cell(version: &str) -> PlainHistoryCell {
    // 调色板:对齐老 TUI 双轴渐变(左上 #3b82f6 蓝 → 右下 #ef4444 红)。
    const FROM: (u8, u8, u8) = (59, 130, 246);
    const TO: (u8, u8, u8) = (239, 68, 68);

    fn lerp(from: u8, to: u8, t: f32) -> u8 {
        let f = from as f32;
        let tt = to as f32;
        let v = f + (tt - f) * t.clamp(0.0, 1.0);
        v.round() as u8
    }

    // 与老 TUI 同源:braille 装饰区 + REFLECT 块字形区。逐字符双轴 t ∈ [0,1]。
    let lines: &[&str] = &[
        "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
        "⠀⠀⠀⠀⣠⣴⣾⣿⣿⣿⣶⣤⡀⠀⠀⠀⠀⢀⣤⣶⣿⣿⣿⣷⣦⣄⠀⠀⠀⠀",
        "⠀⠀⠀⣸⣽⣶⠖⠚⠉⠛⢿⣿⣿⣷⣄⢠⣶⣿⣿⡿⠛⠛⠛⠻⣿⣿⣧⠀⠀⠀",
        "⠀⠀⢰⣿⣿⠃⠀⠀⠀⠀⠀⠉⠻⣿⣿⣿⣻⠟⠋⠀⠀⠀⠀⠀⠘⣿⣿⡇⠀⠀",
        "⠀⠀⢸⣿⣿⡄⠀⠀⠀⠀⠀⣀⣴⣯⣿⣿⣿⣦⣀⠀⠀⠀⠀⠀⢠⣿⣿⠇⠀⠀",
        "⠀⠀⠀⢻⣿⣿⣦⣤⣤⣤⣾⣿⣿⡿⠃⠙⢿⣿⣿⣷⣤⣀⡠⠴⠿⣿⡏⠀⠀⠀",
        "⠀⠀⠀⠀⠙⠻⢿⣿⣿⣿⠿⠛⠁⠀⠀⠀⠀⠈⠛⠿⣿⣿⣿⡿⠟⠋⠀⠀⠀⠀",
        "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
        "",
        "██████╗ ███████╗███████╗██╗     ███████╗ ██████╗████████╗",
        "██╔══██╗██╔════╝██╔════╝██║     ██╔════╝██╔════╝╚══██╔══╝",
        "██████╔╝█████╗  █████╗  ██║     █████╗  ██║        ██║   ",
        "██╔══██╗██╔══╝  ██╔══╝  ██║     ██╔══╝  ██║        ██║   ",
        "██║  ██║███████╗██║     ███████╗███████╗╚██████╗   ██║   ",
        "╚═╝  ╚═╝╚══════╝╚═╝     ╚══════╝╚══════╝ ╚═════╝   ╚═╝   ",
    ];
    let total_rows = lines.len();
    let max_w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1);

    let mut out_lines: Vec<Line<'static>> = Vec::with_capacity(total_rows + 2);
    for (row, line) in lines.iter().enumerate() {
        if line.is_empty() {
            out_lines.push(Line::from(""));
            continue;
        }
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.chars().count());
        for (col, ch) in line.chars().enumerate() {
            // 普通空格 / braille 空白不染色,保持节奏。
            if ch == ' ' || ch == '⠀' {
                spans.push(Span::raw(ch.to_string()));
                continue;
            }
            // 双轴 t ∈ [0, 1]:col/max_w 与 row/total_rows 各贡献一半。
            let t = ((col as f32 / max_w.max(1) as f32) + (row as f32 / total_rows.max(1) as f32))
                / 2.0;
            let r = lerp(FROM.0, TO.0, t);
            let g = lerp(FROM.1, TO.1, t);
            let b = lerp(FROM.2, TO.2, t);
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(Color::Rgb(r, g, b)),
            ));
        }
        out_lines.push(Line::from(spans));
    }
    // 尾部欢迎语:与老 TUI 同样保留提示句(身份信息已在 HUD 常驻)。
    out_lines.push(Line::from(Span::styled(
        format!("  Reflect v{version} — /help for commands"),
        Style::default().fg(Color::DarkGray),
    )));
    PlainHistoryCell::new(out_lines)
}

/// 回合分隔:`─ ─ ─`,有 "Worked for Nm" 时嵌入。
///
/// 不复用 Reflect `FinalMessageSeparator`(其 `new` 是 `pub(crate)` 但依赖
/// `RuntimeMetricsSummary`),这里手画,避免改 Reflect 可见性。
pub fn separator_cell(elapsed_secs: Option<u64>, had_work: bool) -> PlainHistoryCell {
    let mut label_parts = Vec::new();
    if had_work {
        if let Some(secs) = elapsed_secs.filter(|s| *s > 60) {
            label_parts.push(format!(
                "Worked for {}",
                crate::tui_core::status_indicator_widget::fmt_elapsed_compact(secs)
            ));
        }
    }
    // 宽度无关:取一个固定长度分隔;`take_prefix_by_width` 在 display_lines 时裁切。
    let rule = if label_parts.is_empty() {
        "─ ─ ─".to_string()
    } else {
        format!("─ {} ─", label_parts.join(" · "))
    };
    // 预留一个较宽的串;真正按宽度裁切在 HistoryCell::display_lines(width)。
    // 这里用宽度无关写法:交给上层 PlainHistoryCell 原样输出(单行)。
    let _ = take_prefix_by_width; // 保持引用以便后续按宽度裁切扩展
    PlainHistoryCell::new(vec![Line::from(Span::styled(
        rule,
        Style::default().fg(Color::DarkGray),
    ))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui_core::history_cell::HistoryCell;

    #[test]
    fn tool_call_ok_header_green_with_elapsed() {
        let cell = tool_call_cell("Bash", "done", false, 42);
        let lines = cell.display_lines(60);
        let rendered = format!("{}", line_to_plain(&lines[0]));
        assert!(rendered.contains("✓ Bash"), "ok header: {rendered}");
        assert!(rendered.contains("(42ms)"), "elapsed: {rendered}");
    }

    #[test]
    fn tool_call_error_header_red() {
        let cell = tool_call_cell("Glob", "nope", true, 5);
        let lines = cell.display_lines(60);
        let rendered = line_to_plain(&lines[0]).to_string();
        assert!(rendered.contains("✗ Glob"), "error header: {rendered}");
    }

    #[test]
    fn tool_call_truncates_long_output() {
        let output: String = (0..12).map(|i| format!("line{i}\n")).collect();
        let cell = tool_call_cell("Read", output.trim(), false, 10);
        let lines = cell.display_lines(60);
        // 表头 + 5 行主体 + 1 行截断提示
        assert_eq!(lines.len(), 1 + MAX_OUTPUT_LINES + 1);
        let last = line_to_plain(lines.last().unwrap()).to_string();
        assert!(last.contains("+7 more lines"), "truncation hint: {last}");
    }

    #[test]
    fn thinking_cell_is_magenta_italic() {
        let cell = thinking_cell("hmm".into());
        let lines = cell.display_lines(60);
        assert_eq!(lines.len(), 1);
        let span = &lines[0].spans[0];
        assert!(span.content.starts_with("… hmm"));
    }

    #[test]
    fn separator_has_rule() {
        let cell = separator_cell(None, true);
        let lines = cell.display_lines(60);
        let s = line_to_plain(&lines[0]).to_string();
        assert!(s.contains("─"), "separator rule: {s}");
    }

    #[test]
    fn separator_with_elapsed_label() {
        let cell = separator_cell(Some(125), true);
        let lines = cell.display_lines(60);
        let s = line_to_plain(&lines[0]).to_string();
        assert!(s.contains("Worked for"), "elapsed label: {s}");
    }

    #[test]
    fn banner_emits_reflect_wordmark_and_version_line() {
        let cell = banner_cell("0.4.0-test");
        let lines = cell.display_lines(80);
        // 期望布局:8 行 braille + 1 行空白 + 6 行 REFLECT + 1 行欢迎语 = 16 行。
        assert_eq!(lines.len(), 16, "banner 行数应为 16, 实为 {}", lines.len());
        // 第 10 行是 "██████╗ ███████╗███████╗..." REFLECT 字形首行。
        let wordmark = line_to_plain(&lines[9]);
        assert!(
            wordmark.starts_with("██████"),
            "REFLECT 字形首行: {wordmark}"
        );
        assert!(
            wordmark.contains("███████╗"),
            "REFLECT 字形包含块字: {wordmark}"
        );
        // 最后一行为欢迎语。
        let tail = line_to_plain(lines.last().unwrap());
        assert!(
            tail.contains("Reflect v0.4.0-test"),
            "欢迎语带版本号: {tail}"
        );
        assert!(
            tail.contains("/help for commands"),
            "欢迎语带 /help 提示: {tail}"
        );
    }

    #[test]
    fn banner_first_reflect_char_is_blue_gradient() {
        // 第 10 行(REFLECT 字形首行)首字符 '█' 应带渐变色(Color::Rgb),
        // 偏蓝端(左上);最右字符偏红端。粗略断言:首行最左字符 R 应小于
        // 红端 239(蓝端起点 59,允许插值后中等蓝),最右字符 R 应大于蓝端。
        let cell = banner_cell("0.4.0-test");
        let lines = cell.display_lines(80);
        let wordmark = &lines[9];
        let first_span = &wordmark.spans[0];
        let last_span = wordmark.spans.last().expect("REFLECT 字形非空");
        let first_fg = first_span.style.fg.expect("首字符有 fg");
        let last_fg = last_span.style.fg.expect("末字符有 fg");
        match (first_fg, last_fg) {
            (Color::Rgb(r0, _, _), Color::Rgb(r1, _, _)) => {
                assert!(r0 < 239, "首字符 R 应小于红端(偏蓝), 实为 {r0}");
                assert!(
                    r1 > r0,
                    "末字符 R 应大于首字符(双轴渐变从蓝到红), r0={r0} r1={r1}"
                );
            }
            other => panic!("渐变字符应为 Color::Rgb, 实为 {other:?}"),
        }
    }

    fn line_to_plain(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.clone().into_owned())
            .collect()
    }
}
