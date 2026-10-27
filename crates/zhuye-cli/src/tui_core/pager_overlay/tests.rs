//! 分页浮层 的测试集。
//!
//! 从 pager_overlay.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::app_server_protocol::CommandExecutionSource as ExecCommandSource;
use crate::tui_core::history_cell::ReviewDecision;
use insta::assert_snapshot;
use pretty_assertions::assert_eq;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::protocol_compat::parse_command::ParsedCommand;
use crate::tui_core::diff_model::FileChange;
use crate::tui_core::exec_cell::CommandOutput;
use crate::tui_core::history_cell;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::history_cell::new_patch_event;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::text::Text;

#[derive(Debug)]
struct TestCell {
    lines: Vec<Line<'static>>,
}

impl crate::tui_core::history_cell::HistoryCell for TestCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines.clone()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.lines.clone()
    }

    fn transcript_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines.clone()
    }
}

#[derive(Debug)]
struct HeightCountingCell {
    height_calls: Arc<AtomicUsize>,
}

impl crate::tui_core::history_cell::HistoryCell for HeightCountingCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec![Line::from("counted")]
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        vec![Line::from("counted")]
    }

    fn desired_transcript_height(&self, _width: u16) -> u16 {
        self.height_calls.fetch_add(1, Ordering::Relaxed);
        1
    }
}

fn paragraph_block(label: &str, lines: usize) -> Box<dyn Renderable> {
    let text = Text::from(
        (0..lines)
            .map(|i| Line::from(format!("{label}{i}")))
            .collect::<Vec<_>>(),
    );
    Box::new(Paragraph::new(text)) as Box<dyn Renderable>
}

fn default_pager_keymap() -> crate::tui_core::keymap::PagerKeymap {
    crate::tui_core::keymap::RuntimeKeymap::defaults().pager
}

fn transcript_overlay(cells: Vec<Arc<dyn HistoryCell>>) -> TranscriptOverlay {
    TranscriptOverlay::new(cells, default_pager_keymap())
}

fn static_overlay(lines: Vec<Line<'static>>, title: &str) -> StaticOverlay {
    StaticOverlay::with_title(lines, title.to_string(), default_pager_keymap())
}

fn pager_view(
    renderables: Vec<Box<dyn Renderable>>,
    title: &str,
    scroll_offset: usize,
) -> PagerView {
    PagerView::new(
        renderables,
        title.to_string(),
        scroll_offset,
        default_pager_keymap(),
    )
}

#[test]
fn edit_prev_hint_is_visible() {
    let mut overlay = transcript_overlay(vec![Arc::new(TestCell {
        lines: vec![Line::from("hello")],
    })]);

    // 渲染到足够宽的缓冲区，确保页脚提示不被截断。
    let area = Rect::new(0, 0, 120, 10);
    let mut buf = Buffer::empty(area);
    overlay.render(area, &mut buf);

    let s = buffer_to_text(&buf, area);
    assert!(
        s.contains("edit prev"),
        "expected 'edit prev' hint in overlay footer, got: {s:?}"
    );
}

#[test]
fn edit_next_hint_is_visible_when_highlighted() {
    let mut overlay = transcript_overlay(vec![Arc::new(TestCell {
        lines: vec![Line::from("hello")],
    })]);
    overlay.set_highlight_cell(Some(0));

    // 渲染到足够宽的缓冲区，确保页脚提示不被截断。
    let area = Rect::new(0, 0, 120, 10);
    let mut buf = Buffer::empty(area);
    overlay.render(area, &mut buf);

    let s = buffer_to_text(&buf, area);
    assert!(
        s.contains("edit next"),
        "expected 'edit next' hint in overlay footer, got: {s:?}"
    );
}

