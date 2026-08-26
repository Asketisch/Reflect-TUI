//! 检测当前终端模拟器与多路复用器。
//!
//! 检测优先级适配本移植版更简单的
//! 枚举形状（本移植版的 `Multiplexer` 是不带版本字段的 unit 枚举；`TerminalName`
//! 是超集，对部分终端保留两种拼写）。检测顺序：
//! 1. `TERM_PROGRAM`（+ `TERM_PROGRAM_VERSION`）— 带 tmux 透传。
//! 2. 终端特定的环境变量（`WEZTERM_VERSION`、`ITERM_*`、`TERM_SESSION_ID`、
//!    `KITTY_WINDOW_ID`、`ALACRITTY_SOCKET`、`KONSOLE_VERSION`、
//!    `GNOME_TERMINAL_SCREEN`、`VTE_VERSION`、`WT_SESSION`）。
//! 3. `TERM` 功能字符串兜底。
//! 4. `Unknown`。
//!
//! 多路复用器检测：`TMUX`/`TMUX_PANE` → Tmux；`ZELLIJ*` → Zellij。

use std::sync::OnceLock;

/// Reflect 能识别的终端模拟器名称。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalName {
    #[default]
    Unknown,
    AppleTerminal,
    WindowsTerminal,
    ITerm2,
    WezTerm,
    Kitty,
    Alacritty,
    Ghostty,
    Rio,
    Foot,
    Konsole,
    XfceTerminal,
    Screen,
    Tmux,
    Zellij,
    Contour,
    VSCode,
    VSCodeInsiders,
    VSCodium,
    Windsurf,
    Cursor,
    Warp,
    Tabby,
    BlackBox,
    Guake,
    Terminator,
    XTerm,
    Rxvt,
    Terminology,
    Nushell,
    Other,
    GnomeTerminal,
    Dumb,

    GhosttyTerminal,
    Iterm2,
    WarpTerminal,
    VsCode,
    Vte,
}

/// 在终端会话内运行的多路复用器。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Multiplexer {
    #[default]
    None,
    Tmux,
    Zellij,
    Screen,
    Byobu,
}

/// 终端环境信息。
#[derive(Clone, Debug, Default)]
pub struct TerminalInfo {
    pub name: TerminalName,
    pub multiplexer: Option<Multiplexer>,
    pub is_legacy: bool,
    pub is_ci: bool,
    pub term: Option<String>,
    pub term_program: Option<String>,
    pub version: Option<String>,
}

impl TerminalInfo {
    pub fn is_zellij(&self) -> bool {
        matches!(self.multiplexer, Some(Multiplexer::Zellij))
    }

    pub fn is_tmux(&self) -> bool {
        matches!(self.multiplexer, Some(Multiplexer::Tmux))
    }
}

static TERMINAL_INFO: OnceLock<TerminalInfo> = OnceLock::new();

/// 通过检查环境变量返回终端信息。
///
/// 结果在进程生命周期内被缓存。
pub fn terminal_info() -> TerminalInfo {
    TERMINAL_INFO
        .get_or_init(detect_terminal_info_from_env)
        .clone()
}

/// TUI 发起 HTTP 请求时使用的 User-Agent 字符串。
pub fn user_agent() -> String {
    "Reflect-TUI".to_string()
}

/// 通过环境变量检测终端名称。
pub fn detect_terminal_name() -> TerminalName {
    terminal_info().name
}

