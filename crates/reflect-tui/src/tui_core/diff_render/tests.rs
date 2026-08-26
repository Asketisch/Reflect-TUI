//! diff 渲染（DiffRender） 的测试集。
//!
//! 从 diff_render.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use insta::assert_debug_snapshot;
use insta::assert_snapshot;
use pretty_assertions::assert_eq;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

#[test]
fn ansi16_add_style_uses_foreground_only() {
    let style = style_add(
        DiffTheme::Dark,
        DiffColorLevel::Ansi16,
        fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi16),
    );
    assert_eq!(style.fg, Some(Color::Green));
    assert_eq!(style.bg, None);
}

#[test]
fn ansi16_del_style_uses_foreground_only() {
    let style = style_del(
        DiffTheme::Dark,
        DiffColorLevel::Ansi16,
        fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi16),
    );
    assert_eq!(style.fg, Some(Color::Red));
    assert_eq!(style.bg, None);
}

#[test]
fn ansi16_sign_styles_use_foreground_only() {
    let add_sign = style_sign_add(
        DiffTheme::Dark,
        DiffColorLevel::Ansi16,
        fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi16),
    );
    assert_eq!(add_sign.fg, Some(Color::Green));
    assert_eq!(add_sign.bg, None);

    let del_sign = style_sign_del(
        DiffTheme::Dark,
        DiffColorLevel::Ansi16,
        fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi16),
    );
    assert_eq!(del_sign.fg, Some(Color::Red));
    assert_eq!(del_sign.bg, None);
}
fn diff_summary_for_tests(changes: &HashMap<PathBuf, FileChange>) -> Vec<RtLine<'static>> {
    create_diff_summary(changes, &PathBuf::from("/"), /*wrap_cols*/ 80)
}

fn snapshot_lines(name: &str, lines: Vec<RtLine<'static>>, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|f| {
            Paragraph::new(Text::from(lines))
                .wrap(Wrap { trim: false })
                .render_ref(f.area(), f.buffer_mut())
        })
        .expect("draw");
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| !cell.symbol().contains('\t')),
        "diff buffer should not contain literal tabs"
    );
    assert_snapshot!(name, terminal.backend());
}

fn display_width(text: &str) -> usize {
    text.chars()
        .map(|ch| ch.width().unwrap_or(if ch == '\t' { TAB_WIDTH } else { 0 }))
        .sum()
}

fn line_display_width(line: &RtLine<'static>) -> usize {
    line.spans
        .iter()
        .map(|span| display_width(span.content.as_ref()))
        .sum()
}

