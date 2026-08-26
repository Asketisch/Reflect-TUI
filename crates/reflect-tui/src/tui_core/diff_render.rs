//! 渲染带有行号、gutter 符号和可选语法高亮的统一 diff。
//!
//! 每个 `FileChange` 变体（Add / Delete / Update）被渲染为 diff 行的块，
//! 每行以前置右对齐的行号、gutter 符号（`+` / `-` / ` `）和内容文本为前缀。
//! 当存在可识别的文件扩展名时，内容文本使用
//! [`crate::tui_core::render::highlight`] 进行语法高亮。
//!
//! **主题感知样式：** diff 背景根据终端背景亮度通过 [`DiffTheme`] 自适应。
//! 深色终端获得柔和色调（`#212922` 绿色，`#3C170F` 红色）；浅色终端获得 GitHub 风格柔和色，
//! 具有不同的 gutter 背景以形成对比。渲染器使用固定的
//! 调色板（truecolor / 256-color / 16-color）终端，因此增/删行
//! 即使在量化为有限调色板时仍保持视觉差异。
//!
//! **语法主题作用域背景：** 当活动语法主题为 `markup.inserted` / `markup.deleted`（或回退
//! `diff.inserted` / `diff.deleted`）作用域定义了背景颜色时，这些颜色会覆盖
//! 硬编码调色板（仅适用于高级别颜色）。ANSI-16 模式始终使用
//! 仅前景样式，无论主题作用域背景如何。
//!
//! **`Update` diff 的高亮策略：** 渲染器将每个 hunk 作为单个拼接块进行高亮，
//! 而不是逐行。这会保持 syntect 解析器状态在 hunk 内连续行之间
//! （对多行字符串、块注释等很重要）。跨 hunk 状态故意*不*保持，因为 hunk 在视觉上被分隔，
//! 并且在上下文边界处重新同步。
//!
//! **换行：** 长行在可用列宽度处硬换行。
//! 语法高亮片段在字符边界处分割，样式
//! 在分割处保持，因此不会丢失颜色信息。

use diffy::Hunk;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line as RtLine;
use ratatui::text::Span as RtSpan;
use ratatui::widgets::Paragraph;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use crate::utils_absolute_path::AbsolutePathBuf;
use unicode_width::UnicodeWidthChar;

/// 渲染 diff 内容中制表符的替换字符串。
const TAB_REPLACEMENT: &str = "    ";
/// 制表符在列中的显示宽度。
const TAB_WIDTH: usize = TAB_REPLACEMENT.len();

// ── 主题/调色板与样式子系统（外移子模块，降低本文件行数） ──
mod styles;
mod theme;
// `pub(crate) use` 再导出，保持对外路径 `crate::tui_core::diff_render::Foo` 不变
//（theme_picker 等外部模块按此路径引用 DiffLineType / current_diff_render_style_context）。
use styles::*;
pub(crate) use theme::*;

use crate::git_utils::get_git_repo_root;
use crate::terminal_detection::TerminalName;
use crate::terminal_detection::terminal_info;
use crate::tui_core::color::is_light;
use crate::tui_core::color::perceptual_distance;
use crate::tui_core::diff_model::FileChange;
use crate::tui_core::exec_command::relativize_to_home;
use crate::tui_core::render::Insets;
use crate::tui_core::render::highlight::DiffScopeBackgroundRgbs;
use crate::tui_core::render::highlight::diff_scope_background_rgbs;
use crate::tui_core::render::highlight::exceeds_highlight_limits;
use crate::tui_core::render::highlight::highlight_code_to_styled_spans;
use crate::tui_core::render::line_utils::prefix_lines;
use crate::tui_core::render::renderable::ColumnRenderable;
use crate::tui_core::render::renderable::InsetRenderable;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::terminal_palette::StdoutColorLevel;
use crate::tui_core::terminal_palette::XTERM_COLORS;
use crate::tui_core::terminal_palette::default_bg;
use crate::tui_core::terminal_palette::indexed_color;
use crate::tui_core::terminal_palette::rgb_color;
use crate::tui_core::terminal_palette::stdout_color_level;

pub struct DiffSummary {
    changes: HashMap<PathBuf, FileChange>,
    cwd: AbsolutePathBuf,
}

impl DiffSummary {
    pub(crate) fn new(changes: HashMap<PathBuf, FileChange>, cwd: AbsolutePathBuf) -> Self {
        Self { changes, cwd }
    }
}

