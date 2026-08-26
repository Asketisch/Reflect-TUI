//! 自定义终端（custom_terminal） 的测试集。
//!
//! 从 custom_terminal.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use pretty_assertions::assert_eq;
use ratatui::backend::WindowSize;
use ratatui::layout::Rect;
use ratatui::style::Style;

struct CaptureBackend {
    output: Vec<u8>,
    size: Size,
    cursor: Position,
}

impl CaptureBackend {
    fn new(width: u16, height: u16) -> Self {
        Self {
            output: Vec::new(),
            size: Size { width, height },
            cursor: Position { x: 0, y: 0 },
        }
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

impl Write for CaptureBackend {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.output.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Backend for CaptureBackend {
    fn draw<'a, I>(&mut self, _content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.cursor = position.into();
        Ok(())
    }

    fn clear(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn clear_region(&mut self, _clear_type: ClearType) -> io::Result<()> {
        Ok(())
    }

    fn append_lines(&mut self, _line_count: u16) -> io::Result<()> {
        Ok(())
    }

    fn scroll_region_up(
        &mut self,
        _region: std::ops::Range<u16>,
        _scroll_by: u16,
    ) -> io::Result<()> {
        Ok(())
    }

    fn scroll_region_down(
        &mut self,
        _region: std::ops::Range<u16>,
        _scroll_by: u16,
    ) -> io::Result<()> {
        Ok(())
    }

    fn size(&self) -> io::Result<Size> {
        Ok(self.size)
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size,
            pixels: self.size,
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn diff_buffers_does_not_emit_clear_to_end_for_full_width_row() {
    let area = Rect::new(0, 0, 3, 2);
    let previous = Buffer::empty(area);
    let mut next = Buffer::empty(area);

    next.cell_mut((2, 0))
        .expect("cell should exist")
        .set_symbol("X");

    let commands = diff_buffers(&previous, &next);

    let clear_count = commands
        .iter()
        .filter(|command| matches!(command, DrawCommand::ClearToEnd { y, .. } if *y == 0))
        .count();
    assert_eq!(
        0, clear_count,
        "expected diff_buffers not to emit ClearToEnd; commands: {commands:?}",
    );
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, DrawCommand::Put { x: 2, y: 0, .. })),
        "expected diff_buffers to update the final cell; commands: {commands:?}",
    );
}

#[test]
fn diff_buffers_clear_to_end_starts_after_wide_char() {
    let area = Rect::new(0, 0, 10, 1);
    let mut previous = Buffer::empty(area);
    let mut next = Buffer::empty(area);

    previous.set_string(0, 0, "中文", Style::default());
    next.set_string(0, 0, "中", Style::default());

    let commands = diff_buffers(&previous, &next);
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, DrawCommand::ClearToEnd { x: 2, y: 0, .. })),
        "expected clear-to-end to start after the remaining wide char; commands: {commands:?}"
    );
}

#[test]
fn terminal_draw_applies_requested_cursor_style() {
    let mut output = Vec::new();
    let mut terminal =
        Terminal::with_options(CaptureBackend::new(/*width*/ 2, /*height*/ 1)).expect("terminal");
    terminal.set_viewport_area(Rect::new(0, 0, 2, 1));

    terminal
        .try_draw(|frame| {
            frame.set_cursor_style(SetCursorStyle::SteadyBar);
            frame.set_cursor_position((0, 0));
            io::Result::Ok(())
        })
        .expect("draw");

    queue!(output, SetCursorStyle::SteadyBar).expect("queue style");
    let expected = String::from_utf8(output).expect("utf8");
    let actual = terminal.backend().output();
    assert!(
        actual.contains(&expected),
        "expected terminal output to contain cursor style {expected:?}, got {actual:?}"
    );
}