fn snapshot_lines_text(name: &str, lines: &[RtLine<'static>]) {
    // 将 Lines 转换为纯文本行并去除尾随空格，以便在快照中
    // 更容易直观地验证缩进。
    let text = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .map(|s| s.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_snapshot!(name, text);
}

fn diff_gallery_changes() -> HashMap<PathBuf, FileChange> {
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();

    let rust_original =
        "fn greet(name: &str) {\n    println!(\"hello\");\n    println!(\"bye\");\n}\n";
    let rust_modified = "fn greet(name: &str) {\n    println!(\"hello {name}\");\n    println!(\"emoji: 🚀✨ and CJK: 你好世界\");\n}\n";
    let rust_patch = diffy::create_patch(rust_original, rust_modified).to_string();
    changes.insert(
        PathBuf::from("src/lib.rs"),
        FileChange::Update {
            unified_diff: rust_patch,
            move_path: None,
        },
    );

    let py_original = "def add(a, b):\n\treturn a + b\n\nprint(add(1, 2))\n";
    let py_modified = "def add(a, b):\n\treturn a + b + 42\n\nprint(add(1, 2))\n";
    let py_patch = diffy::create_patch(py_original, py_modified).to_string();
    changes.insert(
        PathBuf::from("scripts/calc.txt"),
        FileChange::Update {
            unified_diff: py_patch,
            move_path: Some(PathBuf::from("scripts/calc.py")),
        },
    );

    changes.insert(
        PathBuf::from("assets/banner.txt"),
        FileChange::Add {
            content: "HEADER\tVALUE\nrocket\t🚀\ncity\t東京\n".to_string(),
        },
    );
    changes.insert(
        PathBuf::from("examples/new_sample.rs"),
        FileChange::Add {
            content: "pub fn greet(name: &str) {\n    println!(\"Hello, {name}!\");\n}\n"
                .to_string(),
        },
    );

    changes.insert(
        PathBuf::from("tmp/obsolete.log"),
        FileChange::Delete {
            content: "old line 1\nold line 2\nold line 3\n".to_string(),
        },
    );
    changes.insert(
        PathBuf::from("legacy/old_script.py"),
        FileChange::Delete {
            content: "def legacy(x):\n    return x + 1\nprint(legacy(3))\n".to_string(),
        },
    );

    changes
}

fn snapshot_diff_gallery(name: &str, width: u16, height: u16) {
    let lines = create_diff_summary(
        &diff_gallery_changes(),
        &PathBuf::from("/"),
        usize::from(width),
    );
    snapshot_lines(name, lines, width, height);
}

#[test]
fn display_path_prefers_cwd_without_git_repo() {
    let cwd = if cfg!(windows) {
        PathBuf::from(r"C:\workspace\reflect")
    } else {
        PathBuf::from("/workspace/reflect")
    };
    let path = cwd.join("tui").join("example.png");

    let rendered = display_path_for(&path, &cwd);

    assert_eq!(
        rendered,
        PathBuf::from("tui")
            .join("example.png")
            .display()
            .to_string()
    );
}

#[test]
fn ui_snapshot_wrap_behavior_insert() {
    // 较窄的宽度以在我们的 diff 行渲染中强制换行
    let long_line =
        "this is a very long line that should wrap across multiple terminal columns and continue";

    // 直接调用换行函数，以便精确控制宽度
    let lines = push_wrapped_diff_line_with_style_context(
        /*line_number*/ 1,
        DiffLineType::Insert,
        long_line,
        /*width*/ 80,
        line_number_width(/*max_line_number*/ 1),
        current_diff_render_style_context(),
    );

    // 渲染到小终端中以捕获可视化布局
    snapshot_lines(
        "wrap_behavior_insert",
        lines,
        /*width*/ 90,
        /*height*/ 8,
    );
}

#[test]
fn ui_snapshot_apply_update_block() {
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    let original = "line one\nline two\nline three\n";
    let modified = "line one\nline two changed\nline three\n";
    let patch = diffy::create_patch(original, modified).to_string();

    changes.insert(
        PathBuf::from("example.txt"),
        FileChange::Update {
            unified_diff: patch,
            move_path: None,
        },
    );

    let lines = diff_summary_for_tests(&changes);

    snapshot_lines(
        "apply_update_block",
        lines,
        /*width*/ 80,
        /*height*/ 12,
    );
}

#[test]
fn ui_snapshot_apply_update_with_rename_block() {
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    let original = "A\nB\nC\n";
    let modified = "A\nB changed\nC\n";
    let patch = diffy::create_patch(original, modified).to_string();

    changes.insert(
        PathBuf::from("old_name.rs"),
        FileChange::Update {
            unified_diff: patch,
            move_path: Some(PathBuf::from("new_name.rs")),
        },
    );

    let lines = diff_summary_for_tests(&changes);

    snapshot_lines(
        "apply_update_with_rename_block",
        lines,
        /*width*/ 80,
        /*height*/ 12,
    );
}

#[test]
fn ui_snapshot_apply_multiple_files_block() {
    // 两个文件：一个更新和一个新增，以演练组合标题和逐文件行
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();

    // 文件 a.txt：单行替换（一处删除，一处新增）
    let patch_a = diffy::create_patch("one\n", "one changed\n").to_string();
    changes.insert(
        PathBuf::from("a.txt"),
        FileChange::Update {
            unified_diff: patch_a,
            move_path: None,
        },
    );

    // 文件 b.txt：新增一行
    changes.insert(
        PathBuf::from("b.txt"),
        FileChange::Add {
            content: "new\n".to_string(),
        },
    );

    let lines = diff_summary_for_tests(&changes);

    snapshot_lines(
        "apply_multiple_files_block",
        lines,
        /*width*/ 80,
        /*height*/ 14,
    );
}

#[test]
fn ui_snapshot_apply_add_block() {
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("new_file.txt"),
        FileChange::Add {
            content: "alpha\nbeta\n".to_string(),
        },
    );

    let lines = diff_summary_for_tests(&changes);

    snapshot_lines(
        "apply_add_block",
        lines,
        /*width*/ 80,
        /*height*/ 10,
    );
}

#[test]
fn ui_snapshot_apply_delete_block() {
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("tmp_delete_example.txt"),
        FileChange::Delete {
            content: "first\nsecond\nthird\n".to_string(),
        },
    );

    let lines = diff_summary_for_tests(&changes);
    snapshot_lines(
        "apply_delete_block",
        lines,
        /*width*/ 80,
        /*height*/ 12,
    );
}