#[test]
fn transcript_overlay_snapshot_basic() {
    // 准备一个包含若干行的会话记录浮层
    let mut overlay = transcript_overlay(vec![
        Arc::new(TestCell {
            lines: vec![Line::from("alpha")],
        }),
        Arc::new(TestCell {
            lines: vec![Line::from("beta")],
        }),
        Arc::new(TestCell {
            lines: vec![Line::from("gamma")],
        }),
    ]);
    let mut term = Terminal::new(TestBackend::new(40, 10)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");
    assert_snapshot!(term.backend());
}

#[test]
fn transcript_overlay_preserves_semantic_web_links() {
    let destination = "https://example.com/a/very/long/path";
    let mut overlay = transcript_overlay(vec![Arc::new(history_cell::AgentMarkdownCell::new(
        destination.to_string(),
        std::path::Path::new("/tmp"),
    ))]);
    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 24, /*height*/ 10,
    );
    let mut buf = Buffer::empty(area);

    overlay.render(area, &mut buf);

    assert!(area.positions().any(|position| {
        buf[position]
            .symbol()
            .contains(&format!("\x1b]8;;{destination}\x07"))
    }));
}

#[test]
fn transcript_overlay_renders_live_tail() {
    let mut overlay = transcript_overlay(vec![Arc::new(TestCell {
        lines: vec![Line::from("alpha")],
    })]);
    overlay.sync_live_tail(
        /*width*/ 40,
        Some(ActiveCellTranscriptKey {
            revision: 1,
            is_stream_continuation: false,
            animation_tick: None,
        }),
        |_| Some(vec![HyperlinkLine::from("tail")]),
    );

    let mut term = Terminal::new(TestBackend::new(40, 10)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");
    assert_snapshot!(term.backend());
}

#[test]
fn transcript_overlay_live_tail_preserves_semantic_web_links() {
    let destination = "https://example.com/a/streamed/path";
    let cell =
        history_cell::AgentMarkdownCell::new(destination.to_string(), std::path::Path::new("/tmp"));
    let mut overlay = transcript_overlay(Vec::new());
    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 24, /*height*/ 10,
    );
    let mut buf = Buffer::empty(area);

    overlay.sync_live_tail(
        area.width,
        Some(ActiveCellTranscriptKey {
            revision: 1,
            is_stream_continuation: false,
            animation_tick: None,
        }),
        |width| Some(cell.transcript_hyperlink_lines(width)),
    );
    overlay.render(area, &mut buf);

    assert!(area.positions().any(|position| {
        buf[position]
            .symbol()
            .contains(&format!("\x1b]8;;{destination}\x07"))
    }));
}

#[test]
fn transcript_overlay_sync_live_tail_is_noop_for_identical_key() {
    let mut overlay = transcript_overlay(vec![Arc::new(TestCell {
        lines: vec![Line::from("alpha")],
    })]);

    let calls = std::cell::Cell::new(0usize);
    let key = ActiveCellTranscriptKey {
        revision: 1,
        is_stream_continuation: false,
        animation_tick: None,
    };

    overlay.sync_live_tail(/*width*/ 40, Some(key), |_| {
        calls.set(calls.get() + 1);
        Some(vec![HyperlinkLine::from("tail")])
    });
    overlay.sync_live_tail(/*width*/ 40, Some(key), |_| {
        calls.set(calls.get() + 1);
        Some(vec![HyperlinkLine::from("tail2")])
    });

    assert_eq!(calls.get(), 1);
}

fn buffer_to_text(buf: &Buffer, area: Rect) -> String {
    let mut out = String::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let symbol = buf[(x, y)].symbol();
            if symbol.is_empty() {
                out.push(' ');
            } else {
                out.push(symbol.chars().next().unwrap_or(' '));
            }
        }
        // 去除行尾空格以保证输出稳定。
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    out
}