#[test]
fn reset_cursor_style_emits_default_user_shape() {
    let mut output = Vec::new();
    let mut terminal =
        Terminal::with_options(CaptureBackend::new(/*width*/ 2, /*height*/ 1)).expect("terminal");

    terminal.reset_cursor_style().expect("reset cursor style");
    ratatui::backend::Backend::flush(terminal.backend_mut()).expect("flush backend");

    queue!(output, SetCursorStyle::DefaultUserShape).expect("queue style");
    let expected = String::from_utf8(output).expect("utf8");
    let actual = terminal.backend().output();
    assert!(
        actual.contains(&expected),
        "expected terminal output to contain cursor style reset {expected:?}, got {actual:?}"
    );
}

/// 端到端复现:宽度缩放后,重新绘制相同内容,模拟终端屏幕是否残留旧像素
/// 或出现重叠。用 VT100Backend(真实 vt100 终端模拟)验证 draw→flush→diff
/// 的输出在缩放后是否干净。
#[test]
fn resize_width_then_redraw_no_corruption() {
    use crate::tui_core::test_backend::VT100Backend;
    use crate::viewport::prepare_bottom_viewport;
    use ratatui::prelude::Widget;
    use ratatui::widgets::{Block, Paragraph};
    // 用一个足够宽的网格,viewport 在其中缩放。
    let backend = VT100Backend::new(80, 8);
    let mut terminal = Terminal::with_options(backend).expect("terminal");

    // 第一帧:viewport 宽 60,画一段填充内容(走 prepare_bottom_viewport 的真实路径)。
    prepare_bottom_viewport(&mut terminal, 60, 8, 8).expect("prepare 1");
    terminal
        .draw(|frame| {
            let area = frame.area();
            Paragraph::new("AAAAAAAAAA").block(Block::bordered()).render(area, frame.buffer_mut());
        })
        .expect("draw frame 1");

    // 第二帧:宽度缩到 40,重新画相同结构的内容(再次走 prepare_bottom_viewport)。
    prepare_bottom_viewport(&mut terminal, 40, 8, 8).expect("prepare 2");
    terminal
        .draw(|frame| {
            let area = frame.area();
            Paragraph::new("BBBBBBBBBB").block(Block::bordered()).render(area, frame.buffer_mut());
        })
        .expect("draw frame 2");

    // 断言:屏幕里不应残留第一帧的 A(若 buffer 未正确重置,diff 会漏掉旧行
    // 宽错位的 cell,留下 A 残影/重叠)。
    let text = terminal.backend().vt100().screen().contents();
    assert!(
        !text.contains('A'),
        "宽度缩放后不应残留旧帧的 A(二维布局损坏/残影): {text}"
    );
    assert!(text.contains('B'), "新帧内容应渲染: {text}");
}

/// 端到端 reflow:模拟 Resize 后主循环「clear scrollback + 用新宽度重发
/// state.history」的真实路径。第一帧按宽 60 推入一条 history item;
/// 触发 `clear_scrollback_and_visible_screen_ansi` 模拟 Resize 后清屏;
/// 第二帧按宽 40 推入新 history item。最终屏幕里**不应**残留第一条
/// history 的标记(WIDE),也不应出现行宽错位的 cell(已入 scrollback 的
/// 旧宽度行永久错位是用户报告的"缩放后混乱"症状之一)。
#[test]
fn resize_reflow_reemits_history_at_new_width() {
    use crate::adapter::UiHistoryItem;
    use crate::history_render::history_item_to_lines;
    use crate::tui_core::insert_history::insert_history_lines;
    use crate::tui_core::test_backend::VT100Backend;

    let backend = VT100Backend::new(80, 24);
    let mut terminal = Terminal::with_options(backend).expect("terminal");
    terminal.set_viewport_area(ratatui::layout::Rect::new(0, 0, 80, 24));

    // 第一帧:viewport 宽 60,推入一条 history 行(WIDE 标记)。
    terminal.set_viewport_area(ratatui::layout::Rect::new(0, 0, 60, 8));
    let wide_lines = history_item_to_lines(
        &UiHistoryItem::Notice("history-WIDE".into()),
        58, // 60 - 2(margin) = wrap_width
        std::path::Path::new("."),
    );
    insert_history_lines(&mut terminal, wide_lines).expect("insert wide");

    // 模拟 Resize 后清屏(reflow 修复的核心动作)。
    terminal
        .clear_scrollback_and_visible_screen_ansi()
        .expect("clear scrollback+screen");

    // 第二帧:viewport 宽 40,推入新 history 行(NARROW 标记)。
    terminal.set_viewport_area(ratatui::layout::Rect::new(0, 0, 40, 8));
    let narrow_lines = history_item_to_lines(
        &UiHistoryItem::Notice("history-NARROW".into()),
        38,
        std::path::Path::new("."),
    );
    insert_history_lines(&mut terminal, narrow_lines).expect("insert narrow");

    let text = terminal.backend().vt100().screen().contents();
    assert!(
        !text.contains("history-WIDE"),
        "reflow 后不应残留旧宽度 history 行: {text}"
    );
    assert!(text.contains("history-NARROW"), "新 history 应渲染: {text}");
}