#[test]
fn ui_snapshot_apply_update_block_wraps_long_lines() {
    // 创建一个具有长修改行的 patch 以强制换行
    let original = "line 1\nshort\nline 3\n";
    let modified = "line 1\nshort this_is_a_very_long_modified_line_that_should_wrap_across_multiple_terminal_columns_and_continue_even_further_beyond_eighty_columns_to_force_multiple_wraps\nline 3\n";
    let patch = diffy::create_patch(original, modified).to_string();

    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("long_example.txt"),
        FileChange::Update {
            unified_diff: patch,
            move_path: None,
        },
    );

    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 72);

    // 使用比换行宽度更宽的后端宽度进行渲染，以避免 Paragraph 自动换行。
    snapshot_lines(
        "apply_update_block_wraps_long_lines",
        lines,
        /*width*/ 80,
        /*height*/ 12,
    );
}

#[test]
fn ui_snapshot_apply_update_block_wraps_long_lines_text() {
    // 这与所需的布局示例相对应：符号仅出现在第一个新增行上，
    // 后续的换行片段从行号 gutter 下方开始对齐。
    let original = "1\n2\n3\n4\n";
    let modified = "1\nadded long line which wraps and_if_there_is_a_long_token_it_will_be_broken\n3\n4 context line which also wraps across\n";
    let patch = diffy::create_patch(original, modified).to_string();

    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("wrap_demo.txt"),
        FileChange::Update {
            unified_diff: patch,
            move_path: None,
        },
    );

    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 28);
    snapshot_lines_text("apply_update_block_wraps_long_lines_text", &lines);
}

#[test]
fn ui_snapshot_apply_update_block_line_numbers_three_digits_text() {
    let original = (1..=110).map(|i| format!("line {i}\n")).collect::<String>();
    let modified = (1..=110)
        .map(|i| {
            if i == 100 {
                format!("line {i} changed\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect::<String>();
    let patch = diffy::create_patch(&original, &modified).to_string();

    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("hundreds.txt"),
        FileChange::Update {
            unified_diff: patch,
            move_path: None,
        },
    );

    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 80);
    snapshot_lines_text("apply_update_block_line_numbers_three_digits_text", &lines);
}

#[test]
fn ui_snapshot_apply_update_block_relativizes_path() {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let abs_old = cwd.join("abs_old.rs");
    let abs_new = cwd.join("abs_new.rs");

    let original = "X\nY\n";
    let modified = "X changed\nY\n";
    let patch = diffy::create_patch(original, modified).to_string();

    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        abs_old,
        FileChange::Update {
            unified_diff: patch,
            move_path: Some(abs_new),
        },
    );

    let lines = create_diff_summary(&changes, &cwd, /*wrap_cols*/ 80);

    snapshot_lines(
        "apply_update_block_relativizes_path",
        lines,
        /*width*/ 80,
        /*height*/ 10,
    );
}

#[test]
fn ui_snapshot_syntax_highlighted_insert_wraps() {
    // 一行超过 80 列的 Rust 行，带语法高亮应
    // 换行到多个输出行，而不是被截断。
    let long_rust = "fn very_long_function_name(arg_one: String, arg_two: String, arg_three: String, arg_four: String) -> Result<String, Box<dyn std::error::Error>> { Ok(arg_one) }";

    let syntax_spans =
        highlight_code_to_styled_spans(long_rust, "rust").expect("rust highlighting");
    let spans = &syntax_spans[0];

    let lines = push_wrapped_diff_line_with_syntax_and_style_context(
        /*line_number*/ 1,
        DiffLineType::Insert,
        long_rust,
        /*width*/ 80,
        line_number_width(/*max_line_number*/ 1),
        spans,
        current_diff_render_style_context(),
    );

    assert!(
        lines.len() > 1,
        "syntax-highlighted long line should wrap to multiple lines, got {}",
        lines.len()
    );

    snapshot_lines(
        "syntax_highlighted_insert_wraps",
        lines,
        /*width*/ 90,
        /*height*/ 10,
    );
}

#[test]
fn ui_snapshot_syntax_highlighted_insert_wraps_text() {
    let long_rust = "fn very_long_function_name(arg_one: String, arg_two: String, arg_three: String, arg_four: String) -> Result<String, Box<dyn std::error::Error>> { Ok(arg_one) }";

    let syntax_spans =
        highlight_code_to_styled_spans(long_rust, "rust").expect("rust highlighting");
    let spans = &syntax_spans[0];

    let lines = push_wrapped_diff_line_with_syntax_and_style_context(
        /*line_number*/ 1,
        DiffLineType::Insert,
        long_rust,
        /*width*/ 80,
        line_number_width(/*max_line_number*/ 1),
        spans,
        current_diff_render_style_context(),
    );

    snapshot_lines_text("syntax_highlighted_insert_wraps_text", &lines);
}

#[test]
fn ui_snapshot_diff_gallery_80x24() {
    snapshot_diff_gallery("diff_gallery_80x24", /*width*/ 80, /*height*/ 24);
}

#[test]
fn ui_snapshot_diff_gallery_94x35() {
    snapshot_diff_gallery("diff_gallery_94x35", /*width*/ 94, /*height*/ 35);
}

#[test]
fn ui_snapshot_diff_gallery_120x40() {
    snapshot_diff_gallery(
        "diff_gallery_120x40",
        /*width*/ 120,
        /*height*/ 40,
    );
}

#[test]
fn ui_snapshot_ansi16_insert_delete_no_background() {
    let mut lines = push_wrapped_diff_line_inner_with_theme_and_color_level(
        /*line_number*/ 1,
        DiffLineType::Insert,
        "added in ansi16 mode",
        /*width*/ 80,
        line_number_width(/*max_line_number*/ 2),
        /*syntax_spans*/ None,
        DiffTheme::Dark,
        DiffColorLevel::Ansi16,
        fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi16),
    );
    lines.extend(push_wrapped_diff_line_inner_with_theme_and_color_level(
        /*line_number*/ 2,
        DiffLineType::Delete,
        "deleted in ansi16 mode",
        /*width*/ 80,
        line_number_width(/*max_line_number*/ 2),
        /*syntax_spans*/ None,
        DiffTheme::Dark,
        DiffColorLevel::Ansi16,
        fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi16),
    ));

    snapshot_lines(
        "ansi16_insert_delete_no_background",
        lines,
        /*width*/ 40,
        /*height*/ 4,
    );
}