fn detect_terminal_info_from_env() -> TerminalInfo {
    let multiplexer = detect_multiplexer();

    if let Some(term_program) = env_non_empty("TERM_PROGRAM") {
        let version = env_non_empty("TERM_PROGRAM_VERSION");
        let name = terminal_name_from_term_program(&term_program);
        return TerminalInfo {
            name,
            term_program: Some(term_program),
            version,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("WEZTERM_VERSION") {
        let version = env_non_empty("WEZTERM_VERSION");
        return TerminalInfo {
            name: TerminalName::WezTerm,
            version,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("ITERM_SESSION_ID") || env_has("ITERM_PROFILE") || env_has("ITERM_PROFILE_NAME") {
        return TerminalInfo {
            name: TerminalName::Iterm2,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("TERM_SESSION_ID") {
        return TerminalInfo {
            name: TerminalName::AppleTerminal,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("GHOSTTY_RESOURCES_DIR") {
        let version = env_non_empty("GHOSTTY_VERSION");
        return TerminalInfo {
            name: TerminalName::Ghostty,
            version,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("KITTY_WINDOW_ID")
        || std::env::var("TERM")
            .map(|term| term.contains("kitty"))
            .unwrap_or(false)
    {
        return TerminalInfo {
            name: TerminalName::Kitty,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("ALACRITTY_SOCKET")
        || std::env::var("TERM")
            .map(|term| term == "alacritty")
            .unwrap_or(false)
    {
        return TerminalInfo {
            name: TerminalName::Alacritty,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("KONSOLE_VERSION") {
        let version = env_non_empty("KONSOLE_VERSION");
        return TerminalInfo {
            name: TerminalName::Konsole,
            version,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("GNOME_TERMINAL_SCREEN") {
        return TerminalInfo {
            name: TerminalName::GnomeTerminal,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("VTE_VERSION") {
        let version = env_non_empty("VTE_VERSION");
        return TerminalInfo {
            name: TerminalName::Vte,
            version,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if env_has("WT_SESSION") {
        return TerminalInfo {
            name: TerminalName::WindowsTerminal,
            term: env_non_empty("TERM"),
            multiplexer,
            ..Default::default()
        };
    }

    if let Some(term) = env_non_empty("TERM") {
        let name = if term == "dumb" {
            TerminalName::Dumb
        } else {
            TerminalName::Unknown
        };
        return TerminalInfo {
            name,
            term: Some(term),
            multiplexer,
            ..Default::default()
        };
    }

    TerminalInfo {
        multiplexer,
        ..Default::default()
    }
}

fn detect_multiplexer() -> Option<Multiplexer> {
    if env_has_non_empty("TMUX") || env_has_non_empty("TMUX_PANE") {
        return Some(Multiplexer::Tmux);
    }
    if env_has_non_empty("ZELLIJ")
        || env_has_non_empty("ZELLIJ_SESSION_NAME")
        || env_has_non_empty("ZELLIJ_VERSION")
    {
        return Some(Multiplexer::Zellij);
    }
    None
}

fn terminal_name_from_term_program(term_program: &str) -> TerminalName {
    // 归一化为规范的查找键，以便大小写变体能正确映射。
    let key = term_program.trim().to_ascii_lowercase();
    match key.as_str() {
        "apple_Terminal" | "apple_terminal" | "applesublime" => TerminalName::AppleTerminal,
        "iterm.app" | "iterm2" | "iterm" => TerminalName::Iterm2,
        "warp-wtt" | "warp" => TerminalName::WarpTerminal,
        "vscode" => TerminalName::VsCode,
        "ghostty" => TerminalName::Ghostty,
        "wezterm" => TerminalName::WezTerm,
        "kitty" => TerminalName::Kitty,
        "alacritty" => TerminalName::Alacritty,
        "konsole" => TerminalName::Konsole,
        "gnome-terminal" => TerminalName::GnomeTerminal,
        "rio" => TerminalName::Rio,
        "foot" => TerminalName::Foot,
        "contour" => TerminalName::Contour,
        "tabby" => TerminalName::Tabby,
        "blackbox" => TerminalName::BlackBox,
        _ => TerminalName::Unknown,
    }
}

fn env_has(key: &str) -> bool {
    std::env::var_os(key).is_some()
}

fn env_has_non_empty(key: &str) -> bool {
    std::env::var_os(key)
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

impl Multiplexer {
    pub fn as_ref(&self) -> &'static str {
        match self {
            Multiplexer::Tmux => "tmux",
            Multiplexer::Zellij => "zellij",
            Multiplexer::Screen => "screen",
            Multiplexer::Byobu => "byobu",
            Multiplexer::None => "",
        }
    }
}