/// 端到端 resize 复现 #2:模拟 tui/mod.rs 主循环两帧的真实路径
/// (prepare_bottom_viewport + 真实 `draw` 函数,带完整状态/HUD/composer)——
/// 与用户报告"缩放后 TUI 显示混乱"对应的场景。失败时打印屏幕内容便于诊断。
/// (此测试需要访问 crate 私有项,在 tui/mod.rs draw_tests 中重写。)

/// 宽度变化时 `set_viewport_area` 必须重置 current buffer,避免一维
/// `Buffer::resize` 把旧像素按旧行宽错位排进新缓冲区(缩放后重叠/残影)。
///
/// `Buffer::reset()` 把每个 cell 内容清空(符号归 ' ' / 默认样式),所以
/// 断言:缩窄后 current buffer 所有 cell 都是空符号。
#[test]
fn set_viewport_area_resets_current_buffer_on_width_change() {
    let mut terminal = Terminal::with_options(CaptureBackend::new(/*width*/ 10, /*height*/ 4))
        .expect("terminal");
    terminal.set_viewport_area(Rect::new(0, 0, 10, 4));

    // 在 current buffer 写入内容(模拟已渲染的一帧)。
    {
        let buf = terminal.current_buffer_mut();
        for cell in buf.content.iter_mut() {
            cell.set_symbol("X");
        }
    }
    let any_filled_before = terminal
        .current_buffer_mut()
        .content
        .iter()
        .any(|c| c.symbol() != " ");
    assert!(any_filled_before, "前置:写入后应有非空 cell");

    // 宽度变化(10 → 6):应触发 current buffer reset。
    terminal.set_viewport_area(Rect::new(0, 0, 6, 4));

    let all_empty_after = terminal
        .current_buffer_mut()
        .content
        .iter()
        .all(|c| c.symbol() == " ");
    assert!(
        all_empty_after,
        "宽度变化后 current buffer 应被清空(强制下一帧全量重绘),否则一维 resize 会损坏二维布局"
    );
    // 新 area 元数据正确同步。
    assert_eq!(terminal.current_buffer_mut().area.width, 6);
}

/// 宽度不变(仅高度变化)时不应 reset —— 高度变化不影响 (x,y)↔index 映射,
/// 无需全量重绘,保留 diff 增量更新以减少闪烁。
#[test]
fn set_viewport_area_keeps_buffer_on_height_only_change() {
    let mut terminal = Terminal::with_options(CaptureBackend::new(/*width*/ 8, /*height*/ 4))
        .expect("terminal");
    terminal.set_viewport_area(Rect::new(0, 0, 8, 4));

    {
        let buf = terminal.current_buffer_mut();
        for cell in buf.content.iter_mut() {
            cell.set_symbol("Y");
        }
    }
    // 高度变化(4 → 2),宽度不变:不应 reset。
    terminal.set_viewport_area(Rect::new(0, 0, 8, 2));

    let any_filled_after = terminal
        .current_buffer_mut()
        .content
        .iter()
        .any(|c| c.symbol() == "Y");
    assert!(
        any_filled_after,
        "宽度不变时不应清空 buffer(保留增量 diff)"
    );
}