#[test]
fn truecolor_dark_theme_uses_configured_backgrounds() {
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Insert,
            fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::TrueColor)
        ),
        Style::default().bg(rgb_color(DARK_TC_ADD_LINE_BG_RGB))
    );
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Delete,
            fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::TrueColor)
        ),
        Style::default().bg(rgb_color(DARK_TC_DEL_LINE_BG_RGB))
    );
    assert_eq!(
        style_gutter_for(
            DiffLineType::Insert,
            DiffTheme::Dark,
            DiffColorLevel::TrueColor
        ),
        style_gutter_dim()
    );
    assert_eq!(
        style_gutter_for(
            DiffLineType::Delete,
            DiffTheme::Dark,
            DiffColorLevel::TrueColor
        ),
        style_gutter_dim()
    );
}

#[test]
fn ansi256_dark_theme_uses_distinct_add_and_delete_backgrounds() {
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Insert,
            fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi256)
        ),
        Style::default().bg(indexed_color(DARK_256_ADD_LINE_BG_IDX))
    );
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Delete,
            fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi256)
        ),
        Style::default().bg(indexed_color(DARK_256_DEL_LINE_BG_IDX))
    );
    assert_ne!(
        style_line_bg_for(
            DiffLineType::Insert,
            fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi256)
        ),
        style_line_bg_for(
            DiffLineType::Delete,
            fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi256)
        ),
        "256-color mode should keep add/delete backgrounds distinct"
    );
}

#[test]
fn theme_scope_backgrounds_override_truecolor_fallback_when_available() {
    let backgrounds = resolve_diff_backgrounds_for(
        DiffTheme::Dark,
        DiffColorLevel::TrueColor,
        DiffScopeBackgroundRgbs {
            inserted: Some((1, 2, 3)),
            deleted: Some((4, 5, 6)),
        },
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, backgrounds),
        Style::default().bg(rgb_color((1, 2, 3)))
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, backgrounds),
        Style::default().bg(rgb_color((4, 5, 6)))
    );
}

#[test]
fn theme_scope_backgrounds_quantize_to_ansi256() {
    let backgrounds = resolve_diff_backgrounds_for(
        DiffTheme::Dark,
        DiffColorLevel::Ansi256,
        DiffScopeBackgroundRgbs {
            inserted: Some((0, 95, 0)),
            deleted: None,
        },
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, backgrounds),
        Style::default().bg(indexed_color(/*index*/ 22))
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, backgrounds),
        Style::default().bg(indexed_color(DARK_256_DEL_LINE_BG_IDX))
    );
}