impl Renderable for FileChange {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut lines = vec![];
        render_change(self, &mut lines, area.width as usize, /*lang*/ None);
        Paragraph::new(lines).render(area, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let mut lines = vec![];
        render_change(self, &mut lines, width as usize, /*lang*/ None);
        lines.len() as u16
    }
}

impl From<DiffSummary> for Box<dyn Renderable> {
    fn from(val: DiffSummary) -> Self {
        let mut rows: Vec<Box<dyn Renderable>> = vec![];
        let mut changes: Vec<_> = val.changes.into_iter().collect();
        changes.sort_by(|left, right| left.0.cmp(&right.0));

        for (i, (path, change)) in changes.into_iter().enumerate() {
            if i > 0 {
                rows.push(Box::new(RtLine::from("")));
            }
            let (added, removed) = line_counts(&change);
            let mut path = RtLine::from(display_path_for(&path, val.cwd.as_path()));
            path.push_span(" ");
            path.extend(render_line_count_summary(added, removed));
            rows.push(Box::new(path));
            rows.push(Box::new(RtLine::from("")));
            rows.push(Box::new(InsetRenderable::new(
                Box::new(change) as Box<dyn Renderable>,
                Insets::tlbr(
                    /*top*/ 0, /*left*/ 2, /*bottom*/ 0, /*right*/ 0,
                ),
            )));
        }

        Box::new(ColumnRenderable::with(rows))
    }
}

pub(crate) fn create_diff_summary(
    changes: &HashMap<PathBuf, FileChange>,
    cwd: &Path,
    wrap_cols: usize,
) -> Vec<RtLine<'static>> {
    let rows = collect_rows(changes);
    render_changes_block(rows, wrap_cols, cwd)
}

/// 用于每个文件展示的共享行结构。
struct Row<'a> {
    path: &'a Path,
    move_path: Option<&'a Path>,
    added: usize,
    removed: usize,
    change: &'a FileChange,
}

fn collect_rows(changes: &HashMap<PathBuf, FileChange>) -> Vec<Row<'_>> {
    let mut rows = Vec::with_capacity(changes.len());
    for (path, change) in changes.iter() {
        let (added, removed) = line_counts(change);
        let move_path = match change {
            FileChange::Update {
                move_path: Some(new),
                ..
            } => Some(new.as_path()),
            _ => None,
        };
        rows.push(Row {
            path: path.as_path(),
            move_path,
            added,
            removed,
            change,
        });
    }
    rows.sort_by(|left, right| left.path.cmp(right.path));
    rows
}

fn line_counts(change: &FileChange) -> (usize, usize) {
    match change {
        FileChange::Add { content } => (content.lines().count(), 0),
        FileChange::Delete { content } => (0, content.lines().count()),
        FileChange::Update { unified_diff, .. } => calculate_add_remove_from_diff(unified_diff),
    }
}

fn render_line_count_summary(added: usize, removed: usize) -> Vec<RtSpan<'static>> {
    let mut spans = Vec::new();
    spans.push("(".into());
    spans.push(format!("+{added}").green());
    spans.push(" ".into());
    spans.push(format!("-{removed}").red());
    spans.push(")".into());
    spans
}

