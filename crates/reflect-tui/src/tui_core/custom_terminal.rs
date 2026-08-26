// 本文件衍生自 `ratatui::Terminal`，其遵循以下许可条款：
//
// MIT 许可证 (MIT)
// 版权所有 (c) 2016-2022 Florian Dehau
// 版权所有 (c) 2023-2025 The Ratatui Developers
//
// 特此免费授予任何获得本软件及相关文档文件（“本软件”）副本的人以
// 不受限制地处理本软件的权利，包括但不限于使用、复制、修改、合并、
// 发布、分发、再许可和/或销售本软件副本的权利，并允许向其提供本
// 软件的人这样做，但须满足以下条件：
//
// 上述版权声明和本许可声明应包含在本软件的所有副本或实质性部分中。
//
// 本软件按“原样”提供，不附有任何形式的明示或暗示的保证，包括但
// 不限于适销性、特定用途适用性和非侵权性的保证。在任何情况下，
// 作者或版权持有人均不对任何索赔、损害或其他责任负责，无论是在
// 合同、侵权或其他行为中，还是因本软件或与本软件的使用或其他交易
// 有关而引发。
use std::io;
use std::io::Write;

use crossterm::cursor::MoveTo;
use crossterm::cursor::SetCursorStyle;
use crossterm::queue;
use crossterm::style::Colors;
use crossterm::style::Print;
use crossterm::style::SetAttribute;
use crossterm::style::SetBackgroundColor;
use crossterm::style::SetColors;
use crossterm::style::SetForegroundColor;
use crossterm::terminal::Clear;
use derive_more::IsVariant;
use ratatui::backend::Backend;
use ratatui::backend::ClearType;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::layout::Size;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::widgets::WidgetRef;
use unicode_width::UnicodeWidthStr;

/// 返回单元格符号的显示宽度，忽略 OSC 转义序列。
///
/// OSC 序列（例如 OSC 8 超链接：`\x1B]8;;URL\x07`）是不占用显示列宽的终端
/// 控制序列。标准的 `UnicodeWidthStr::width()` 方法会错误地把 OSC
/// 载荷中的可打印字符（如 `]`、`8`、`;` 以及 URL 字符）也算进宽度。
/// 本函数先剥离这些序列，使只有可见字符才对宽度有贡献。
fn display_width(s: &str) -> usize {
    // 快速路径：不存在转义序列。
    if !s.contains('\x1B') {
        return s.width();
    }

    // 剥离 OSC 序列：ESC ] ... BEL
    let mut visible = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1B' && chars.clone().next() == Some(']') {
            // 消耗掉 ']' 以及其后直到并包括 BEL 的所有内容。
            chars.next(); // 跳过 ']'
            for c in chars.by_ref() {
                if c == '\x07' {
                    break;
                }
            }
            continue;
        }
        visible.push(ch);
    }
    visible.width()
}

pub struct Frame<'a> {
    /// 绘制完这一帧后，光标应该位于何处？
    ///
    /// 若为 `None`，则光标被隐藏，其位置由后端控制。若为 `Some((x, y))`，
    /// 则在调用 `Terminal::draw()` 后显示光标并将其置于 `(x, y)`。
    pub(crate) cursor_position: Option<Position>,

    /// 绘制完这一帧后要应用的可见光标形状。
    cursor_style: SetCursorStyle,

    /// 视口区域
    pub(crate) viewport_area: Rect,

    /// 用于绘制当前帧的缓冲区
    pub(crate) buffer: &'a mut Buffer,
}

