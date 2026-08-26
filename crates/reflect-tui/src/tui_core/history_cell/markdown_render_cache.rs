//! 由已完成的源支持 markdown 历史单元共享的单宽度渲染缓存。

use crate::tui_core::terminal_hyperlinks::HyperlinkLine;
use std::sync::Mutex;
use std::sync::PoisonError;

#[derive(Debug, Default)]
pub(super) struct MarkdownRenderCache {
    pub(super) cached: Mutex<Option<(MarkdownRenderCacheKey, Vec<HyperlinkLine>)>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MarkdownRenderCacheKey {
    pub(super) width: u16,
    pub(super) syntax_theme_revision: u64,
    pub(super) terminal_fg: Option<(u8, u8, u8)>,
    pub(super) terminal_bg: Option<(u8, u8, u8)>,
    pub(super) color_level: crate::tui_core::terminal_palette::StdoutColorLevel,
}

impl MarkdownRenderCache {
    /// 返回已为该宽度和终端渲染状态缓存的行，缓存未命中时执行渲染。
    ///
    /// 仅保留最近一次的条目，因此当宽度、语法主题或终端颜色发生变化时，会替换缓存的渲染结果。
    pub(super) fn render(
        &self,
        width: u16,
        render: impl FnOnce() -> Vec<HyperlinkLine>,
    ) -> Vec<HyperlinkLine> {
        let key = MarkdownRenderCacheKey {
            width,
            syntax_theme_revision: crate::tui_core::render::highlight::syntax_theme_revision(),
            terminal_fg: crate::tui_core::terminal_palette::default_fg(),
            terminal_bg: crate::tui_core::terminal_palette::default_bg(),
            color_level: crate::tui_core::terminal_palette::stdout_color_level(),
        };
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((cached_key, lines)) = cached.as_ref()
            && *cached_key == key
        {
            return lines.clone();
        }

        let lines = render();
        *cached = Some((key, lines.clone()));
        lines
    }
}
