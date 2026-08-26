//! 在 Unix / Windows 上提供终端颜色探测。Reflect TUI 目前
//! 不会执行启动期探测;此桩仅用于 `crate::terminal_probe::DefaultColors`
//! 在 `tui_core::terminal_palette` 的跨 crate 导入中可用。

use std::time::Duration;

/// 启动时探测到的默认终端颜色。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DefaultColors {
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
}

/// 终端探测的默认超时。
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_millis(100);

#[cfg(unix)]
mod imp {
    use std::time::Duration;

    use super::DefaultColors;

    pub(crate) fn default_colors(_timeout: Duration) -> std::io::Result<Option<DefaultColors>> {
        Ok(None)
    }

    pub(crate) fn cursor_position(_timeout: Duration) -> std::io::Result<Option<(u16, u16)>> {
        Ok(None)
    }

    pub(crate) struct StartupProbe {
        pub default_colors: Option<DefaultColors>,
        pub cursor_position: Option<(u16, u16)>,
        pub keyboard_enhancement_supported: bool,
    }

    pub(crate) enum StartupKeyboardEnhancementProbe {
        Query,
        Skip,
    }

    pub(crate) fn startup(
        _timeout: Duration,
        _keyboard_probe: StartupKeyboardEnhancementProbe,
    ) -> std::io::Result<StartupProbe> {
        Ok(StartupProbe {
            default_colors: None,
            cursor_position: None,
            keyboard_enhancement_supported: false,
        })
    }
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use super::DefaultColors;

    pub(crate) fn default_colors(_timeout: Duration) -> std::io::Result<Option<DefaultColors>> {
        Ok(None)
    }
}

#[cfg(any(unix, windows))]
pub(crate) use imp::*;