impl Frame<'_> {
    /// 当前帧的区域
    ///
    /// 这一区域在渲染期间保证不会改变，因此可以多次调用。
    ///
    /// 如果你的应用会监听来自后端的尺寸变化事件，那么对于任何用于渲染当前帧的计算，
    /// 都应忽略事件中的值，而改用此值，因为这里返回的正是用于渲染当前帧的缓冲区区域。
    pub const fn area(&self) -> Rect {
        self.viewport_area
    }

    /// 使用 [`WidgetRef::render_ref`] 将 [`WidgetRef`] 渲染到当前缓冲区。
    ///
    /// 通常 area 参数是当前帧的大小或当前帧的一个子区域（可通过 [`Layout`]
    /// 分割总区域来获得）。
    #[allow(clippy::needless_pass_by_value)]
    pub fn render_widget_ref<W: WidgetRef>(&mut self, widget: W, area: Rect) {
        widget.render_ref(area, self.buffer);
    }

    /// 绘制完这一帧后，显示光标并将其置于指定的 (x, y) 坐标处。
    /// 如果未调用此方法，光标将被隐藏。
    ///
    /// 请注意，这会与 [`Terminal::hide_cursor`]、[`Terminal::show_cursor`] 以及
    /// [`Terminal::set_cursor_position`] 的调用相互干扰。请选择其中一种 API 并坚持使用。
    ///
    /// [`Terminal::hide_cursor`]: crate::Terminal::hide_cursor
    /// [`Terminal::show_cursor`]: crate::Terminal::show_cursor
    /// [`Terminal::set_cursor_position`]: crate::Terminal::set_cursor_position
    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) {
        self.cursor_position = Some(position.into());
    }

    /// 绘制完这一帧后，设置终端的可见光标样式。
    pub fn set_cursor_style(&mut self, style: SetCursorStyle) {
        self.cursor_style = style;
    }

    /// 以可变引用的形式获取此 `Frame` 绘制所用的缓冲区。
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        self.buffer
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
pub struct Terminal<B>
where
    B: Backend + Write,
{
    /// 用于与终端交互的后端
    backend: B,
    /// 保存当前与上一次绘制调用的结果。每次绘制结束后会比较两者，
    /// 以便向终端输出必要的更新
    buffers: [Buffer; 2],
    /// 在上述数组中当前缓冲区的索引
    current: usize,
    /// 光标当前是否被隐藏
    pub hidden_cursor: bool,
    /// 视口区域
    pub viewport_area: Rect,
    /// 终端最后一次已知的尺寸。用于检测内部缓冲区是否需要调整大小。
    pub last_known_screen_size: Size,
    /// 光标最后一次已知的位置。当视口处于内联模式且终端被调整大小时，
    /// 用于确定新的区域。
    pub last_known_cursor_pos: Position,
    /// 内联模式下渲染在视口上方的可见历史行数。
    visible_history_rows: u16,
}

impl<B> Drop for Terminal<B>
where
    B: Backend,
    B: Write,
{
    #[allow(clippy::print_stderr)]
    fn drop(&mut self) {
        // 尝试恢复光标状态
        if let Err(err) = self.reset_cursor_style() {
            eprintln!("Failed to reset the cursor style: {err}");
        }

        if self.hidden_cursor
            && let Err(err) = self.show_cursor()
        {
            eprintln!("Failed to show the cursor: {err}");
        }
    }
}

