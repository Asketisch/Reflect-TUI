//! 为 TUI 构建 `/theme` 选择器对话框。
//!
//! 该选择器列出所有捆绑主题以及在
//! `{REFLECT_HOME}/themes/` 下找到的自定义 `.tmTheme` 文件。它提供：
//!
//! - **实时预览**：`on_selection_changed` 回调在用户导航时
//!   切换运行时语法主题，在预览面板和任何可见的
//!   代码块中提供即时视觉反馈。
//! - **取消恢复**：关闭时（Esc / Ctrl+C）`on_cancel` 回调
//!   恢复选择器打开时截取的主题快照。
//! - **确认持久化**：`AppEvent::SyntaxThemeSelected` 动作
//!   通过 `ConfigEditsBuilder` 将 `[tui] theme = "..."` 持久化到 `config.toml`。
//!
//! 两个预览 renderable 适配终端宽度：
//!
//! - `ThemePreviewWideRenderable` —— 垂直居中、缩进 2 列，
//!   当终端足够宽以支持并排布局时显示在侧面板
//!   （>= 44 列侧面板且 >= 40 列列表）。
//! - `ThemePreviewNarrowRenderable` —— 当并排布局放不下时，
//!   堆叠在列表下方的紧凑 4 行片段。

use std::path::Path;

use crate::tui_core::app_event::AppEvent;
use crate::tui_core::bottom_pane::SelectionItem;
use crate::tui_core::bottom_pane::SelectionViewParams;
use crate::tui_core::bottom_pane::SideContentWidth;
use crate::tui_core::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::tui_core::bottom_pane::popup_content_width;
use crate::tui_core::bottom_pane::side_by_side_layout_widths;
use crate::tui_core::diff_render::DiffLineType;
use crate::tui_core::diff_render::current_diff_render_style_context;
use crate::tui_core::diff_render::line_number_width;
use crate::tui_core::diff_render::push_wrapped_diff_line_with_style_context;
use crate::tui_core::diff_render::push_wrapped_diff_line_with_syntax_and_style_context;
use crate::tui_core::render::highlight;
use crate::tui_core::render::renderable::Renderable;
use crate::tui_core::status::format_directory_display;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreviewDiffKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PreviewRow {
    line_no: usize,
    kind: PreviewDiffKind,
    code: &'static str,
}

/// 堆叠（窄）模式中使用的紧凑回退预览。
/// 始终保持恰好一行移除和一行添加可见。
const NARROW_PREVIEW_ROWS: [PreviewRow; 4] = [
    PreviewRow {
        line_no: 12,
        kind: PreviewDiffKind::Context,
        code: "fn greet(name: &str) -> String {",
    },
    PreviewRow {
        line_no: 13,
        kind: PreviewDiffKind::Removed,
        code: "    format!(\"Hello, {}!\", name)",
    },
    PreviewRow {
        line_no: 13,
        kind: PreviewDiffKind::Added,
        code: "    format!(\"Hello, {name}!\")",
    },
    PreviewRow {
        line_no: 14,
        kind: PreviewDiffKind::Context,
        code: "}",
    },
];

/// 并排模式中使用的更宽 diff 预览。
/// 此示例刻意混合了上下文、添加和移除。
const WIDE_PREVIEW_ROWS: [PreviewRow; 8] = [
    PreviewRow {
        line_no: 31,
        kind: PreviewDiffKind::Context,
        code: "fn summarize(users: &[User]) -> String {",
    },
    PreviewRow {
        line_no: 32,
        kind: PreviewDiffKind::Removed,
        code: "    let active = users.iter().filter(|u| u.is_active).count();",
    },
    PreviewRow {
        line_no: 32,
        kind: PreviewDiffKind::Added,
        code: "    let active = users.iter().filter(|u| u.is_active()).count();",
    },
    PreviewRow {
        line_no: 33,
        kind: PreviewDiffKind::Context,
        code: "    let names: Vec<&str> = users.iter().map(User::name).take(3).collect();",
    },
    PreviewRow {
        line_no: 34,
        kind: PreviewDiffKind::Removed,
        code: "    format!(\"{} active: {}\", active, names.join(\", \"))",
    },
    PreviewRow {
        line_no: 34,
        kind: PreviewDiffKind::Added,
        code: "    format!(\"{active} active users: {}\", names.join(\", \"))",
    },
    PreviewRow {
        line_no: 35,
        kind: PreviewDiffKind::Added,
        code: "        .trim()",
    },
    PreviewRow {
        line_no: 36,
        kind: PreviewDiffKind::Context,
        code: "}",
    },
];