fn render_changes_block(rows: Vec<Row<'_>>, wrap_cols: usize, cwd: &Path) -> Vec<RtLine<'static>> {
    let mut out: Vec<RtLine<'static>> = Vec::new();

    let render_path = |row: &Row<'_>| -> Vec<RtSpan<'static>> {
        let mut spans = Vec::new();
        spans.push(display_path_for(row.path, cwd).into());
        if let Some(move_path) = row.move_path {
            spans.push(format!(" → {}", display_path_for(move_path, cwd)).into());
        }
        spans
    };

    // 标题
    let total_added: usize = rows.iter().map(|r| r.added).sum();
    let total_removed: usize = rows.iter().map(|r| r.removed).sum();
    let file_count = rows.len();
    let noun = if file_count == 1 { "file" } else { "files" };
    let mut header_spans: Vec<RtSpan<'static>> = vec!["• ".dim()];
    if let [row] = &rows[..] {
        let verb = match row.change {
            FileChange::Add { .. } => "Added",
            FileChange::Delete { .. } => "Deleted",
            _ => "Edited",
        };
        header_spans.push(verb.bold());
        header_spans.push(" ".into());
        header_spans.extend(render_path(row));
        header_spans.push(" ".into());
        header_spans.extend(render_line_count_summary(row.added, row.removed));
    } else {
        header_spans.push("Edited".bold());
        header_spans.push(format!(" {file_count} {noun} ").into());
        header_spans.extend(render_line_count_summary(total_added, total_removed));
    }
    out.push(RtLine::from(header_spans));

    for (idx, r) in rows.into_iter().enumerate() {
        // 在文件块之间插入空行分隔（除了第一个之前）
        if idx > 0 {
            out.push("".into());
        }
        // 文件标题行（当单文件标题已经显示文件名时跳过）
        let skip_file_header = file_count == 1;
        if !skip_file_header {
            let mut header: Vec<RtSpan<'static>> = Vec::new();
            header.push("  └ ".dim());
            header.extend(render_path(&r));
            header.push(" ".into());
            header.extend(render_line_count_summary(r.added, r.removed));
            out.push(RtLine::from(header));
        }

        // 对于重命名，使用目标扩展名进行高亮——因为
        // diff 内容反映的是新文件，而不是旧文件。
        let lang_path = r.move_path.unwrap_or(r.path);
        let lang = detect_lang_for_path(lang_path);
        let mut lines = vec![];
        render_change(r.change, &mut lines, wrap_cols - 4, lang.as_deref());
        out.extend(prefix_lines(lines, "    ".into(), "    ".into()));
    }

    out
}

/// 检测编程语言的函数，用于通过文件路径扩展名识别语言。
/// 返回原始扩展名字符串供 `normalize_lang` / `find_syntax`
/// 在下游解析。
fn detect_lang_for_path(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    Some(ext.to_string())
}

fn render_change(
    change: &FileChange,
    out: &mut Vec<RtLine<'static>>,
    width: usize,
    lang: Option<&str>,
) {
    let style_context = current_diff_render_style_context();
    match change {
        FileChange::Add { content } => {
            // 整体预高亮整个文件内容。
            let syntax_lines = lang.and_then(|l| highlight_code_to_styled_spans(content, l));
            let line_number_width = line_number_width(content.lines().count());
            for (i, raw) in content.lines().enumerate() {
                let syn = syntax_lines.as_ref().and_then(|sl| sl.get(i));
                if let Some(spans) = syn {
                    out.extend(push_wrapped_diff_line_inner_with_theme_and_color_level(
                        i + 1,
                        DiffLineType::Insert,
                        raw,
                        width,
                        line_number_width,
                        Some(spans),
                        style_context.theme,
                        style_context.color_level,
                        style_context.diff_backgrounds,
                    ));
                } else {
                    out.extend(push_wrapped_diff_line_inner_with_theme_and_color_level(
                        i + 1,
                        DiffLineType::Insert,
                        raw,
                        width,
                        line_number_width,
                        /*syntax_spans*/ None,
                        style_context.theme,
                        style_context.color_level,
                        style_context.diff_backgrounds,
                    ));
                }
            }
        }
        FileChange::Delete { content } => {
            let syntax_lines = lang.and_then(|l| highlight_code_to_styled_spans(content, l));
            let line_number_width = line_number_width(content.lines().count());
            for (i, raw) in content.lines().enumerate() {
                let syn = syntax_lines.as_ref().and_then(|sl| sl.get(i));
                if let Some(spans) = syn {
                    out.extend(push_wrapped_diff_line_inner_with_theme_and_color_level(
                        i + 1,
                        DiffLineType::Delete,
                        raw,
                        width,
                        line_number_width,
                        Some(spans),
                        style_context.theme,
                        style_context.color_level,
                        style_context.diff_backgrounds,
                    ));
                } else {
                    out.extend(push_wrapped_diff_line_inner_with_theme_and_color_level(
                        i + 1,
                        DiffLineType::Delete,
                        raw,
                        width,
                        line_number_width,
                        /*syntax_spans*/ None,
                        style_context.theme,
                        style_context.color_level,
                        style_context.diff_backgrounds,
                    ));
                }
            }
        }
        FileChange::Update { unified_diff, .. } => {
            if let Ok(patch) = diffy::Patch::from_str(unified_diff) {
                let mut max_line_number = 0;
                let mut total_diff_bytes: usize = 0;
                let mut total_diff_lines: usize = 0;
                for h in patch.hunks() {
                    let mut old_ln = h.old_range().start();
                    let mut new_ln = h.new_range().start();
                    for l in h.lines() {
                        let text = match l {
                            diffy::Line::Insert(t)
                            | diffy::Line::Delete(t)
                            | diffy::Line::Context(t) => t,
                        };
                        total_diff_bytes += text.len();
                        total_diff_lines += 1;
                        match l {
                            diffy::Line::Insert(_) => {
                                max_line_number = max_line_number.max(new_ln);
                                new_ln += 1;
                            }
                            diffy::Line::Delete(_) => {
                                max_line_number = max_line_number.max(old_ln);
                                old_ln += 1;
                            }
                            diffy::Line::Context(_) => {
                                max_line_number = max_line_number.max(new_ln);
                                old_ln += 1;
                                new_ln += 1;
                            }
                        }
                    }
                }

                // 当 patch 过大时，跳过逐行语法高亮——避免
                // 在大型 diff 上进行数千次解析器初始化
                // 导致渲染卡顿。
                let diff_lang = if exceeds_highlight_limits(total_diff_bytes, total_diff_lines) {
                    None
                } else {
                    lang
                };

                let line_number_width = line_number_width(max_line_number);
                let mut is_first_hunk = true;
                for h in patch.hunks() {
                    if !is_first_hunk {
                        let spacer = format!("{:width$} ", "", width = line_number_width.max(1));
                        let spacer_span = RtSpan::styled(
                            spacer,
                            style_gutter_for(
                                DiffLineType::Context,
                                style_context.theme,
                                style_context.color_level,
                            ),
                        );
                        out.push(RtLine::from(vec![spacer_span, "⋮".dim()]));
                    }
                    is_first_hunk = false;

                    // 将每个 hunk 作为单个块进行高亮，以便 syntect 解析器
                    // 状态在连续行之间保持。
                    let hunk_syntax_lines = diff_lang.and_then(|language| {
                        let hunk_text: String = h
                            .lines()
                            .iter()
                            .map(|line| match line {
                                diffy::Line::Insert(text)
                                | diffy::Line::Delete(text)
                                | diffy::Line::Context(text) => *text,
                            })
                            .collect();
                        let syntax_lines = highlight_code_to_styled_spans(&hunk_text, language)?;
                        (syntax_lines.len() == h.lines().len()).then_some(syntax_lines)
                    });

                    let mut old_ln = h.old_range().start();
                    let mut new_ln = h.new_range().start();
                    for (line_idx, l) in h.lines().iter().enumerate() {
                        let syntax_spans = hunk_syntax_lines
                            .as_ref()
                            .and_then(|syntax_lines| syntax_lines.get(line_idx));
                        match l {
                            diffy::Line::Insert(text) => {
                                let s = text.trim_end_matches('\n');
                                if let Some(syn) = syntax_spans {
                                    out.extend(
                                        push_wrapped_diff_line_inner_with_theme_and_color_level(
                                            new_ln,
                                            DiffLineType::Insert,
                                            s,
                                            width,
                                            line_number_width,
                                            Some(syn),
                                            style_context.theme,
                                            style_context.color_level,
                                            style_context.diff_backgrounds,
                                        ),
                                    );
                                } else {
                                    out.extend(
                                        push_wrapped_diff_line_inner_with_theme_and_color_level(
                                            new_ln,
                                            DiffLineType::Insert,
                                            s,
                                            width,
                                            line_number_width,
                                            /*syntax_spans*/ None,
                                            style_context.theme,
                                            style_context.color_level,
                                            style_context.diff_backgrounds,
                                        ),
                                    );
                                }
                                new_ln += 1;
                            }
                            diffy::Line::Delete(text) => {
                                let s = text.trim_end_matches('\n');
                                if let Some(syn) = syntax_spans {
                                    out.extend(
                                        push_wrapped_diff_line_inner_with_theme_and_color_level(
                                            old_ln,
                                            DiffLineType::Delete,
                                            s,
                                            width,
                                            line_number_width,
                                            Some(syn),
                                            style_context.theme,
                                            style_context.color_level,
                                            style_context.diff_backgrounds,
                                        ),
                                    );
                                } else {
                                    out.extend(
                                        push_wrapped_diff_line_inner_with_theme_and_color_level(
                                            old_ln,
                                            DiffLineType::Delete,
                                            s,
                                            width,
                                            line_number_width,
                                            /*syntax_spans*/ None,
                                            style_context.theme,
                                            style_context.color_level,
                                            style_context.diff_backgrounds,
                                        ),
                                    );
                                }
                                old_ln += 1;
                            }
                            diffy::Line::Context(text) => {
                                let s = text.trim_end_matches('\n');
                                if let Some(syn) = syntax_spans {
                                    out.extend(
                                        push_wrapped_diff_line_inner_with_theme_and_color_level(
                                            new_ln,
                                            DiffLineType::Context,
                                            s,
                                            width,
                                            line_number_width,
                                            Some(syn),
                                            style_context.theme,
                                            style_context.color_level,
                                            style_context.diff_backgrounds,
                                        ),
                                    );
                                } else {
                                    out.extend(
                                        push_wrapped_diff_line_inner_with_theme_and_color_level(
                                            new_ln,
                                            DiffLineType::Context,
                                            s,
                                            width,
                                            line_number_width,
                                            /*syntax_spans*/ None,
                                            style_context.theme,
                                            style_context.color_level,
                                            style_context.diff_backgrounds,
                                        ),
                                    );
                                }
                                old_ln += 1;
                                new_ln += 1;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 格式化路径以供显示，尽可能相对于当前工作目录，
/// 在 jj/no-`.git` 工作空间中保持输出稳定（例如图像
/// 工具调用应显示 `example.png` 而不是绝对路径）。
pub(crate) fn display_path_for(path: &Path, cwd: &Path) -> String {
    if path.is_relative() {
        return path.display().to_string();
    }

    if let Ok(stripped) = path.strip_prefix(cwd) {
        return stripped.display().to_string();
    }

    let path_in_same_repo = match (get_git_repo_root(cwd), get_git_repo_root(path)) {
        (Some(cwd_repo), Some(path_repo)) => cwd_repo == path_repo,
        _ => false,
    };
    let chosen = if path_in_same_repo {
        pathdiff::diff_paths(path, cwd).unwrap_or_else(|| path.to_path_buf())
    } else {
        relativize_to_home(path)
            .map(|p| PathBuf::from_iter([Path::new("~"), p.as_path()]))
            .unwrap_or_else(|| path.to_path_buf())
    };
    chosen.display().to_string()
}

pub(crate) fn calculate_add_remove_from_diff(diff: &str) -> (usize, usize) {
    if let Ok(patch) = diffy::Patch::from_str(diff) {
        patch
            .hunks()
            .iter()
            .flat_map(Hunk::lines)
            .fold((0, 0), |(a, d), l| match l {
                diffy::Line::Insert(_) => (a + 1, d),
                diffy::Line::Delete(_) => (a, d + 1),
                diffy::Line::Context(_) => (a, d),
            })
    } else {
        // 对于无法解析的 diff，两个计数都返回 0。
        (0, 0)
    }
}

/// 渲染单个纯文本（非语法高亮）diff 行，换行到
/// `width` 列，使用预计算的 [`DiffRenderStyleContext`]。
///
/// 这是便利入口点，用于主题选择器预览和
/// 任何没有语法 spans 的调用者。委托给内部
/// 渲染核心，`syntax_spans = None`。
pub(crate) fn push_wrapped_diff_line_with_style_context(
    line_number: usize,
    kind: DiffLineType,
    text: &str,
    width: usize,
    line_number_width: usize,
    style_context: DiffRenderStyleContext,
) -> Vec<RtLine<'static>> {
    push_wrapped_diff_line_inner_with_theme_and_color_level(
        line_number,
        kind,
        text,
        width,
        line_number_width,
        /*syntax_spans*/ None,
        style_context.theme,
        style_context.color_level,
        style_context.diff_backgrounds,
    )
}

/// 渲染语法高亮的 diff 行，换行到 `width` 列，使用
/// 预计算的 [`DiffRenderStyleContext`]。
///
/// 与 [`push_wrapped_diff_line_with_style_context`] 类似，但在 diff
/// 着色上叠加 `syntax_spans`（来自 [`highlight_code_to_styled_spans`]）。
/// 删除行接收 `DIM` 修饰符，因此语法颜色不会
/// 压倒删除提示。
pub(crate) fn push_wrapped_diff_line_with_syntax_and_style_context(
    line_number: usize,
    kind: DiffLineType,
    text: &str,
    width: usize,
    line_number_width: usize,
    syntax_spans: &[RtSpan<'static>],
    style_context: DiffRenderStyleContext,
) -> Vec<RtLine<'static>> {
    push_wrapped_diff_line_inner_with_theme_and_color_level(
        line_number,
        kind,
        text,
        width,
        line_number_width,
        Some(syntax_spans),
        style_context.theme,
        style_context.color_level,
        style_context.diff_backgrounds,
    )
}

// ── 换行辅助子系统（外移子模块） ──
mod wrap;
use wrap::*;

pub(crate) fn line_number_width(max_line_number: usize) -> usize {
    if max_line_number == 0 {
        1
    } else {
        max_line_number.to_string().len()
    }
}

#[cfg(test)]
mod tests;