impl<B> Terminal<B>
where
    B: Backend,
    B: Write,
{
    /// 使用给定的 [`Backend`] 和 [`TerminalOptions`] 创建一个新的 [`Terminal`]。
    pub fn with_options(mut backend: B) -> io::Result<Self> {
        let screen_size = backend.size()?;
        let cursor_pos = backend.get_cursor_position().unwrap_or_else(|err| {
            // 某些 PTY 不会响应 CPR（`ESC[6n`）；与其导致 TUI 启动失败，
            // 不如使用一个安全的默认值继续。
            tracing::warn!("failed to read initial cursor position; defaulting to origin: {err}");
            Position { x: 0, y: 0 }
        });
        Ok(Self {
            backend,
            buffers: [Buffer::empty(Rect::ZERO), Buffer::empty(Rect::ZERO)],
            current: 0,
            hidden_cursor: false,
            viewport_area: Rect::new(
                /*x 坐标*/ 0,
                cursor_pos.y,
                /*宽度*/ 0,
                /*高度*/ 0,
            ),
            last_known_screen_size: screen_size,
            last_known_cursor_pos: cursor_pos,
            visible_history_rows: 0,
        })
    }

    /// 根据调用方提供的初始光标位置创建一个新的 [`Terminal`]。
    ///
    /// 获取一个 Frame 对象，它提供对渲染所用终端状态的一致视图。
    pub fn get_frame(&mut self) -> Frame<'_> {
        Frame {
            cursor_position: None,
            cursor_style: SetCursorStyle::DefaultUserShape,
            viewport_area: self.viewport_area,
            buffer: self.current_buffer_mut(),
        }
    }

    /// 以引用形式获取当前缓冲区。
    fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.current]
    }

    /// 以可变引用形式获取当前缓冲区。
    fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    /// 以引用形式获取上一个缓冲区。
    fn previous_buffer(&self) -> &Buffer {
        &self.buffers[1 - self.current]
    }

    /// 以可变引用形式获取上一个缓冲区。
    fn previous_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[1 - self.current]
    }

    /// 获取后端
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    /// 以可变引用形式获取后端
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// 计算上一个缓冲区与当前缓冲区之间的差异，并将其传递给当前后端进行绘制。
    pub fn flush(&mut self) -> io::Result<()> {
        let updates = diff_buffers(self.previous_buffer(), self.current_buffer());
        let last_put_command = updates.iter().rfind(|command| command.is_put());
        if let Some(&DrawCommand::Put { x, y, .. }) = last_put_command {
            self.last_known_cursor_pos = Position { x, y };
        }
        draw(&mut self.backend, updates.into_iter())
    }

    /// 更新 Terminal，使内部缓冲区与请求的区域一致。
    ///
    /// 请求的区域会被保存下来，以便在渲染时保持一致。这会导致整屏清空。
    pub fn resize(&mut self, screen_size: Size) -> io::Result<()> {
        self.last_known_screen_size = screen_size;
        Ok(())
    }

    /// 设置视口区域。
    pub fn set_viewport_area(&mut self, area: Rect) {
        // 宽度变化防御:`Buffer::resize` 是一维 truncate/pad 扁平 `Vec<Cell>`,
        // 不重映射 (x,y)↔index。宽度变了仍直接 resize 会把旧像素按旧行宽
        // 错位排进新缓冲区,下一帧 diff 输出与终端真实状态错位 → 缩放后
        // 出现重叠/错乱/残影。检测到纯宽度变化时,resize 后 reset current
        // buffer(与 previous 一起全空),强制下一帧全量重绘新宽度的整帧,
        // 绕开一维 resize 的二维布局损坏。高度变化不影响 (x,y) 映射,无需此处理。
        let width_changed = !self.viewport_area.is_empty() && self.viewport_area.width != area.width;
        self.current_buffer_mut().resize(area);
        self.previous_buffer_mut().resize(area);
        if width_changed {
            // resize 已把 content 长度对齐新 area;reset 清空所有 cell 内容,
            // 让 diff(current vs previous)下一帧输出整帧,避免残留旧像素。
            self.current_buffer_mut().reset();
        }
        self.viewport_area = area;
        self.visible_history_rows = self.visible_history_rows.min(area.top());
    }

    /// 向后端查询尺寸；如果与上一次的尺寸不一致，则执行调整。
    pub fn autoresize(&mut self) -> io::Result<()> {
        let screen_size = self.size()?;
        if screen_size != self.last_known_screen_size {
            self.resize(screen_size)?;
        }
        Ok(())
    }

    /// 向终端绘制单帧。
    ///
    /// 成功时返回一个 [`CompletedFrame`]，失败时返回 [`std::io::Error`]。
    ///
    /// 如果传入本方法的渲染回调可能失败，请改用 [`try_draw`]。
    ///
    /// 应用程序应在循环中调用 `draw` 或 [`try_draw`] 以持续渲染终端。
    /// 这些方法是向终端绘制的主要入口。
    ///
    /// [`try_draw`]: Terminal::try_draw
    ///
    /// 此方法将：
    ///
    /// - 在必要时自动调整终端尺寸
    /// - 调用渲染回调，并传入一个 [`Frame`] 引用供其渲染
    /// - 通过把当前缓冲区复制到后端来刷新当前内部状态
    /// - 如果在渲染闭包中设置了光标位置，则把光标移动到最后一次已知的位置
    ///
    /// 渲染回调在被调用时应完整渲染整帧，包括与上一帧相比没有变化的部分。
    /// 这是因为每一帧都会与上一帧比较以确定变化的部分，且只有变化的部分会写入终端。
    /// 如果渲染回调没有完整渲染整帧，终端将无法保持一致状态。
    pub fn draw<F>(&mut self, render_callback: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        self.try_draw(|frame| {
            render_callback(frame);
            io::Result::Ok(())
        })
    }

    /// 尝试向终端绘制单帧。
    ///
    /// 成功时返回包含 [`CompletedFrame`] 的 [`Result::Ok`]，失败时返回
    /// 包含导致失败的 [`std::io::Error`] 的 [`Result::Err`]。
    ///
    /// 这与 [`Terminal::draw`] 等价，区别在于渲染回调是返回 `Result` 的函数或闭包，
    /// 而不是什么都不返回。
    ///
    /// 应用程序应在循环中调用 `try_draw` 或 [`draw`] 以持续渲染终端。
    /// 这些方法是向终端绘制的主要入口。
    ///
    /// [`draw`]: Terminal::draw
    ///
    /// 此方法将：
    ///
    /// - 在必要时自动调整终端尺寸
    /// - 调用渲染回调，并传入一个 [`Frame`] 引用供其渲染
    /// - 通过把当前缓冲区复制到后端来刷新当前内部状态
    /// - 如果在渲染闭包中设置了光标位置，则把光标移动到最后一次已知的位置
    /// - 返回包含当前缓冲区和终端区域的 [`CompletedFrame`]
    ///
    /// 传给 `try_draw` 的渲染回调可以返回任意错误类型可经 [`Into`] 特征转换为
    /// [`std::io::Error`] 的 [`Result`]。这使得可以用 `?` 运算符传播渲染期间发生的错误。
    /// 如果渲染回调返回错误，该错误会作为 [`std::io::Error`] 从 `try_draw` 返回，
    /// 并且终端不会被更新。
    ///
    /// 此方法返回的 [`CompletedFrame`] 对调试或测试很有用，但在常规应用中通常不会用到。
    ///
    /// 渲染回调在被调用时应完整渲染整帧，包括与上一帧相比没有变化的部分。
    /// 这是因为每一帧都会与上一帧比较以确定变化的部分，且只有变化的部分会写入终端。
    /// 如果渲染函数没有完整渲染整帧，终端将无法保持一致状态。
    pub fn try_draw<F, E>(&mut self, render_callback: F) -> io::Result<()>
    where
        F: FnOnce(&mut Frame) -> Result<(), E>,
        E: Into<io::Error>,
    {
        // 自动调整尺寸——否则在缩小时会出现闪烁，在放大时窗口与终端之间
        // 可能出现不同步，从而越界。
        self.autoresize()?;

        let mut frame = self.get_frame();

        render_callback(&mut frame).map_err(Into::into)?;

        // 我们不能立刻更改光标位置，因为必须先向 stdout 刷新帧。但也不能
        // 继续持有 frame，因为它持有一个 Buffer 的 &mut。因此，我们把
        // 重要的数据从 Frame 中取出，然后将其丢弃。
        let cursor_position = frame.cursor_position;
        let cursor_style = frame.cursor_style;

        // 绘制到 stdout
        self.flush()?;

        match cursor_position {
            None => self.hide_cursor()?,
            Some(position) => {
                self.set_cursor_style(cursor_style)?;
                self.show_cursor()?;
                self.set_cursor_position(position)?;
            }
        }

        self.swap_buffers();

        Backend::flush(&mut self.backend)?;

        Ok(())
    }

    /// 隐藏光标。
    pub fn hide_cursor(&mut self) -> io::Result<()> {
        self.backend.hide_cursor()?;
        self.hidden_cursor = true;
        Ok(())
    }

    /// 显示光标。
    pub fn show_cursor(&mut self) -> io::Result<()> {
        self.backend.show_cursor()?;
        self.hidden_cursor = false;
        Ok(())
    }

    /// 设置可见的终端光标样式。
    pub fn set_cursor_style(&mut self, style: SetCursorStyle) -> io::Result<()> {
        queue!(self.backend, style)
    }

    /// 恢复用户配置的终端光标样式。
    pub fn reset_cursor_style(&mut self) -> io::Result<()> {
        self.set_cursor_style(SetCursorStyle::DefaultUserShape)
    }

    /// 获取当前光标位置。
    ///
    /// 这是上一次绘制调用之后光标所处的位置。
    #[allow(dead_code)]
    pub fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.backend.get_cursor_position()
    }

    /// 设置光标位置。
    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        let position = position.into();
        self.backend.set_cursor_position(position)?;
        self.last_known_cursor_pos = position;
        Ok(())
    }

    /// 清空终端，并强制下次绘制调用时进行全量重绘。
    pub fn clear(&mut self) -> io::Result<()> {
        if self.viewport_area.is_empty() {
            return Ok(());
        }
        self.clear_after_position(self.viewport_area.as_position())
    }

    /// 从 `position` 开始清空到可见屏幕末尾，并强制全量重绘。
    pub(crate) fn clear_after_position(&mut self, position: Position) -> io::Result<()> {
        self.backend.set_cursor_position(position)?;
        self.backend.clear_region(ClearType::AfterCursor)?;
        // 重置后备缓冲区，确保下一次更新会重绘所有内容。
        self.previous_buffer_mut().reset();
        Ok(())
    }

    /// 使用显式 ANSI 序列硬重置回滚缓冲区和可见屏幕。
    ///
    /// 某些终端在 purge 与 clear 以单个 ANSI 序列发出（而不是作为独立的后端命令）时
    /// 表现得更为可靠。
    pub fn clear_scrollback_and_visible_screen_ansi(&mut self) -> io::Result<()> {
        if self.viewport_area.is_empty() {
            return Ok(());
        }

        // 重置滚动区域和样式状态、光标归位、清屏、清除回滚缓冲区。
        // 该顺序与常见的 shell `clear && printf '\e[3J'` 行为一致。
        write!(self.backend, "\x1b[r\x1b[0m\x1b[H\x1b[2J\x1b[3J\x1b[H")?;
        std::io::Write::flush(&mut self.backend)?;
        self.last_known_cursor_pos = Position { x: 0, y: 0 };
        self.visible_history_rows = 0;
        self.previous_buffer_mut().reset();
        Ok(())
    }

    pub(crate) fn note_history_rows_inserted(&mut self, inserted_rows: u16) {
        self.visible_history_rows = self
            .visible_history_rows
            .saturating_add(inserted_rows)
            .min(self.viewport_area.top());
    }

    /// 清空未激活的缓冲区，并将其与当前缓冲区交换
    pub fn swap_buffers(&mut self) {
        self.previous_buffer_mut().reset();
        self.current = 1 - self.current;
    }

    /// 查询后端的真实尺寸。
    pub fn size(&self) -> io::Result<Size> {
        self.backend.size()
    }
}

