//! ascii_animation 模块的桩（动画已按迁移计划移除）。
#[derive(Debug, Clone, Default)]
pub struct AsciiAnimation;

impl AsciiAnimation {
    pub fn new(_request_frame: crate::tui_core::tui::FrameRequester) -> Self {
        Self::default()
    }

    pub fn with_variants(
        _variants: Vec<Vec<&'static str>>,
        _request_frame: crate::tui_core::tui::FrameRequester,
    ) -> Self {
        Self::default()
    }

    pub fn pick_random_variant(&self) -> () {}
}

impl AsciiAnimation {
    pub fn current_frame(&self) -> String {
        String::new()
    }
    pub fn schedule_next_frame(&self) -> () {}
}
