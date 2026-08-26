//! v1.x Tier 4.5:`/image` picker overlay widget。
//!
//! 居中覆盖层,复用 `plan_approval_block` 的布局(标题 + 列表 + 底部 hint)。
//! 列出 cwd 下符合 MIME + 大小约束的图片文件,选中后回填 `@<path>` 到 composer。
//!
//! 渲染管线:`tui/mod.rs::draw` 调用 `image_picker::draw(frame, state, area)`,
//! 在所有其它区块之上画(类似 `plan_approval_block`)。

use crate::events::{ImageFileEntry, ImagePickerState, UiState};
use crate::tui_core::custom_terminal::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// MIME 白名单(对齐归档 `image_attach.rs::is_supported_image`)。
const SUPPORTED_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/bmp",
];

/// 扫描 cwd 下一层(非递归)的图片文件,写入 picker。
///
/// 失败(无 cwd / 无权限)静默返回,不推 Notice——调用方在打开 picker 后
/// 自然会发现 files 为空。
pub fn scan_cwd(picker: &mut ImagePickerState) {
    picker.files.clear();
    let Ok(entries) = std::fs::read_dir(&picker.cwd) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(mime) = infer_mime(&path) else {
            continue;
        };
        if !SUPPORTED_MIMES.contains(&mime.as_str()) {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if size > picker.max_size_mib * 1024 * 1024 {
            continue; // 超过 10 MiB 上限,跳过
        }
        picker.files.push(ImageFileEntry {
            path,
            size_bytes: size,
            mime,
        });
    }
    picker.selected = 0;
}

/// `handle_key` 返回的动作。`Attach(path)` 把 `@<path>` 回填 composer(由调用方执行)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImagePickerAction {
    Close,
    Attach(std::path::PathBuf),
    Nop,
}

/// 处理按键。`↑/k` `↓/j` 移动;`Enter` 选中并 attach;`Esc/q/Ctrl+C` 关闭。
pub fn handle_key(
    picker: &mut ImagePickerState,
    k: crossterm::event::KeyEvent,
) -> ImagePickerAction {
    use crossterm::event::{KeyCode, KeyModifiers};
    if k.kind == crossterm::event::KeyEventKind::Release {
        return ImagePickerAction::Nop;
    }
    if k.code == KeyCode::Esc
        || matches!(k.code, KeyCode::Char('q'))
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
    {
        return ImagePickerAction::Close;
    }
    let last = picker.files.len().saturating_sub(1);
    match k.code {
        KeyCode::Up | KeyCode::Char('k') => {
            picker.selected = picker.selected.saturating_sub(1);
            ImagePickerAction::Nop
        }
        KeyCode::Down | KeyCode::Char('j') => {
            picker.selected = (picker.selected + 1).min(last);
            ImagePickerAction::Nop
        }
        KeyCode::Enter => picker
            .files
            .get(picker.selected)
            .map(|f| ImagePickerAction::Attach(f.path.clone()))
            .unwrap_or(ImagePickerAction::Nop),
        _ => ImagePickerAction::Nop,
    }
}

/// 简易 MIME 推断(按扩展名,不需要 mime_guess 依赖)。
fn infer_mime(path: &std::path::Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(
        match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => return None,
        }
        .to_string(),
    )
}

/// 渲染 image picker(居中覆盖层)。由 `tui/mod.rs::draw` 在主布局之上调用。
pub fn draw(frame: &mut Frame<'_>, state: &UiState, area: Rect) {
    let picker = match &state.image_picker {
        Some(p) => p,
        None => return,
    };
    let modal_area = centered_rect(75, 70, area);
    frame.render_widget_ref(Clear, modal_area);
    frame.render_widget_ref(build_paragraph(picker), modal_area);
}