use ratatui::buffer::Cell;

#[derive(Debug, IsVariant)]
enum DrawCommand {
    Put { x: u16, y: u16, cell: Cell },
    ClearToEnd { x: u16, y: u16, bg: Color },
}

fn diff_buffers(a: &Buffer, b: &Buffer) -> Vec<DrawCommand> {
    let previous_buffer = &a.content;
    let next_buffer = &b.content;

    let mut updates = vec![];
    let mut last_nonblank_columns = vec![0; a.area.height as usize];
    for y in 0..a.area.height {
        let row_start = y as usize * a.area.width as usize;
        let row_end = row_start + a.area.width as usize;
        let row = &next_buffer[row_start..row_end];
        let bg = row.last().map(|cell| cell.bg).unwrap_or(Color::Reset);

        // 扫描该行，找出最右侧仍需要关注的列：任何非空格字形、任何背景色与行尾
        // 背景色不同的单元格，或任何带修饰符的单元格。多宽度字形会把该区域延伸
        // 到其完整的显示宽度。超过该点之后，行的其余部分可以用一次 ClearToEnd
        // 清空，这比发出多个空格 Put 命令更高效。
        let mut last_nonblank_column = 0usize;
        let mut column = 0usize;
        while column < row.len() {
            let cell = &row[column];
            let width = display_width(cell.symbol());
            if cell.symbol() != " " || cell.bg != bg || cell.modifier != Modifier::empty() {
                last_nonblank_column = column + (width.saturating_sub(1));
            }
            column += width.max(1); // 将零宽符号视为宽度 1
        }

        if last_nonblank_column + 1 < row.len() {
            let (x, y) = a.pos_of(row_start + last_nonblank_column + 1);
            updates.push(DrawCommand::ClearToEnd { x, y, bg });
        }

        last_nonblank_columns[y as usize] = last_nonblank_column as u16;
    }

    // 因绘制/替换前面多宽度字符而失效的单元格：
    let mut invalidated: usize = 0;
    // 当前缓冲区中需要跳过的单元格：因其位置被前面的多宽度字符占用
    //（被跳过的单元格本来就应该是空白），或因为逐单元格跳过：
    let mut to_skip: usize = 0;
    for (i, (current, previous)) in next_buffer.iter().zip(previous_buffer.iter()).enumerate() {
        if !current.skip && (current != previous || invalidated > 0) && to_skip == 0 {
            let (x, y) = a.pos_of(i);
            let row = i / a.area.width as usize;
            if x <= last_nonblank_columns[row] {
                updates.push(DrawCommand::Put {
                    x,
                    y,
                    cell: next_buffer[i].clone(),
                });
            }
        }

        to_skip = display_width(current.symbol()).saturating_sub(1);

        let affected_width = std::cmp::max(
            display_width(current.symbol()),
            display_width(previous.symbol()),
        );
        invalidated = std::cmp::max(affected_width, invalidated).saturating_sub(1);
    }
    updates
}

