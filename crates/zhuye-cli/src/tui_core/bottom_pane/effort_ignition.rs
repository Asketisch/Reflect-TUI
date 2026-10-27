//! 当活跃推理投入档位切换为 Max 或 Ultra 时，在输入框区域内展示的一次性
//! 庆祝动画，以及该档位生效期间一直保留的提示强调效果。
//!
//! Wave、Aurora 和 Pulse 对文本安全：背景混合在草稿下方，字形只落在空单元格中。
//! 真实的档位切换会随机选择样式，且不会重复上一次的样式。
//!
//! 时钟从首个渲染帧开始计时。浏览器后端以及不支持可靠 ANSI-256/truecolor
//! 的原生终端、以及关闭了动画的会话，都会保持静态。

use crate::random_range_compat::*;
use std::cell::Cell;
use std::time::Duration;
use std::time::Instant;

use crate::protocol_compat::openai_models::ReasoningEffort;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;

use crate::tui_core::color::blend;
use crate::tui_core::color::is_light;
use crate::tui_core::style::user_message_bg_rgb;
use crate::tui_core::terminal_palette::StdoutColorLevel;
use crate::tui_core::terminal_palette::best_color_for_level;
use crate::tui_core::terminal_palette::default_bg;
use crate::tui_core::terminal_palette::default_fg;
use crate::tui_core::terminal_palette::effective_stdout_color_level;

#[path = "effort_ignition_styles.rs"]
mod styles;

use styles::Canvas;
use styles::paint_style;

const PROMPT_ACCENT_ALPHA: f32 = 0.86;
const CHARGE: Duration = Duration::from_millis(150);

pub(crate) const IGNITION_FRAME_TICK: Duration = Duration::from_millis(33);

pub(crate) fn effort_animation_enabled(
    animations_enabled: bool,
    color_level: StdoutColorLevel,
) -> bool {
    animations_enabled
        && matches!(
            color_level,
            StdoutColorLevel::TrueColor | StdoutColorLevel::Ansi256
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EffortTier {
    Max,
    Ultra,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IgnitionStyle {
    Wave,
    Aurora,
    Pulse,
}

impl IgnitionStyle {
    const ALL: [Self; 3] = [Self::Wave, Self::Aurora, Self::Pulse];

    pub(crate) fn random(previous: Option<Self>) -> Self {
        let mut rng = rand::thread_rng();
        loop {
            let style = Self::ALL[rng.random_range(0..Self::ALL.len())];
            if Some(style) != previous {
                return style;
            }
        }
    }

    fn total_duration(self, tier: EffortTier) -> Duration {
        let (max, ultra) = match self {
            Self::Wave => (1000, 1300),
            Self::Aurora => (1300, 1600),
            Self::Pulse => (900, 1250),
        };
        Duration::from_millis(match tier {
            EffortTier::Max => max,
            EffortTier::Ultra => ultra,
        })
    }
}

impl EffortTier {
    pub(crate) fn from_effort(effort: Option<&ReasoningEffort>) -> Option<Self> {
        match effort {
            Some(ReasoningEffort::Max) => Some(Self::Max),
            Some(ReasoningEffort::Ultra) => Some(Self::Ultra),
            _ => None,
        }
    }

    fn prompt_glyph(self) -> &'static str {
        match self {
            Self::Max => "›",
            Self::Ultra => "»",
        }
    }

    pub(super) fn hues(self, on_light_bg: bool) -> [(u8, u8, u8); 3] {
        match (self, on_light_bg) {
            (Self::Max, false) => [(255, 178, 66), (255, 214, 120), (255, 120, 60)],
            (Self::Max, true) => [(176, 98, 0), (150, 110, 0), (200, 70, 20)],
            (Self::Ultra, false) => [(186, 130, 255), (255, 120, 220), (120, 170, 255)],
            (Self::Ultra, true) => [(124, 58, 217), (190, 40, 150), (30, 100, 220)],
        }
    }

    pub(super) fn accent_rgb(self, on_light_bg: bool) -> (u8, u8, u8) {
        self.hues(on_light_bg)[0]
    }

    fn accent_fallback(self) -> Color {
        match self {
            Self::Max => Color::Yellow,
            Self::Ultra => Color::Magenta,
        }
    }

    pub(crate) fn prompt(self, charge: f32) -> Span<'static> {
        self.prompt_for(
            charge,
            default_fg(),
            default_bg(),
            effective_stdout_color_level(),
        )
    }

    fn prompt_for(
        self,
        charge: f32,
        terminal_fg: Option<(u8, u8, u8)>,
        terminal_bg: Option<(u8, u8, u8)>,
        color_level: StdoutColorLevel,
    ) -> Span<'static> {
        let color = self.accent_color_for(charge, terminal_fg, terminal_bg, color_level);
        let mut style = Style::default().add_modifier(Modifier::BOLD);
        if let Some(color) = color {
            style = style.fg(color);
        }
        Span::styled(self.prompt_glyph(), style)
    }

    fn accent_color_for(
        self,
        charge: f32,
        terminal_fg: Option<(u8, u8, u8)>,
        terminal_bg: Option<(u8, u8, u8)>,
        color_level: StdoutColorLevel,
    ) -> Option<Color> {
        let on_light_bg = terminal_bg.is_some_and(is_light);
        let accent = self.accent_rgb(on_light_bg);
        let target = match terminal_fg {
            Some(fg) => blend(accent, fg, charge.clamp(0.0, 1.0) * PROMPT_ACCENT_ALPHA),
            None => accent,
        };
        match color_level {
            StdoutColorLevel::TrueColor | StdoutColorLevel::Ansi256 => {
                Some(best_color_for_level(target, color_level))
            }
            StdoutColorLevel::Ansi16 => Some(self.accent_fallback()),
            StdoutColorLevel::Unknown => None,
        }
    }
}