/// 并排主题预览的最小侧面板宽度。
const WIDE_PREVIEW_MIN_WIDTH: u16 = 44;

/// 宽预览内容使用的左缩进。
const WIDE_PREVIEW_LEFT_INSET: u16 = 2;

/// 垂直居中宽预览使用的最小框架内边距。
const PREVIEW_FRAME_PADDING: u16 = 1;

const PREVIEW_FALLBACK_SUBTITLE: &str = "Move up/down to live preview themes";

/// 并排预览：语法高亮的 Rust diff 片段，垂直居中并带 2 列左侧内缩。
/// 填满整个侧边面板高度。
struct ThemePreviewWideRenderable;

/// 堆叠预览：当终端宽度不足以容纳并排布局时，在列表下方
/// 显示紧凑的 4 行 Rust diff 片段。
struct ThemePreviewNarrowRenderable;

fn preview_diff_line_type(kind: PreviewDiffKind) -> DiffLineType {
    match kind {
        PreviewDiffKind::Context => DiffLineType::Context,
        PreviewDiffKind::Added => DiffLineType::Insert,
        PreviewDiffKind::Removed => DiffLineType::Delete,
    }
}

fn centered_offset(available: u16, content: u16, min_frame: u16) -> u16 {
    let free = available.saturating_sub(content);
    let frame = if free >= min_frame.saturating_mul(2) {
        min_frame
    } else {
        0
    };
    frame + free.saturating_sub(frame.saturating_mul(2)) / 2
}

fn render_preview(
    area: Rect,
    buf: &mut Buffer,
    preview_rows: &[PreviewRow],
    center_vertically: bool,
    left_inset: u16,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    if preview_rows.is_empty() {
        return;
    }
    let preview_code = preview_rows
        .iter()
        .map(|row| row.code)
        .collect::<Vec<_>>()
        .join("\n");
    let syntax_lines = highlight::highlight_code_to_styled_spans(&preview_code, "rust");

    let max_line_no = preview_rows
        .iter()
        .map(|row| row.line_no)
        .max()
        .unwrap_or(1);
    let ln_width = line_number_width(max_line_no);

    let content_height = (preview_rows.len() as u16).min(area.height);

    let left_pad = left_inset.min(area.width.saturating_sub(1));
    let top_pad = if center_vertically {
        centered_offset(area.height, content_height, PREVIEW_FRAME_PADDING)
    } else {
        0
    };

    let render_width = area.width.saturating_sub(left_pad);
    let style_context = current_diff_render_style_context();
    for (y, (idx, row)) in (area.y.saturating_add(top_pad)..).zip(preview_rows.iter().enumerate()) {
        if y >= area.y + area.height {
            break;
        }
        let diff_type = preview_diff_line_type(row.kind);
        let wrapped = if let Some(syn) = syntax_lines.as_ref().and_then(|sl| sl.get(idx)) {
            push_wrapped_diff_line_with_syntax_and_style_context(
                row.line_no,
                diff_type,
                row.code,
                render_width as usize,
                ln_width,
                syn,
                style_context,
            )
        } else {
            push_wrapped_diff_line_with_style_context(
                row.line_no,
                diff_type,
                row.code,
                render_width as usize,
                ln_width,
                style_context,
            )
        };
        let first_line = wrapped.into_iter().next().unwrap_or_else(|| Line::from(""));
        first_line.render(
            Rect::new(area.x.saturating_add(left_pad), y, render_width, 1),
            buf,
        );
    }
}

impl Renderable for ThemePreviewWideRenderable {
    fn desired_height(&self, _width: u16) -> u16 {
        u16::MAX
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        render_preview(
            area,
            buf,
            &WIDE_PREVIEW_ROWS,
            /*center_vertically*/ true,
            WIDE_PREVIEW_LEFT_INSET,
        );
    }
}

impl Renderable for ThemePreviewNarrowRenderable {
    fn desired_height(&self, _width: u16) -> u16 {
        NARROW_PREVIEW_ROWS.len() as u16
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        render_preview(
            area,
            buf,
            &NARROW_PREVIEW_ROWS,
            /*center_vertically*/ false,
            /*left_inset*/ 0,
        );
    }
}