#[test]
fn transcript_overlay_apply_patch_scroll_vt100_clears_previous_page() {
    let cwd = PathBuf::from("/repo");
    let mut cells: Vec<Arc<dyn HistoryCell>> = Vec::new();

    let mut approval_changes = HashMap::new();
    approval_changes.insert(
        PathBuf::from("foo.txt"),
        FileChange::Add {
            content: "hello\nworld\n".to_string(),
        },
    );
    let approval_cell: Arc<dyn HistoryCell> = Arc::new(new_patch_event(approval_changes, &cwd));
    cells.push(approval_cell);

    let mut apply_changes = HashMap::new();
    apply_changes.insert(
        PathBuf::from("foo.txt"),
        FileChange::Add {
            content: "hello\nworld\n".to_string(),
        },
    );
    let apply_begin_cell: Arc<dyn HistoryCell> = Arc::new(new_patch_event(apply_changes, &cwd));
    cells.push(apply_begin_cell);

    let apply_end_cell: Arc<dyn HistoryCell> = history_cell::new_approval_decision_cell(
        history_cell::ApprovalDecisionSubject::Command(vec!["ls".into()]),
        ReviewDecision::Approved,
        history_cell::ApprovalDecisionActor::User,
    )
    .into();
    cells.push(apply_end_cell);

    let mut exec_cell = crate::tui_core::exec_cell::new_active_exec_command(
        "exec-1".into(),
        vec!["bash".into(), "-lc".into(), "ls".into()],
        vec![ParsedCommand::Unknown { cmd: "ls".into() }],
        ExecCommandSource::Agent,
        /*interaction_input*/ None,
        /*animations_enabled*/ true,
    );
    exec_cell.complete_call(
        "exec-1",
        CommandOutput::new(/*exit_code*/ 0, "src\nREADME.md\n".into()),
        Duration::from_millis(420),
    );
    let exec_cell: Arc<dyn HistoryCell> = Arc::new(exec_cell);
    cells.push(exec_cell);

    let mut overlay = transcript_overlay(cells);
    let area = Rect::new(0, 0, 80, 12);
    let mut buf = Buffer::empty(area);

    overlay.render(area, &mut buf);
    overlay.view.scroll_offset = 0;
    overlay.render(area, &mut buf);

    let snapshot = buffer_to_text(&buf, area);
    assert_snapshot!("transcript_overlay_apply_patch_scroll_vt100", snapshot);
}

