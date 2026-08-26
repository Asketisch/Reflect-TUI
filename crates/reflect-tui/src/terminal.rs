//! 基于 Reflect 的 `custom_terminal::Terminal` 构建的终端生命周期 RAII。
//!
//! 镜像 Reflect 的 "inline viewport"（行内视口）模型:我们停留在 **主屏**
//! (不使用备用屏),以保留终端的原生 scrollback。已提交
//! 的历史行通过 `insert_history_lines` 推入原生 scrollback,
//! 用户可以使用终端自身的 scrollback 控件(鼠标滚轮、PgUp 等)查看。
//! 每帧只将活动区、composer 和状态行渲染到屏幕底部
//! 锚定的小视口。
//!
//! 启用 Bracketed paste(`?2004h`),使粘贴文本以 `Event::Paste` 形式到达。
//! 故意**不**启用 Alternate scroll(`?1007h`):启用后,
//! 终端会把滚轮/触控板滚动转换为 Up/Down 方向键事件,
//! 而 composer 会用它进行输入历史导航。不启用时,
//! 终端直接滚动其原生 scrollback ——这正是我们
//! 审查已提交历史时想要的行为。

use crate::terminal_probe::{self, DefaultColors};
use crate::tui_core::custom_terminal::Terminal as ReflectTerminal;
use crate::tui_core::terminal_palette;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Color as CrosstermColor, query_background_color, query_foreground_color};
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};

pub type Backend = CrosstermBackend<Stdout>;
pub type Terminal = ReflectTerminal<Backend>;

pub struct TerminalGuard {
    terminal: Terminal,
    /// 是否已进入备用屏（Ctrl+T transcript overlay）。
    alt_screen: bool,
    /// 是否全屏模式：启动即进备用屏且不再退回主屏（overlay 与之共享同一屏）。
    fullscreen: bool,
}

impl TerminalGuard {
    /// 初始化终端。`fullscreen=true` 时直接进入备用屏（替代 inline viewport 的
    /// 主屏 + 原生 scrollback 模型，避免 shrink 时空行污染 scrollback）。
    pub fn enter(fullscreen: bool) -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        // 主屏 + bracketed paste；不启 alternate scroll（与 Reflect 主界面一致）。
        crossterm::execute!(stdout, crossterm::event::EnableBracketedPaste)?;

        // 启动时探测默认前景/背景色，供 user_message_style / TextArea 可见性使用。
        probe_and_cache_default_colors();