fn build_paragraph(picker: &ImagePickerState) -> Paragraph<'static> {
    let title = Line::from(Span::styled(
        " 🖼  Image Picker ",
        Style::default()
            .fg(Color::Magenta)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
    ));

    let mut body: Vec<Line<'static>> = Vec::new();
    body.push(Line::from(Span::styled(
        format!(" cwd: {}", picker.cwd.display()),
        Style::default().fg(Color::DarkGray),
    )));
    body.push(Line::from(""));

    if picker.files.is_empty() {
        body.push(Line::from(Span::styled(
            "  (no supported images in cwd)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, f) in picker.files.iter().enumerate() {
            let is_sel = i == picker.selected;
            let marker = if is_sel { "▶" } else { " " };
            let color = if is_sel { Color::Yellow } else { Color::White };
            let name = f
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("(?)")
                .to_string();
            let size_kb = f.size_bytes / 1024;
            body.push(Line::from(vec![
                Span::styled(format!(" {marker} "), Style::default().fg(color)),
                Span::styled(name, Style::default().fg(color)),
                Span::styled(
                    format!("  {}  {} KiB", f.mime, size_kb),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    let hint = Line::from(vec![
        Span::styled(
            " ↑↓ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("select · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Enter ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("attach · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "Esc ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("close", Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let mut all = vec![title];
    all.extend(body);
    all.push(Line::from(""));
    all.push(hint);
    Paragraph::new(all).block(block).wrap(Wrap { trim: false })
}

/// 复制居中矩形 helper(避免每次 picker 都内联)。
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let pop_w = area.width.saturating_mul(percent_x) / 100;
    let pop_h = area.height.saturating_mul(percent_y) / 100;
    let pop_w = pop_w.max(40).min(area.width);
    let pop_h = pop_h.max(10).min(area.height);
    let x = area.x + (area.width.saturating_sub(pop_w)) / 2;
    let y = area.y + (area.height.saturating_sub(pop_h)) / 2;
    Rect::new(x, y, pop_w, pop_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_mime_known_extensions() {
        assert_eq!(
            infer_mime(std::path::Path::new("a.png")).as_deref(),
            Some("image/png")
        );
        assert_eq!(
            infer_mime(std::path::Path::new("b.JPG")).as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(
            infer_mime(std::path::Path::new("c.gif")).as_deref(),
            Some("image/gif")
        );
        assert_eq!(
            infer_mime(std::path::Path::new("d.webp")).as_deref(),
            Some("image/webp")
        );
    }

    #[test]
    fn infer_mime_unknown_returns_none() {
        assert!(infer_mime(std::path::Path::new("a.txt")).is_none());
        assert!(infer_mime(std::path::Path::new("README")).is_none());
    }

    #[test]
    fn scan_cwd_finds_supported_images() {
        // 在 std::env::temp_dir() 下建一个 png + 一个 txt,扫描应只含 png。
        let tmp = std::env::temp_dir().join("reflect_image_picker_test");
        let _ = std::fs::create_dir_all(&tmp);
        let png_path = tmp.join("test.png");
        let txt_path = tmp.join("test.txt");
        let _ = std::fs::write(&png_path, b"\x89PNG_FAKE");
        let _ = std::fs::write(&txt_path, b"hello");

        let mut picker = ImagePickerState {
            cwd: tmp.clone(),
            ..Default::default()
        };
        scan_cwd(&mut picker);
        let names: Vec<String> = picker
            .files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"test.png".to_string()), "got: {names:?}");
        assert!(!names.contains(&"test.txt".to_string()), "txt 不应入选");

        // 清理
        let _ = std::fs::remove_file(&png_path);
        let _ = std::fs::remove_file(&txt_path);
        let _ = std::fs::remove_dir(&tmp);
    }

    #[test]
    fn scan_cwd_empty_dir() {
        let mut picker = ImagePickerState::default();
        scan_cwd(&mut picker);
        // 不 panic 即可;具体内容取决于环境
        assert!(picker.selected == 0);
    }

    fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn handle_key_j_k_navigate() {
        let mut picker = ImagePickerState {
            files: vec![
                ImageFileEntry {
                    path: "a.png".into(),
                    size_bytes: 1,
                    mime: "image/png".into(),
                },
                ImageFileEntry {
                    path: "b.png".into(),
                    size_bytes: 1,
                    mime: "image/png".into(),
                },
            ],
            selected: 0,
            ..Default::default()
        };
        // 下移 → 索引 1。
        let act = handle_key(&mut picker, key(crossterm::event::KeyCode::Char('j')));
        assert!(matches!(act, ImagePickerAction::Nop));
        assert_eq!(picker.selected, 1);
        // 越界钳到最后一个。
        handle_key(&mut picker, key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(picker.selected, 1);
        // 上移 → 索引 0。
        handle_key(&mut picker, key(crossterm::event::KeyCode::Char('k')));
        assert_eq!(picker.selected, 0);
        // 不会下溢。
        handle_key(&mut picker, key(crossterm::event::KeyCode::Char('k')));
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn handle_key_esc_closes() {
        let mut picker = ImagePickerState::default();
        let act = handle_key(&mut picker, key(crossterm::event::KeyCode::Esc));
        assert!(matches!(act, ImagePickerAction::Close));
    }

    #[test]
    fn handle_key_enter_attaches_selected_path() {
        let mut picker = ImagePickerState {
            files: vec![ImageFileEntry {
                path: std::path::PathBuf::from("logo.png"),
                size_bytes: 1,
                mime: "image/png".into(),
            }],
            selected: 0,
            ..Default::default()
        };
        let act = handle_key(&mut picker, key(crossterm::event::KeyCode::Enter));
        match act {
            ImagePickerAction::Attach(p) => {
                assert_eq!(p, std::path::PathBuf::from("logo.png"))
            }
            other => panic!("expected Attach, got {other:?}"),
        }
    }

    #[test]
    fn handle_key_enter_on_empty_is_nop() {
        let mut picker = ImagePickerState::default();
        let act = handle_key(&mut picker, key(crossterm::event::KeyCode::Enter));
        assert!(matches!(act, ImagePickerAction::Nop));
    }
}