#[test]
fn ui_snapshot_theme_scope_background_resolution() {
    let backgrounds = resolve_diff_backgrounds_for(
        DiffTheme::Dark,
        DiffColorLevel::TrueColor,
        DiffScopeBackgroundRgbs {
            inserted: Some((12, 34, 56)),
            deleted: None,
        },
    );
    let snapshot = format!(
        "insert={:?}\ndelete={:?}",
        style_line_bg_for(DiffLineType::Insert, backgrounds).bg,
        style_line_bg_for(DiffLineType::Delete, backgrounds).bg,
    );
    assert_snapshot!("theme_scope_background_resolution", snapshot);
}

#[test]
fn ansi16_disables_line_and_gutter_backgrounds() {
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Insert,
            fallback_diff_backgrounds(DiffTheme::Dark, DiffColorLevel::Ansi16)
        ),
        Style::default()
    );
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Delete,
            fallback_diff_backgrounds(DiffTheme::Light, DiffColorLevel::Ansi16)
        ),
        Style::default()
    );
    assert_eq!(
        style_gutter_for(
            DiffLineType::Insert,
            DiffTheme::Light,
            DiffColorLevel::Ansi16
        ),
        Style::default().fg(Color::Black)
    );
    assert_eq!(
        style_gutter_for(
            DiffLineType::Delete,
            DiffTheme::Light,
            DiffColorLevel::Ansi16
        ),
        Style::default().fg(Color::Black)
    );
    let themed_backgrounds = resolve_diff_backgrounds_for(
        DiffTheme::Light,
        DiffColorLevel::Ansi16,
        DiffScopeBackgroundRgbs {
            inserted: Some((8, 9, 10)),
            deleted: Some((11, 12, 13)),
        },
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Insert, themed_backgrounds),
        Style::default()
    );
    assert_eq!(
        style_line_bg_for(DiffLineType::Delete, themed_backgrounds),
        Style::default()
    );
}

#[test]
fn light_truecolor_theme_uses_readable_gutter_and_line_backgrounds() {
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Insert,
            fallback_diff_backgrounds(DiffTheme::Light, DiffColorLevel::TrueColor)
        ),
        Style::default().bg(rgb_color(LIGHT_TC_ADD_LINE_BG_RGB))
    );
    assert_eq!(
        style_line_bg_for(
            DiffLineType::Delete,
            fallback_diff_backgrounds(DiffTheme::Light, DiffColorLevel::TrueColor)
        ),
        Style::default().bg(rgb_color(LIGHT_TC_DEL_LINE_BG_RGB))
    );
    assert_eq!(
        style_gutter_for(
            DiffLineType::Insert,
            DiffTheme::Light,
            DiffColorLevel::TrueColor
        ),
        Style::default()
            .fg(rgb_color(LIGHT_TC_GUTTER_FG_RGB))
            .bg(rgb_color(LIGHT_TC_ADD_NUM_BG_RGB))
    );
    assert_eq!(
        style_gutter_for(
            DiffLineType::Delete,
            DiffTheme::Light,
            DiffColorLevel::TrueColor
        ),
        Style::default()
            .fg(rgb_color(LIGHT_TC_GUTTER_FG_RGB))
            .bg(rgb_color(LIGHT_TC_DEL_NUM_BG_RGB))
    );
}

#[test]
fn light_theme_wrapped_lines_keep_number_gutter_contrast() {
    let lines = push_wrapped_diff_line_inner_with_theme_and_color_level(
        /*line_number*/ 12,
        DiffLineType::Insert,
        "abcdefghij",
        /*width*/ 8,
        line_number_width(/*max_line_number*/ 12),
        /*syntax_spans*/ None,
        DiffTheme::Light,
        DiffColorLevel::TrueColor,
        fallback_diff_backgrounds(DiffTheme::Light, DiffColorLevel::TrueColor),
    );

    assert!(
        lines.len() > 1,
        "expected wrapped output for gutter style verification"
    );
    assert_eq!(
        lines[0].spans[0].style,
        Style::default()
            .fg(rgb_color(LIGHT_TC_GUTTER_FG_RGB))
            .bg(rgb_color(LIGHT_TC_ADD_NUM_BG_RGB))
    );
    assert_eq!(
        lines[1].spans[0].style,
        Style::default()
            .fg(rgb_color(LIGHT_TC_GUTTER_FG_RGB))
            .bg(rgb_color(LIGHT_TC_ADD_NUM_BG_RGB))
    );
    assert_eq!(lines[0].style.bg, Some(rgb_color(LIGHT_TC_ADD_LINE_BG_RGB)));
    assert_eq!(lines[1].style.bg, Some(rgb_color(LIGHT_TC_ADD_LINE_BG_RGB)));
}