fn subtitle_available_width(terminal_width: Option<u16>) -> usize {
    let width = terminal_width.unwrap_or(80);
    let content_width = popup_content_width(width);
    if let Some((list_width, _side_width)) = side_by_side_layout_widths(
        content_width,
        SideContentWidth::Half,
        WIDE_PREVIEW_MIN_WIDTH,
    ) {
        list_width as usize
    } else {
        content_width as usize
    }
}

fn theme_picker_subtitle(reflect_home: Option<&Path>, terminal_width: Option<u16>) -> String {
    let themes_dir = reflect_home.map(|home| home.join("themes"));
    let themes_dir_display = themes_dir
        .as_deref()
        .map(|path| format_directory_display(path, /*max_width*/ None));
    let available_width = subtitle_available_width(terminal_width);

    if let Some(path) = themes_dir_display
        && path.starts_with('~')
    {
        let subtitle = format!("Custom .tmTheme files can be added to the {path} directory.");
        if UnicodeWidthStr::width(subtitle.as_str()) <= available_width {
            return subtitle;
        }
    }

    PREVIEW_FALLBACK_SUBTITLE.to_string()
}

/// 为 `/theme` 选择器对话框构建 [`SelectionViewParams`]。
///
/// 列出所有捆绑主题和自定义 `.tmTheme` 文件，光标
/// 移动时实时预览，取消时恢复。
///
/// `current_name` 应为 `Config::tui_theme` 的值（持久化的
/// 偏好）。当它命名的主题当前可用时，选择器
/// 预选它；否则选择器回退到已配置的名称（或
/// 自适应默认值），因此打开没有持久化偏好的选择器
/// 仍会高亮最可能想要的条目。
pub(crate) fn build_theme_picker_params(
    current_name: Option<&str>,
    reflect_home: Option<&Path>,
    terminal_width: Option<u16>,
) -> SelectionViewParams {
    // 截取当前主题快照，以便取消时恢复。
    let original_theme = highlight::current_syntax_theme();

    let entries = highlight::list_available_themes(reflect_home);
    let reflect_home_owned = reflect_home.map(Path::to_path_buf);

    // 解析有效的主题名称：仅当显式配置
    // 当前可用时才采用；否则回退到已配置/默认选择，
    // 使打开 `/theme` 不会自动预览无关的第一个条目。
    let effective_name = if let Some(name) = current_name
        && entries.iter().any(|entry| entry.name == name)
    {
        name.to_string()
    } else {
        highlight::configured_theme_name()
    };

    // 跟踪当前主题的索引以便预选。
    let mut initial_idx = None;

    let items: Vec<SelectionItem> = entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let display_name = if entry.is_custom {
                format!("{} (custom)", entry.name)
            } else {
                entry.name.clone()
            };
            let is_current = entry.name == effective_name;
            if is_current {
                initial_idx = Some(idx);
            }
            let name_for_action = entry.name.clone();
            SelectionItem {
                name: display_name,
                is_current,
                dismiss_on_select: true,
                search_value: Some(entry.name.clone()),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::SyntaxThemeSelected {
                        name: name_for_action.clone(),
                    });
                })],
                ..Default::default()
            }
        })
        .collect();

    // 从最终的 `items` 列表（而非 `entries`）派生预览目标，
    // 以便在条目构造/排序变化时预览顺序保持对齐。
    let preview_theme_names: Vec<Option<String>> =
        items.iter().map(|item| item.search_value.clone()).collect();
    let preview_home = reflect_home_owned.clone();
    let on_selection_changed = Some(Box::new(
        move |idx: usize, tx: &crate::tui_core::app_event_sender::AppEventSender| {
            if let Some(Some(name)) = preview_theme_names.get(idx)
                && let Some(theme) = highlight::resolve_theme_by_name(name, preview_home.as_deref())
            {
                highlight::set_syntax_theme(theme);
                tx.send(AppEvent::SyntaxThemePreviewed);
            }
        },
    )
        as Box<dyn Fn(usize, &crate::tui_core::app_event_sender::AppEventSender) + Send + Sync>);

    // 取消时恢复原始主题。
    let on_cancel = Some(Box::new(
        move |tx: &crate::tui_core::app_event_sender::AppEventSender| {
            highlight::set_syntax_theme(original_theme.clone());
            tx.send(AppEvent::SyntaxThemePreviewed);
        },
    )
        as Box<
            dyn Fn(&crate::tui_core::app_event_sender::AppEventSender) + Send + Sync,
        >);
    SelectionViewParams {
        title: Some("Select Syntax Theme".to_string()),
        subtitle: Some(theme_picker_subtitle(
            reflect_home_owned.as_deref(),
            terminal_width,
        )),
        footer_hint: Some(standard_popup_hint_line()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to filter themes...".to_string()),
        initial_selected_idx: initial_idx,
        side_content: Box::new(ThemePreviewWideRenderable),
        side_content_width: SideContentWidth::Half,
        side_content_min_width: WIDE_PREVIEW_MIN_WIDTH,
        stacked_side_content: Some(Box::new(ThemePreviewNarrowRenderable)),
        preserve_side_content_bg: true,
        on_selection_changed,
        on_cancel,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::style::Modifier;

    fn render_buffer(renderable: &dyn Renderable, width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        renderable.render(area, &mut buf);
        buf
    }

    fn render_lines(renderable: &dyn Renderable, width: u16, height: u16) -> Vec<String> {
        let buf = render_buffer(renderable, width, height);
        (0..height)
            .map(|row| {
                let mut line = String::new();
                for col in 0..width {
                    let symbol = buf[(col, row)].symbol();
                    if symbol.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(symbol);
                    }
                }
                line
            })
            .collect()
    }

    fn first_non_space_style_after_marker(buf: &Buffer, row: u16, width: u16) -> Option<Modifier> {
        let marker_col = (0..width)
            .find(|&col| buf[(col, row)].symbol() == "-" || buf[(col, row)].symbol() == "+")?;
        for col in marker_col + 1..width {
            if buf[(col, row)].symbol() != " " {
                return Some(buf[(col, row)].style().add_modifier);
            }
        }
        None
    }

    fn preview_line_number(line: &str) -> Option<usize> {
        let trimmed = line.trim_start();
        let digits_len = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits_len == 0 {
            return None;
        }
        let digits = &trimmed[..digits_len];
        if !trimmed[digits_len..].starts_with(' ') {
            return None;
        }
        digits.parse::<usize>().ok()
    }

    fn preview_line_marker(line: &str) -> Option<char> {
        let trimmed = line.trim_start();
        let digits_len = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits_len == 0 {
            return None;
        }
        let mut chars = trimmed[digits_len..].chars();
        if chars.next()? != ' ' {
            return None;
        }
        chars.next()
    }

    #[test]
    fn theme_picker_uses_half_width_with_stacked_fallback_preview() {
        let params = build_theme_picker_params(
            /*current_name*/ None, /*reflect_home*/ None, /*terminal_width*/ None,
        );
        assert_eq!(params.side_content_width, SideContentWidth::Half);
        assert_eq!(params.side_content_min_width, WIDE_PREVIEW_MIN_WIDTH);
        assert!(params.stacked_side_content.is_some());
    }

    #[test]
    fn theme_picker_items_include_search_values_for_preview_mapping() {
        let params = build_theme_picker_params(
            /*current_name*/ None, /*reflect_home*/ None, /*terminal_width*/ None,
        );
        assert!(
            params.items.iter().all(|item| item.search_value.is_some()),
            "theme picker preview mapping relies on item search_value to stay aligned with final item order"
        );
    }

    #[test]
    fn wide_preview_renders_all_lines_with_vertical_center_and_left_inset() {
        let lines = render_lines(
            &ThemePreviewWideRenderable,
            /*width*/ 80,
            /*height*/ 20,
        );
        let numbered_rows: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter_map(|(idx, line)| preview_line_number(line).map(|_| idx))
            .collect();
        let total_preview_lines = WIDE_PREVIEW_ROWS.len();

        assert_eq!(numbered_rows.len(), total_preview_lines);
        let first_row = *numbered_rows
            .first()
            .expect("expected at least one preview row");
        let last_row = *numbered_rows
            .last()
            .expect("expected at least one preview row");
        assert!(
            first_row > 0,
            "expected top padding before centered preview"
        );
        assert!(
            last_row < 19,
            "expected bottom padding after centered preview"
        );

        let first_line = &lines[first_row];
        assert!(
            first_line.starts_with("  31  fn summarize"),
            "expected wide preview to start after a 2-char inset"
        );

        let markers: Vec<char> = lines
            .iter()
            .filter_map(|line| preview_line_marker(line))
            .collect();
        assert!(
            markers.contains(&'+'),
            "expected wide preview to include at least one addition line"
        );
        assert!(
            markers.contains(&'-'),
            "expected wide preview to include at least one removal line"
        );
    }

    #[test]
    fn narrow_preview_renders_single_add_and_single_remove_in_four_lines() {
        let lines = render_lines(
            &ThemePreviewNarrowRenderable,
            /*width*/ 80,
            /*height*/ 6,
        );
        let numbered_lines: Vec<usize> = lines
            .iter()
            .filter_map(|line| preview_line_number(line))
            .collect();
        let markers: Vec<char> = lines
            .iter()
            .filter_map(|line| preview_line_marker(line))
            .collect();

        assert_eq!(numbered_lines, vec![12, 13, 13, 14]);
        assert_eq!(markers.len(), 4);
        assert_eq!(markers.iter().filter(|&&m| m == '+').count(), 1);
        assert_eq!(markers.iter().filter(|&&m| m == '-').count(), 1);
        let first_numbered = lines
            .iter()
            .find(|line| preview_line_number(line).is_some())
            .expect("expected at least one rendered preview row");
        assert!(
            first_numbered.starts_with("12  fn greet"),
            "expected narrow preview line numbers to start at the left edge"
        );
    }

    #[test]
    fn deleted_preview_code_uses_dim_overlay_like_real_diff_renderer() {
        let width = 80;
        let height = 6;
        let buf = render_buffer(&ThemePreviewNarrowRenderable, width, height);
        let lines = render_lines(&ThemePreviewNarrowRenderable, width, height);
        let deleted_row = lines
            .iter()
            .enumerate()
            .find_map(|(row, line)| (preview_line_marker(line) == Some('-')).then_some(row as u16))
            .expect("expected a deleted preview row");
        let modifiers = first_non_space_style_after_marker(&buf, deleted_row, width)
            .expect("expected code text after diff marker");
        assert!(
            modifiers.contains(Modifier::DIM),
            "expected deleted preview code to be dimmed"
        );
    }

    #[test]
    fn subtitle_uses_tilde_path_when_reflect_home_under_home_directory() {
        let home = dirs::home_dir().expect("home directory should be available");
        let reflect_home = home.join(".reflect");

        let subtitle = theme_picker_subtitle(Some(&reflect_home), Some(200));

        assert!(subtitle.contains("~"));
        assert!(subtitle.contains("directory"));
    }

    #[test]
    fn subtitle_falls_back_when_tilde_path_subtitle_is_too_wide() {
        let home = dirs::home_dir().expect("home directory should be available");
        let long_segment = "a".repeat(120);
        let reflect_home = home.join(long_segment).join(".reflect");

        let subtitle = theme_picker_subtitle(Some(&reflect_home), Some(140));

        assert_eq!(subtitle, PREVIEW_FALLBACK_SUBTITLE);
    }

    #[test]
    fn subtitle_falls_back_to_preview_instructions_without_tilde_path() {
        let subtitle =
            theme_picker_subtitle(/*reflect_home*/ None, /*terminal_width*/ None);
        assert_eq!(subtitle, PREVIEW_FALLBACK_SUBTITLE);
    }

    #[test]
    fn subtitle_falls_back_for_94_column_terminal_side_by_side_layout() {
        let home = dirs::home_dir().expect("home directory should be available");
        let reflect_home = home.join(".reflect");

        let subtitle = theme_picker_subtitle(Some(&reflect_home), Some(94));

        assert_eq!(subtitle, PREVIEW_FALLBACK_SUBTITLE);
    }

    #[test]
    fn unavailable_configured_theme_falls_back_to_configured_or_default_selection() {
        let configured_or_default_theme = highlight::configured_theme_name();
        let params = build_theme_picker_params(
            Some("not-a-real-theme"),
            /*reflect_home*/ None,
            Some(120),
        );
        let selected_idx = params
            .initial_selected_idx
            .expect("expected selected index for active fallback theme");
        let selected_name = params.items[selected_idx]
            .search_value
            .as_deref()
            .expect("expected search value to contain canonical theme name");

        assert_eq!(selected_name, configured_or_default_theme);
    }
}