        let terminal = Terminal::with_options(CrosstermBackend::new(stdout))?;
        // 预热 palette 缓存（若 probe 已写入则直接命中）。
        let _ = terminal_palette::default_colors();
        let mut guard = Self {
            terminal,
            alt_screen: false,
            fullscreen,
        };
        if fullscreen {
            // 全屏模式启动即进备用屏，后续 overlay 共享此屏。
            let _ = guard.enter_alt_screen();
        }
        Ok(guard)
    }

    pub fn terminal(&mut self) -> &mut Terminal {
        &mut self.terminal
    }

    /// 进入备用屏（transcript overlay）。启用 alternate scroll 以便在 overlay 内滚轮翻页。
    pub fn enter_alt_screen(&mut self) -> io::Result<()> {
        if self.alt_screen {
            return Ok(());
        }
        crossterm::execute!(
            self.terminal.backend_mut(),
            crossterm::terminal::EnterAlternateScreen,
        )?;
        // Overlay 内滚轮 → 方向键，供 pager 消费。
        let _ = write_osc(self.terminal.backend_mut(), "\x1b[?1007h");
        self.alt_screen = true;
        let size = self.terminal.size()?;
        self.terminal
            .set_viewport_area(ratatui::layout::Rect::new(0, 0, size.width, size.height));
        self.terminal.clear()?;
        Ok(())
    }

    /// 离开备用屏，回到主屏 inline viewport。
    /// 真正离开备用屏（全屏模式退出 TUI 时也调用此内部方法恢复主屏）。
    fn leave_alt_screen_inner(&mut self) -> io::Result<()> {
        if !self.alt_screen {
            return Ok(());
        }
        let _ = write_osc(self.terminal.backend_mut(), "\x1b[?1007l");
        crossterm::execute!(
            self.terminal.backend_mut(),
            crossterm::terminal::LeaveAlternateScreen,
        )?;
        self.alt_screen = false;
        Ok(())
    }

    pub fn leave_alt_screen(&mut self) -> io::Result<()> {
        // 全屏模式下不退出备用屏：overlay 与之共享同一屏，关闭 overlay 只是清除
        // 状态并回到全屏主界面，不应真正 LeaveAlternateScreen。
        if self.fullscreen {
            return Ok(());
        }
        self.leave_alt_screen_inner()
    }

    pub fn is_alt_screen(&self) -> bool {
        self.alt_screen
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // 退出时始终真正离开备用屏（全屏模式也恢复主屏，避免终端残留）。
        let _ = self.leave_alt_screen_inner();
        let _ = self.terminal.show_cursor();
        let _ = crossterm::execute!(
            self.terminal.backend_mut(),
            crossterm::event::DisableBracketedPaste,
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// 写入 OSC 序列(如 `ESC]0;titleBEL`)。
fn write_osc(w: &mut impl io::Write, seq: &str) -> io::Result<()> {
    w.write_all(seq.as_bytes())?;
    w.flush()
}

/// 设置终端标题(OSC 0 `ESC]0;titleBEL`)。
///
/// 对齐老 TUI `terminal_title.rs` 的行为:对特殊字符做清洗(去除 ESC +
/// 控制字符,保留可打印字符),截断到 80 字符,避免溢出导致终端异常。
///
/// 在 `tui/mod.rs` 启动期 + 状态变化(busy 切换/回合完成/审批)时调用,
/// 让终端标签页始终显示当前上下文(`⠹ model · git:(branch) · ...`)。
/// 对不支持 OSC 0 的终端(iTerm/tmux/Kitty 均支持)是静默无操作。
pub fn set_terminal_title(title: &str) {
    // 对特殊字符做简单清洗:只保留可打印字符,去除 ESC。
    let sanitized: String = title
        .chars()
        .filter(|c| u32::from(*c) >= 0x20)
        .filter(|c| *c != '\x1b')
        .take(80)
        .collect();
    let _ = write_osc(&mut std::io::stdout(), &format!("\x1b]0;{}\x07", sanitized));
}

#[cfg(test)]
mod terminal_title_tests {
    use super::*;

    #[test]
    fn terminal_title_sanitizes_esc_and_control_chars() {
        // 测试 sanitized 字符串逻辑:ESC 和控制字符被移除。
        let title = "Reflect · test\x1b\x00\x1f";
        let sanitized: String = title
            .chars()
            .filter(|c| u32::from(*c) >= 0x20)
            .filter(|c| *c != '\x1b')
            .take(80)
            .collect();
        assert!(!sanitized.contains('\x1b'), "ESC should be removed");
        assert!(!sanitized.contains('\x00'), "NUL should be removed");
        assert!(
            !sanitized.contains('\x1f'),
            "Control char should be removed"
        );
        assert_eq!(sanitized, "Reflect · test");
    }

    #[test]
    fn terminal_title_truncates_long_title() {
        let title = "a".repeat(100);
        let sanitized: String = title
            .chars()
            .filter(|c| u32::from(*c) >= 0x20)
            .filter(|c| *c != '\x1b')
            .take(80)
            .collect();
        assert_eq!(sanitized.len(), 80, "should be truncated to 80 chars");
    }
}

/// 通过 crossterm OSC 查询写入 palette，并同步到 terminal_probe 形状。
fn probe_and_cache_default_colors() {
    if !should_probe_default_colors() {
        return;
    }
    let fg = query_foreground_color()
        .ok()
        .flatten()
        .and_then(crossterm_rgb);
    let bg = query_background_color()
        .ok()
        .flatten()
        .and_then(crossterm_rgb);
    let colors = fg.zip(bg).map(|(fg, bg)| DefaultColors { fg, bg });
    terminal_palette::set_default_colors_from_startup_probe(colors);
    // 若 OSC 不可用，尝试 terminal_probe stub 路径（当前恒为 None，保留扩展点）。
    if colors.is_none() {
        if let Ok(Some(c)) = terminal_probe::default_colors(terminal_probe::DEFAULT_TIMEOUT) {
            terminal_palette::set_default_colors_from_startup_probe(Some(c));
        }
    }
}

/// 启动期是否探测终端默认前景/背景色（OSC 10/11 查询）。
///
/// crossterm fork 的颜色查询对无响应终端**每次固定等 2s 超时**，两个 slot
/// 共 4s —— macOS Terminal.app、无透传的 tmux/screen、CI/测试用的哑 pty
/// 都不应答，直接表现为 TUI 启动黑屏数秒（叠加 CPR 光标查询可达 6s）。
/// 因此只在已知会应答 OSC 颜色查询的终端上探测；探测失败本就有安全的
/// 样式兜底，跳过只是少了 fg/bg 精确配色。
///
/// `REFLECT_TUI_COLOR_PROBE=0` 强制关闭，`=1` 强制开启。
fn should_probe_default_colors() -> bool {
    if let Ok(v) = std::env::var("REFLECT_TUI_COLOR_PROBE") {
        return v != "0";
    }
    use crate::tui_core::terminal_detection::{terminal_info, TerminalName};
    matches!(
        terminal_info().name,
        TerminalName::ITerm2
            | TerminalName::WezTerm
            | TerminalName::Kitty
            | TerminalName::Alacritty
            | TerminalName::Ghostty
            | TerminalName::Rio
            | TerminalName::Foot
            | TerminalName::Konsole
            | TerminalName::Contour
            | TerminalName::Warp
            | TerminalName::VSCode
            | TerminalName::VSCodeInsiders
            | TerminalName::VSCodium
            | TerminalName::Windsurf
            | TerminalName::Cursor
    )
}

fn crossterm_rgb(color: CrosstermColor) -> Option<(u8, u8, u8)> {
    match color {
        CrosstermColor::Rgb { r, g, b } => Some((r, g, b)),
        _ => None,
    }
}

pub fn poll_event(timeout: std::time::Duration) -> anyhow::Result<Option<Event>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }
    Ok(Some(event::read()?))
}

pub fn is_exit_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL)
}