#[test]
fn windows_terminal_promotes_ansi16_to_truecolor_for_diffs() {
    assert_eq!(
        diff_color_level_for_terminal(
            StdoutColorLevel::Ansi16,
            TerminalName::WindowsTerminal,
            /*has_wt_session*/ false,
            /*has_force_color_override*/ false,
        ),
        DiffColorLevel::TrueColor
    );
}

#[test]
fn wt_session_promotes_ansi16_to_truecolor_for_diffs() {
    assert_eq!(
        diff_color_level_for_terminal(
            StdoutColorLevel::Ansi16,
            TerminalName::Unknown,
            /*has_wt_session*/ true,
            /*has_force_color_override*/ false,
        ),
        DiffColorLevel::TrueColor
    );
}

#[test]
fn non_windows_terminal_keeps_ansi16_diff_palette() {
    assert_eq!(
        diff_color_level_for_terminal(
            StdoutColorLevel::Ansi16,
            TerminalName::WezTerm,
            /*has_wt_session*/ false,
            /*has_force_color_override*/ false,
        ),
        DiffColorLevel::Ansi16
    );
}

#[test]
fn wt_session_promotes_unknown_color_level_to_truecolor() {
    assert_eq!(
        diff_color_level_for_terminal(
            StdoutColorLevel::Unknown,
            TerminalName::WindowsTerminal,
            /*has_wt_session*/ true,
            /*has_force_color_override*/ false,
        ),
        DiffColorLevel::TrueColor
    );
}

#[test]
fn non_wt_windows_terminal_keeps_unknown_color_level_conservative() {
    assert_eq!(
        diff_color_level_for_terminal(
            StdoutColorLevel::Unknown,
            TerminalName::WindowsTerminal,
            /*has_wt_session*/ false,
            /*has_force_color_override*/ false,
        ),
        DiffColorLevel::Ansi16
    );
}

#[test]
fn explicit_force_override_keeps_ansi16_on_windows_terminal() {
    assert_eq!(
        diff_color_level_for_terminal(
            StdoutColorLevel::Ansi16,
            TerminalName::WindowsTerminal,
            /*has_wt_session*/ false,
            /*has_force_color_override*/ true,
        ),
        DiffColorLevel::Ansi16
    );
}

#[test]
fn explicit_force_override_keeps_ansi256_on_windows_terminal() {
    assert_eq!(
        diff_color_level_for_terminal(
            StdoutColorLevel::Ansi256,
            TerminalName::WindowsTerminal,
            /*has_wt_session*/ true,
            /*has_force_color_override*/ true,
        ),
        DiffColorLevel::Ansi256
    );
}

#[test]
fn add_diff_uses_path_extension_for_highlighting() {
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("highlight_add.rs"),
        FileChange::Add {
            content: "pub fn sum(a: i32, b: i32) -> i32 { a + b }\n".to_string(),
        },
    );

    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 80);
    let has_rgb = lines.iter().any(|line| {
        line.spans
            .iter()
            .any(|s| matches!(s.style.fg, Some(ratatui::style::Color::Rgb(..))))
    });
    assert!(
        has_rgb,
        "add diff for .rs file should produce syntax-highlighted (RGB) spans"
    );
}

#[test]
fn cpp_module_extensions_use_cpp_highlighting() {
    let highlighted_tokens = ["cpp", "cppm", "CPPM", "cxxm", "CxXm", "ixx", "IXX"]
        .into_iter()
        .map(|extension| {
            let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
            changes.insert(
                PathBuf::from(format!("math.{extension}")),
                FileChange::Add {
                    content:
                        "export module math;\nexport int sum(int a, int b) { return a + b; }\n"
                            .to_string(),
                },
            );

            let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 80);
            let rgb_tokens = lines
                .iter()
                .flat_map(|line| &line.spans)
                .filter(|span| matches!(span.style.fg, Some(ratatui::style::Color::Rgb(..))))
                .map(|span| span.content.to_string())
                .collect::<Vec<_>>();
            assert!(
                !rgb_tokens.is_empty(),
                "add diff for .{extension} file should produce syntax-highlighted (RGB) spans"
            );
            (extension, rgb_tokens.join("|"))
        })
        .collect::<Vec<_>>();

    assert_debug_snapshot!("cpp_module_extension_highlighting", highlighted_tokens);
}

#[test]
fn unknown_extension_falls_back_without_syntax_highlighting() {
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("math.unknown-extension"),
        FileChange::Add {
            content: "export module math;\nexport int value = 42;\n".to_string(),
        },
    );

    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 80);
    assert!(lines.iter().all(|line| {
        line.spans
            .iter()
            .all(|span| !matches!(span.style.fg, Some(ratatui::style::Color::Rgb(..))))
    }));
}