pub(crate) struct EffortIgnition {
    tier: EffortTier,
    style: IgnitionStyle,
    started_at: Cell<Option<Instant>>,
    cancelled: Cell<bool>,
}

impl EffortIgnition {
    pub(crate) fn new(tier: EffortTier, style: IgnitionStyle) -> Self {
        Self {
            tier,
            style,
            started_at: Cell::new(/*value*/ None),
            cancelled: Cell::new(/*value*/ false),
        }
    }

    fn elapsed(&self) -> Option<Duration> {
        self.started_at.get().map(|started| started.elapsed())
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.cancelled.get()
            || self
                .elapsed()
                .is_some_and(|elapsed| elapsed >= self.style.total_duration(self.tier))
    }

    pub(crate) fn charge_alpha(&self) -> f32 {
        match self.elapsed() {
            Some(elapsed) => (elapsed.as_secs_f32() / CHARGE.as_secs_f32()).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    pub(crate) fn render(&self, area: Rect, protected: Rect, buf: &mut Buffer) -> bool {
        if area.is_empty() {
            return false;
        }
        let color_level = effective_stdout_color_level();
        if !effort_animation_enabled(/*animations_enabled*/ true, color_level) {
            self.cancelled.set(/*val*/ true);
            return false;
        }
        let Some(term_bg) = default_bg() else {
            self.cancelled.set(/*val*/ true);
            return false;
        };
        let elapsed = match self.started_at.get() {
            Some(started) => started.elapsed(),
            None => {
                self.started_at.set(Some(Instant::now()));
                Duration::ZERO
            }
        };
        let mut canvas = Canvas {
            area,
            protected,
            buf,
            band_rgb: user_message_bg_rgb(term_bg),
            color_level,
        };
        paint_style(
            self.tier,
            self.style,
            elapsed,
            self.tier.hues(is_light(term_bg)),
            &mut canvas,
        );
        true
    }
}

#[cfg(test)]
#[path = "effort_ignition_tests.rs"]
mod tests;
