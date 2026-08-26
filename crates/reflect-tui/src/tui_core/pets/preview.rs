//! `/pets` 侧边面板的共享预览状态模型。
//!
//! 预览面板刻意保持小巧且有状态：选择弹窗同步渲染它，而异步预览加载则从
//! widget 树外部更新此状态。把状态放在基于互斥锁的对象里，可让选择器记住
//! 上一次的预览区域以进行带外图片渲染，而无需其余弹窗机制了解宠物图片。

use std::sync::Arc;
use std::sync::Mutex;

use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use crate::tui_core::render::renderable::Renderable;

#[derive(Debug, Clone, Default)]
pub(crate) struct PetPickerPreviewState {
    inner: Arc<Mutex<PetPickerPreviewInner>>,
}

impl PetPickerPreviewState {
    /// 返回选择器侧边面板的可渲染包装器。
    ///
    /// 该包装器克隆开销低，且刻意与控制器共享内部状态，
    /// 这样选择变更回调就能在不重建弹窗的情况下更新可见的加载/错误/就绪状态。
    pub(crate) fn renderable(&self) -> PetPickerPreviewRenderable {
        PetPickerPreviewRenderable {
            inner: Arc::clone(&self.inner),
        }
    }

    pub(crate) fn set_loading(&self) {
        self.update(|inner| {
            inner.status = PetPickerPreviewStatus::Loading;
        });
    }

    pub(crate) fn set_disabled(&self) {
        self.update(|inner| {
            inner.status = PetPickerPreviewStatus::Disabled;
        });
    }

    pub(crate) fn set_ready(&self) {
        self.update(|inner| {
            inner.status = PetPickerPreviewStatus::Ready;
        });
    }

    pub(crate) fn set_error(&self, message: String) {
        self.update(|inner| {
            inner.status = PetPickerPreviewStatus::Error { message };
        });
    }

    pub(crate) fn clear(&self) {
        self.update(|inner| {
            inner.status = PetPickerPreviewStatus::Hidden;
            inner.last_area = None;
        });
    }

    pub(crate) fn area(&self) -> Option<Rect> {
        self.inner.lock().ok().and_then(|inner| inner.last_area)
    }

    fn update(&self, f: impl FnOnce(&mut PetPickerPreviewInner)) {
        if let Ok(mut inner) = self.inner.lock() {
            f(&mut inner);
        }
    }
}

#[derive(Debug, Default)]
struct PetPickerPreviewInner {
    status: PetPickerPreviewStatus,
    last_area: Option<Rect>,
}

#[derive(Debug, Default)]
enum PetPickerPreviewStatus {
    #[default]
    Hidden,
    Loading,
    Disabled,
    Ready,
    Error {
        message: String,
    },
}

pub(crate) struct PetPickerPreviewRenderable {
    inner: Arc<Mutex<PetPickerPreviewInner>>,
}

impl Renderable for PetPickerPreviewRenderable {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let (title, body) = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            inner.last_area = Some(area);
            match &inner.status {
                PetPickerPreviewStatus::Hidden => return,
                PetPickerPreviewStatus::Loading => ("Loading preview...", None),
                PetPickerPreviewStatus::Disabled => (
                    "Terminal pets disabled",
                    Some("No pet will be shown.".to_string()),
                ),
                PetPickerPreviewStatus::Ready => return,
                PetPickerPreviewStatus::Error { message } => {
                    ("Preview unavailable", Some(message.clone()))
                }
            }
        };

        let text_height = if body.is_some() { 2 } else { 1 };
        let text_area = centered_text_area(area, text_height);
        let mut lines = vec![Line::from(title.bold())];
        if let Some(body) = body {
            lines.push(Line::from(body.dim()));
        }
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .render(text_area, buf);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        4
    }
}

fn centered_text_area(area: Rect, height: u16) -> Rect {
    let height = height.min(area.height);
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(area.x, y, area.width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_text_area_centers_vertically() {
        assert_eq!(
            centered_text_area(
                Rect::new(
                    /*x*/ 5, /*y*/ 10, /*width*/ 20, /*height*/ 8
                ),
                /*height*/ 2
            ),
            Rect::new(
                /*x*/ 5, /*y*/ 13, /*width*/ 20, /*height*/ 2
            )
        );
    }
}