#[test]
fn delete_diff_uses_path_extension_for_highlighting() {
    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("highlight_delete.py"),
        FileChange::Delete {
            content: "def scale(x):\n    return x * 2\n".to_string(),
        },
    );

    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 80);
    let has_rgb = lines.iter().any(|line| {
        line.spans
            .iter()
            .any(|s| matches!(s.style.fg, Some(ratatui::style::Color::Rgb(..))))
    });
    assert!(
        has_rgb,
        "delete diff for .py file should produce syntax-highlighted (RGB) spans"
    );
}

#[test]
fn detect_lang_for_common_paths() {
    // 标准扩展名会被检测。
    assert!(detect_lang_for_path(Path::new("foo.rs")).is_some());
    assert!(detect_lang_for_path(Path::new("bar.py")).is_some());
    assert!(detect_lang_for_path(Path::new("app.tsx")).is_some());

    // 无扩展名的文件返回 None。
    assert!(detect_lang_for_path(Path::new("Makefile")).is_none());
    assert!(detect_lang_for_path(Path::new("randomfile")).is_none());
}

#[test]
fn wrap_styled_spans_single_line() {
    // 适合一行的内容应仅产生一个块。
    let spans = vec![RtSpan::raw("short")];
    let result = wrap_styled_spans(&spans, /*max_cols*/ 80);
    assert_eq!(result.len(), 1);
}

#[test]
fn wrap_styled_spans_splits_long_content() {
    // 宽度超过 max_cols 的内容应产生多个块。
    let long_text = "a".repeat(100);
    let spans = vec![RtSpan::raw(long_text)];
    let result = wrap_styled_spans(&spans, /*max_cols*/ 40);
    assert!(
        result.len() >= 3,
        "100 chars at 40 cols should produce at least 3 lines, got {}",
        result.len()
    );
}

#[test]
fn wrap_styled_spans_flushes_at_span_boundary() {
    // 当 span A 正好填满到 max_cols 且 span B 跟随时，该行
    // 必须在 B 开始之前刷新。否则 B 的第一个字符会
    // 落在已经满的行上，产生超宽输出。
    let style_a = Style::default().fg(Color::Red);
    let style_b = Style::default().fg(Color::Blue);
    let spans = vec![
        RtSpan::styled("aaaa", style_a), // 4 列，在 max_cols=4 时正好填满行
        RtSpan::styled("bb", style_b),   // 应在新行上开始
    ];
    let result = wrap_styled_spans(&spans, /*max_cols*/ 4);
    assert_eq!(
        result.len(),
        2,
        "span ending exactly at max_cols should flush before next span: {result:?}"
    );
    // 第一行应仅包含 'a' span。
    let first_width: usize = result[0].iter().map(|s| s.content.chars().count()).sum();
    assert!(
        first_width <= 4,
        "first line should be at most 4 cols wide, got {first_width}"
    );
}

#[test]
fn wrap_styled_spans_preserves_styles() {
    // 验证样式在分割边界处得到保留。
    let style = Style::default().fg(Color::Green);
    let text = "x".repeat(50);
    let spans = vec![RtSpan::styled(text, style)];
    let result = wrap_styled_spans(&spans, /*max_cols*/ 20);
    for chunk in &result {
        for span in chunk {
            assert_eq!(span.style, style, "style should be preserved across wraps");
        }
    }
}

#[test]
fn wrap_styled_spans_tabs_have_visible_width() {
    // 制表符应计为 TAB_WIDTH 列，而不是零。
    // 在 max_cols=8 时，一个制表符（4 列） + "abcde"（5 列）= 9 列 → 必须换行。
    let style = Style::default().fg(Color::Green);
    let spans = vec![RtSpan::styled("\tabcde", style)];
    let result = wrap_styled_spans(&spans, /*max_cols*/ 8);
    assert_eq!(
        result,
        vec![
            vec![RtSpan::styled("    abcd", style)],
            vec![RtSpan::styled("e", style)],
        ]
    );
}