fn draw<I>(writer: &mut impl Write, commands: I) -> io::Result<()>
where
    I: Iterator<Item = DrawCommand>,
{
    let mut fg = Color::Reset;
    let mut bg = Color::Reset;
    let mut modifier = Modifier::empty();
    let mut last_pos: Option<Position> = None;
    for command in commands {
        let (x, y) = match command {
            DrawCommand::Put { x, y, .. } => (x, y),
            DrawCommand::ClearToEnd { x, y, .. } => (x, y),
        };
        // 如果上一次的位置不是 (x - 1, y)，则移动光标
        if !matches!(last_pos, Some(p) if x == p.x + 1 && y == p.y) {
            queue!(writer, MoveTo(x, y))?;
        }
        last_pos = Some(Position { x, y });
        match command {
            DrawCommand::Put { cell, .. } => {
                if cell.modifier != modifier {
                    let diff = ModifierDiff {
                        from: modifier,
                        to: cell.modifier,
                    };
                    diff.queue(writer)?;
                    modifier = cell.modifier;
                }
                if cell.fg != fg || cell.bg != bg {
                    queue!(
                        writer,
                        SetColors(Colors::new(cell.fg.into(), cell.bg.into()))
                    )?;
                    fg = cell.fg;
                    bg = cell.bg;
                }

                queue!(writer, Print(cell.symbol()))?;
            }
            DrawCommand::ClearToEnd { bg: clear_bg, .. } => {
                queue!(writer, SetAttribute(crossterm::style::Attribute::Reset))?;
                modifier = Modifier::empty();
                queue!(writer, SetBackgroundColor(clear_bg.into()))?;
                bg = clear_bg;
                queue!(writer, Clear(crossterm::terminal::ClearType::UntilNewLine))?;
            }
        }
    }

    queue!(
        writer,
        SetForegroundColor(crossterm::style::Color::Reset),
        SetBackgroundColor(crossterm::style::Color::Reset),
        SetAttribute(crossterm::style::Attribute::Reset),
    )?;

    Ok(())
}