#[test]
fn transcript_overlay_keeps_scroll_pinned_at_bottom() {
    let mut overlay = transcript_overlay(
        (0..20)
            .map(|i| {
                Arc::new(TestCell {
                    lines: vec![Line::from(format!("line{i}"))],
                }) as Arc<dyn HistoryCell>
            })
            .collect(),
    );
    let mut term = Terminal::new(TestBackend::new(40, 12)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");

    assert!(
        overlay.view.is_scrolled_to_bottom(),
        "expected initial render to leave view at bottom"
    );

    overlay.insert_cell(Arc::new(TestCell {
        lines: vec!["tail".into()],
    }));

    assert_eq!(overlay.view.scroll_offset, usize::MAX);
}

#[test]
fn transcript_overlay_preserves_manual_scroll_position() {
    let mut overlay = transcript_overlay(
        (0..20)
            .map(|i| {
                Arc::new(TestCell {
                    lines: vec![Line::from(format!("line{i}"))],
                }) as Arc<dyn HistoryCell>
            })
            .collect(),
    );
    let mut term = Terminal::new(TestBackend::new(40, 12)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");

    overlay.view.scroll_offset = 0;

    overlay.insert_cell(Arc::new(TestCell {
        lines: vec!["tail".into()],
    }));

    assert_eq!(overlay.view.scroll_offset, 0);
}

#[test]
fn transcript_overlay_insert_preserves_cached_cell_heights() {
    let height_calls = Arc::new(AtomicUsize::new(0));
    let mut overlay = transcript_overlay(vec![Arc::new(HeightCountingCell {
        height_calls: height_calls.clone(),
    })]);
    let area = Rect::new(0, 0, 40, 12);
    let mut buf = Buffer::empty(area);

    overlay.render(area, &mut buf);
    assert_eq!(height_calls.load(Ordering::Relaxed), 1);

    overlay.insert_cell(Arc::new(TestCell {
        lines: vec![Line::from("inserted")],
    }));
    overlay.render(area, &mut buf);

    assert_eq!(height_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn transcript_overlay_consolidation_remaps_highlight_inside_range() {
    let mut overlay = transcript_overlay(
        (0..6)
            .map(|i| {
                Arc::new(TestCell {
                    lines: vec![Line::from(format!("line{i}"))],
                }) as Arc<dyn HistoryCell>
            })
            .collect(),
    );
    overlay.set_highlight_cell(Some(3));

    overlay.consolidate_cells(
        2..5,
        Arc::new(TestCell {
            lines: vec![Line::from("consolidated")],
        }),
    );

    assert_eq!(
        overlay.highlight_cell,
        Some(2),
        "highlight inside consolidated range should point to replacement cell",
    );
}

#[test]
fn transcript_overlay_consolidation_remaps_highlight_after_range() {
    let mut overlay = transcript_overlay(
        (0..7)
            .map(|i| {
                Arc::new(TestCell {
                    lines: vec![Line::from(format!("line{i}"))],
                }) as Arc<dyn HistoryCell>
            })
            .collect(),
    );
    overlay.set_highlight_cell(Some(6));

    overlay.consolidate_cells(
        2..5,
        Arc::new(TestCell {
            lines: vec![Line::from("consolidated")],
        }),
    );

    assert_eq!(
        overlay.highlight_cell,
        Some(4),
        "highlight after consolidated range should shift left by removed cells",
    );
}

#[test]
fn static_overlay_snapshot_basic() {
    // 准备一个包含若干行内容和标题的静态浮层
    let mut overlay = static_overlay(
        vec!["one".into(), "two".into(), "three".into()],
        "S T A T I C",
    );
    let mut term = Terminal::new(TestBackend::new(40, 10)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");
    assert_snapshot!(term.backend());
}

/// 渲染会话记录浮层，并按顺序返回可见的行号（`line-NN`）。
fn transcript_line_numbers(overlay: &mut TranscriptOverlay, area: Rect) -> Vec<usize> {
    let mut buf = Buffer::empty(area);
    overlay.render(area, &mut buf);

    let top_h = area.height.saturating_sub(3);
    let top = Rect::new(area.x, area.y, area.width, top_h);
    let content_area = overlay.view.content_area(top);

    let mut nums = Vec::new();
    for y in content_area.y..content_area.bottom() {
        let mut line = String::new();
        for x in content_area.x..content_area.right() {
            line.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
        }
        if let Some(n) = line
            .split_whitespace()
            .find_map(|w| w.strip_prefix("line-"))
            .and_then(|s| s.parse().ok())
        {
            nums.push(n);
        }
    }
    nums
}

#[test]
fn transcript_overlay_paging_is_continuous_and_round_trips() {
    let mut overlay = transcript_overlay(
        (0..50)
            .map(|i| {
                Arc::new(TestCell {
                    lines: vec![Line::from(format!("line-{i:02}"))],
                }) as Arc<dyn HistoryCell>
            })
            .collect(),
    );
    let area = Rect::new(0, 0, 40, 15);

    // 先执行一次布局，使 last_content_height 被填充，从而分页使用真实内容高度。
    let mut buf = Buffer::empty(area);
    overlay.view.scroll_offset = 0;
    overlay.render(area, &mut buf);
    let page_height = overlay.view.page_height(area);

    // 场景 1：从顶部开始，PageDown 应显示下一页内容。
    overlay.view.scroll_offset = 0;
    let page1 = transcript_line_numbers(&mut overlay, area);
    let page1_len = page1.len();
    let expected_page1: Vec<usize> = (0..page1_len).collect();
    assert_eq!(
        page1, expected_page1,
        "first page should start at line-00 and show a full page of content"
    );

    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_add(page_height);
    let page2 = transcript_line_numbers(&mut overlay, area);
    assert_eq!(
        page2.len(),
        page1_len,
        "second page should have the same number of visible lines as the first page"
    );
    let expected_page2_first = *page1.last().unwrap() + 1;
    assert_eq!(
        page2[0], expected_page2_first,
        "second page after PageDown should immediately follow the first page"
    );

    // 场景 2：从内部偏移（start=3）出发，PageDown 后再 PageUp 应能往返还原。
    let interior_offset = 3usize;
    overlay.view.scroll_offset = interior_offset;
    let before = transcript_line_numbers(&mut overlay, area);
    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_add(page_height);
    let _ = transcript_line_numbers(&mut overlay, area);
    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_sub(page_height);
    let after = transcript_line_numbers(&mut overlay, area);
    assert_eq!(
        before, after,
        "PageDown+PageUp from interior offset ({interior_offset}) should round-trip"
    );

    // 场景 3：从第二页顶部出发，PageUp 后再 PageDown 应能往返还原。
    overlay.view.scroll_offset = page_height;
    let before2 = transcript_line_numbers(&mut overlay, area);
    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_sub(page_height);
    let _ = transcript_line_numbers(&mut overlay, area);
    overlay.view.scroll_offset = overlay.view.scroll_offset.saturating_add(page_height);
    let after2 = transcript_line_numbers(&mut overlay, area);
    assert_eq!(
        before2, after2,
        "PageUp+PageDown from the top of the second page should round-trip"
    );
}

#[test]
fn static_overlay_wraps_long_lines() {
    let mut overlay = static_overlay(
        vec![
            "a very long line that should wrap when rendered within a narrow pager overlay width"
                .into(),
        ],
        "S T A T I C",
    );
    let mut term = Terminal::new(TestBackend::new(24, 8)).expect("term");
    term.draw(|f| overlay.render(f.area(), f.buffer_mut()))
        .expect("draw");
    assert_snapshot!(term.backend());
}

#[test]
fn pager_view_content_height_counts_renderables() {
    let pv = pager_view(
        vec![
            paragraph_block("a", /*lines*/ 2),
            paragraph_block("b", /*lines*/ 3),
        ],
        "T",
        /*scroll_offset*/ 0,
    );

    assert_eq!(pv.content_height(/*width*/ 80), 5);
}

#[test]
fn pager_view_ensure_chunk_visible_scrolls_down_when_needed() {
    let mut pv = pager_view(
        vec![
            paragraph_block("a", /*lines*/ 1),
            paragraph_block("b", /*lines*/ 3),
            paragraph_block("c", /*lines*/ 3),
        ],
        "T",
        /*scroll_offset*/ 0,
    );
    let area = Rect::new(0, 0, 20, 8);

    pv.scroll_offset = 0;
    let content_area = pv.content_area(area);
    pv.ensure_chunk_visible(/*idx*/ 2, content_area);

    let mut buf = Buffer::empty(area);
    pv.render(area, &mut buf);
    let rendered = buffer_to_text(&buf, area);

    assert!(
        rendered.contains("c0"),
        "expected chunk top in view: {rendered:?}"
    );
    assert!(
        rendered.contains("c1"),
        "expected chunk middle in view: {rendered:?}"
    );
    assert!(
        rendered.contains("c2"),
        "expected chunk bottom in view: {rendered:?}"
    );
}

#[test]
fn pager_view_ensure_chunk_visible_scrolls_up_when_needed() {
    let mut pv = pager_view(
        vec![
            paragraph_block("a", /*lines*/ 2),
            paragraph_block("b", /*lines*/ 3),
            paragraph_block("c", /*lines*/ 3),
        ],
        "T",
        /*scroll_offset*/ 0,
    );
    let area = Rect::new(0, 0, 20, 3);

    pv.scroll_offset = 6;
    pv.ensure_chunk_visible(/*idx*/ 0, area);

    assert_eq!(pv.scroll_offset, 0);
}

#[test]
fn pager_view_is_scrolled_to_bottom_accounts_for_wrapped_height() {
    let mut pv = pager_view(
        vec![paragraph_block("a", /*lines*/ 10)],
        "T",
        /*scroll_offset*/ 0,
    );
    let area = Rect::new(0, 0, 20, 8);
    let mut buf = Buffer::empty(area);

    pv.render(area, &mut buf);

    assert!(
        !pv.is_scrolled_to_bottom(),
        "expected view to report not at bottom when offset < max"
    );

    pv.scroll_offset = usize::MAX;
    pv.render(area, &mut buf);

    assert!(
        pv.is_scrolled_to_bottom(),
        "expected view to report at bottom after scrolling to end"
    );
}
