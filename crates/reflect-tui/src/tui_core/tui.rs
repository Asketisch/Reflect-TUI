//! Reflect TUI 壳：FrameRequester + 最小 Tui 类型，供 vendored UI 与生产循环共用。
//!
//! 完整上游 `tui.rs`（pets / notifications / job_control）过重；此处实现
//! FrameRequester 真调度，以及 Overlay 所需的最小 `Tui` 字段，生产路径仍走
//! crate 根 `tui::run_async`。

use std::time::{Duration, Instant};

use futures::Stream;
use ratatui::prelude::{Frame as RatatuiFrame, Rect};
use ratatui::widgets::{Widget, WidgetRef};
use tokio::sync::{broadcast, mpsc};

/// 目标帧间隔（约 60fps）。
pub const TARGET_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// 供 draw 闭包使用的桩帧类型（vendored Overlay 用）。
pub struct TuiFrame<'a> {
    pub area: Rect,
    pub inner: RatatuiFrame<'a>,
    pub buffer: &'a mut ratatui::buffer::Buffer,
}

impl<'a> TuiFrame<'a> {
    pub fn area(&self) -> Rect {
        self.area
    }
    pub fn buffer(&mut self) -> &mut ratatui::buffer::Buffer {
        self.inner.buffer_mut()
    }
    pub fn render_widget_ref<W>(&mut self, widget: &W, area: Rect)
    where
        for<'b> &'b W: WidgetRef,
    {
        widget.render_ref(area, self.inner.buffer_mut());
    }
    pub fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, self.inner.buffer_mut());
    }
}

pub type FrameBuffer<'a> = ratatui::buffer::Buffer;
pub type FrameArea = ratatui::prelude::Rect;

pub trait TuiFrameExt {
    fn buffer_field(&mut self) -> &mut ratatui::buffer::Buffer;
}

impl<'a> TuiFrameExt for TuiFrame<'a> {
    fn buffer_field(&mut self) -> &mut ratatui::buffer::Buffer {
        self.inner.buffer_mut()
    }
}

/// 真 FrameRequester：合并重绘请求到 broadcast，供主循环 `draw_rx.recv()`。
#[derive(Debug, Clone)]
pub struct FrameRequester {
    frame_schedule_tx: mpsc::UnboundedSender<Instant>,
}

impl Default for FrameRequester {
    fn default() -> Self {
        Self::test_dummy()
    }
}

impl FrameRequester {
    /// 创建 requester 并 spawn 合并调度任务。
    pub fn new(draw_tx: broadcast::Sender<()>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(frame_scheduler_loop(rx, draw_tx));
        Self {
            frame_schedule_tx: tx,
        }
    }

    pub fn test_dummy() -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();
        Self {
            frame_schedule_tx: tx,
        }
    }

    pub fn request_frame(&self) {
        let _ = self.frame_schedule_tx.send(Instant::now());
    }

    pub fn schedule_frame(&self) {
        self.request_frame();
    }

    pub fn schedule_frame_in(&self, delay: Duration) {
        let _ = self.frame_schedule_tx.send(Instant::now() + delay);
    }
}

async fn frame_scheduler_loop(
    mut rx: mpsc::UnboundedReceiver<Instant>,
    draw_tx: broadcast::Sender<()>,
) {
    let mut next_deadline: Option<Instant> = None;
    loop {
        match next_deadline {
            None => match rx.recv().await {
                Some(at) => next_deadline = Some(at),
                None => break,
            },
            Some(deadline) => {
                let now = Instant::now();
                if deadline > now {
                    tokio::select! {
                        maybe = rx.recv() => {
                            match maybe {
                                Some(at) => {
                                    next_deadline = Some(match next_deadline {
                                        Some(d) => d.min(at),
                                        None => at,
                                    });
                                }
                                None => break,
                            }
                        }
                        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                            let _ = draw_tx.send(());
                            next_deadline = None;
                            // 合并积压请求。
                            while let Ok(at) = rx.try_recv() {
                                if at <= Instant::now() {
                                    continue;
                                }
                                next_deadline = Some(at);
                            }
                        }
                    }
                } else {
                    let _ = draw_tx.send(());
                    next_deadline = None;
                }
            }
        }
    }
}

/// 最小 Tui 壳（vendored Overlay / ChatWidget 引用）。
#[derive(Debug, Default)]
pub struct Tui {
    pub terminal: TuiTerminal,
    pub terminal_title: String,
    pub composer: Option<String>,
    pub alt_screen_active: bool,
    pub status: String,
    pub chat_widget: Option<String>,
    pub overlay: Option<String>,
    pub backtrack: bool,
    pub approval: bool,
    pub transcript_cells: Vec<String>,
    pub terminal_size: (u16, u16),
    frame_requester: FrameRequester,
}

#[derive(Debug, Default)]
pub struct TuiTerminal {
    pub viewport_area: ratatui::prelude::Rect,
    pub set_viewport_area: ratatui::prelude::Rect,
    pub last_known_screen_size: Option<(u16, u16)>,
    pub last_known_cursor_pos: Option<(u16, u16)>,
}

impl TuiTerminal {
    pub fn clear(&mut self) {}
    pub fn set_cursor_visible(&mut self, _visible: bool) {}
    pub fn restore_cursor(&mut self) {}
}

impl Tui {
    pub fn frame_requester(&mut self) -> FrameRequester {
        self.frame_requester.clone()
    }

    pub fn draw<F>(&mut self, _max_width: u16, _f: F) -> std::io::Result<()>
    where
        F: FnOnce(&mut TuiFrame<'_>),
    {
        Ok(())
    }

    pub fn insert_history_lines(&mut self, _lines: Vec<String>) {}

    pub fn event_stream(&mut self) -> std::pin::Pin<Box<dyn Stream<Item = TuiEvent> + Send>> {
        Box::pin(futures::stream::pending())
    }
    pub fn terminal_mut(&mut self) -> &mut Self {
        self
    }
    pub fn frame_requester_mut(&mut self) -> FrameRequester {
        self.frame_requester.clone()
    }
    pub fn set_terminal_title(&mut self, _title: &str) {}
    pub fn set_alt_screen(&mut self, _enabled: bool) {}
    pub fn restore_alt_screen(&mut self) {}
    pub fn insert_history_lines_with_wrap_policy(&mut self, _lines: Vec<String>, _policy: &str) {}
    pub fn clear(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    pub fn suspend(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    pub fn resume(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    pub fn enter_alt_screen(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    pub fn leave_alt_screen(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    pub fn notify(&mut self, _msg: &str) {}
    pub fn insert_history_hyperlink_lines_with_wrap_policy(
        &mut self,
        _lines: Vec<String>,
        _policy: String,
    ) {
    }
}

#[derive(Debug, Clone)]
pub enum TuiEvent {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
    Paste(String),
    FocusGained,
    FocusLost,
    Tick,
    Frame,
    Draw,
}

impl Default for TuiEvent {
    fn default() -> Self {
        Self::Tick
    }
}