#[test]
fn wrap_styled_spans_wraps_before_first_overflowing_char() {
    let spans = vec![RtSpan::raw("abcd\t界")];
    let result = wrap_styled_spans(&spans, /*max_cols*/ 5);

    let line_text: Vec<String> = result
        .iter()
        .map(|line| {
            line.iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();
    assert_eq!(line_text, vec!["abcd", "    ", "界"]);

    let line_width = |line: &[RtSpan<'static>]| -> usize {
        line.iter()
            .flat_map(|span| span.content.chars())
            .map(|ch| ch.width().unwrap_or(if ch == '\t' { TAB_WIDTH } else { 0 }))
            .sum()
    };
    for line in &result {
        assert!(
            line_width(line) <= 5,
            "wrapped line exceeded width 5: {line:?}"
        );
    }
}

#[test]
fn fallback_wrapping_uses_display_width_for_tabs_and_wide_chars() {
    let width = 8;
    let lines = push_wrapped_diff_line_with_style_context(
        /*line_number*/ 1,
        DiffLineType::Insert,
        "abcd\t界🙂",
        width,
        line_number_width(/*max_line_number*/ 1),
        current_diff_render_style_context(),
    );

    assert!(lines.len() >= 2, "expected wrapped output, got {lines:?}");
    for line in &lines {
        assert!(
            line_display_width(line) <= width,
            "fallback wrapped line exceeded width {width}: {line:?}"
        );
    }
}

#[test]
fn large_update_diff_skips_highlighting() {
    // 构建一个足够大的 patch 以超过 MAX_HIGHLIGHT_LINES (10_000)。
    // 如果没有预检查，这里会尝试 10k+ 次解析器初始化。
    let line_count = 10_500;
    let original: String = (0..line_count).map(|i| format!("line {i}\n")).collect();
    let modified: String = (0..line_count)
        .map(|i| {
            if i % 2 == 0 {
                format!("line {i} changed\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect();
    let patch = diffy::create_patch(&original, &modified).to_string();

    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("huge.rs"),
        FileChange::Update {
            unified_diff: patch,
            move_path: None,
        },
    );

    // 应快速完成（无逐行解析器初始化）。如果绕过保护措施
    // 这会非常慢。
    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 80);

    // diff 渲染未超时——保护措施阻止了
    // 数千次逐行解析器初始化。验证我们确实
    // 获得了输出（patch 非空）。
    assert!(
        lines.len() > 100,
        "expected many output lines from large diff, got {}",
        lines.len(),
    );

    // 不应有 span 包含 RGB 前景颜色（语法主题
    // 产生 RGB；纯 diff 样式仅使用命名的 Color 变体）。
    for line in &lines {
        for span in &line.spans {
            if let Some(ratatui::style::Color::Rgb(..)) = span.style.fg {
                panic!(
                    "large diff should not have syntax-highlighted spans, \
                         got RGB color in style {:?} for {:?}",
                    span.style, span.content,
                );
            }
        }
    }
}

#[test]
fn rename_diff_uses_destination_extension_for_highlighting() {
    // 从未知扩展名到 .rs 的重命名应高亮为 Rust。
    // 没有此修复，detect_lang_for_path 使用源路径（.xyzzy），
    // 该路径没有语法定义，因此跳过高亮。
    let original = "fn main() {}\n";
    let modified = "fn main() { println!(\"hi\"); }\n";
    let patch = diffy::create_patch(original, modified).to_string();

    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("foo.xyzzy"),
        FileChange::Update {
            unified_diff: patch,
            move_path: Some(PathBuf::from("foo.rs")),
        },
    );

    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 80);
    let has_rgb = lines.iter().any(|line| {
        line.spans
            .iter()
            .any(|s| matches!(s.style.fg, Some(ratatui::style::Color::Rgb(..))))
    });
    assert!(
        has_rgb,
        "rename from .xyzzy to .rs should produce syntax-highlighted (RGB) spans"
    );
}

#[test]
fn update_diff_preserves_multiline_highlight_state_within_hunk() {
    let original = "fn demo() {\n    let s = \"hello\";\n}\n";
    let modified = "fn demo() {\n    let s = \"hello\nworld\";\n}\n";
    let patch = diffy::create_patch(original, modified).to_string();

    let mut changes: HashMap<PathBuf, FileChange> = HashMap::new();
    changes.insert(
        PathBuf::from("demo.rs"),
        FileChange::Update {
            unified_diff: patch,
            move_path: None,
        },
    );

    let expected_multiline =
        highlight_code_to_styled_spans("    let s = \"hello\nworld\";\n", "rust")
            .expect("rust highlighting");
    let expected_style = expected_multiline
        .get(1)
        .and_then(|line| {
            line.iter()
                .find(|span| span.content.as_ref().contains("world"))
        })
        .map(|span| span.style)
        .expect("expected highlighted span for second multiline string line");

    let lines = create_diff_summary(&changes, &PathBuf::from("/"), /*wrap_cols*/ 120);
    let actual_style = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.as_ref().contains("world"))
        .map(|span| span.style)
        .expect("expected rendered diff span containing 'world'");

    assert_eq!(actual_style, expected_style);
}