/// `ModifierDiff` 结构体用于计算两个 `Modifier` 值之间的差异。
/// 这在更新终端显示时很有用，因为它只发送必要的更改，
/// 从而带来更高效的更新。
struct ModifierDiff {
    pub from: Modifier,
    pub to: Modifier,
}

impl ModifierDiff {
    fn queue<W: io::Write>(self, w: &mut W) -> io::Result<()> {
        use crossterm::style::Attribute as CAttribute;
        let removed = self.from - self.to;
        if removed.contains(Modifier::REVERSED) {
            queue!(w, SetAttribute(CAttribute::NoReverse))?;
        }
        if removed.contains(Modifier::BOLD) {
            queue!(w, SetAttribute(CAttribute::NormalIntensity))?;
            if self.to.contains(Modifier::DIM) {
                queue!(w, SetAttribute(CAttribute::Dim))?;
            }
        }
        if removed.contains(Modifier::ITALIC) {
            queue!(w, SetAttribute(CAttribute::NoItalic))?;
        }
        if removed.contains(Modifier::UNDERLINED) {
            queue!(w, SetAttribute(CAttribute::NoUnderline))?;
        }
        if removed.contains(Modifier::DIM) {
            queue!(w, SetAttribute(CAttribute::NormalIntensity))?;
        }
        if removed.contains(Modifier::CROSSED_OUT) {
            queue!(w, SetAttribute(CAttribute::NotCrossedOut))?;
        }
        if removed.contains(Modifier::SLOW_BLINK) || removed.contains(Modifier::RAPID_BLINK) {
            queue!(w, SetAttribute(CAttribute::NoBlink))?;
        }

        let added = self.to - self.from;
        if added.contains(Modifier::REVERSED) {
            queue!(w, SetAttribute(CAttribute::Reverse))?;
        }
        if added.contains(Modifier::BOLD) {
            queue!(w, SetAttribute(CAttribute::Bold))?;
        }
        if added.contains(Modifier::ITALIC) {
            queue!(w, SetAttribute(CAttribute::Italic))?;
        }
        if added.contains(Modifier::UNDERLINED) {
            queue!(w, SetAttribute(CAttribute::Underlined))?;
        }
        if added.contains(Modifier::DIM) {
            queue!(w, SetAttribute(CAttribute::Dim))?;
        }
        if added.contains(Modifier::CROSSED_OUT) {
            queue!(w, SetAttribute(CAttribute::CrossedOut))?;
        }
        if added.contains(Modifier::SLOW_BLINK) {
            queue!(w, SetAttribute(CAttribute::SlowBlink))?;
        }
        if added.contains(Modifier::RAPID_BLINK) {
            queue!(w, SetAttribute(CAttribute::RapidBlink))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
