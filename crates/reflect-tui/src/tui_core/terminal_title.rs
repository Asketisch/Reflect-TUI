//! TUI 的终端标题输出辅助。
//!
//! 此模块拥有底层 OSC 标题写入路径和发出前
//! 立即进行的净化处理。它的职责刻意保持狭窄：
//! 由调用方决定标题何时应更改，以及空标题意味着
//! "保留旧标题"还是"清除 Reflect 上次写入的标题"。
//! 此模块不尝试读取或恢复终端之前的
//! 标题，因为这无法在终端间移植。
//!
//! 净化是必要的，因为标题内容由不可信的
//! 文本源组装而来，例如模型输出、线程名称、项目路径和配置。
//! 在将该文本放入 OSC 序列之前，我们剥离：
//! - 可能终止或重塑转义序列的控制字符
//! - 可以在视觉上重排或隐藏文本的 bidi/不可见格式化码点
//!   （Trojan Source 文章中讨论的同类问题）
//! - 会使标题嘈杂或难以浏览的冗余空白

use std::fmt;
use std::io;
use std::io::IsTerminal;
use std::io::stdout;

use crossterm::Command;
use ratatui::crossterm::execute;

/// 标题长度的实际上限，以 Rust `char` 计。
///
/// 大多数终端会静默截断超过几百个字符的标题。
/// 240 为 OSC 框架字节留出余量，同时保持标题
/// 在标签栏和窗口管理器中可读。
const MAX_TERMINAL_TITLE_CHARS: usize = 240;

/// [`set_terminal_title`] 调用的结果。
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SetTerminalTitleResult {
    /// 已写入净化后的标题，或 stdout 不是终端因此无需写入。
    Applied,
    /// 净化移除了每个可见字符，因此未发出标题。
    ///
    /// 这与清除标题不同。由调用方决定
    /// 净化后为空的值应导致无操作、清除
    /// Reflect 管理的标题，还是其他回退行为。
    NoVisibleContent,
}

/// 向 stdout 写入净化后的 OSC 窗口标题序列。
///
/// 输入被视为不可信的显示文本：在发出标题之前，
/// 控制字符、不可见格式化字符和冗余空白都会被移除。
/// 如果净化移除了所有可见内容，
/// 函数返回 [`SetTerminalTitleResult::NoVisibleContent`] 而不是
/// 清除标题，因为清除和恢复是高层
/// 调用方的策略决定。在机制上，净化将空白序列
/// 折叠为单个空格、丢弃不允许的码点，并在写入 OSC 0 之前将
/// 结果限制为 [`MAX_TERMINAL_TITLE_CHARS`] 个可见字符。
pub(crate) fn set_terminal_title(title: &str) -> io::Result<SetTerminalTitleResult> {
    if !stdout().is_terminal() {
        return Ok(SetTerminalTitleResult::Applied);
    }

    let title = sanitize_terminal_title(title);
    if title.is_empty() {
        return Ok(SetTerminalTitleResult::NoVisibleContent);
    }

    execute!(stdout(), SetWindowTitle(title))?;
    Ok(SetTerminalTitleResult::Applied)
}

/// 通过写入空 OSC 标题负载清除当前终端标题。
///
/// 这会清除可见标题；它不会恢复 shell 或之前的程序
/// 在 Reflect 开始管理标题之前可能设置的任何标题。
pub(crate) fn clear_terminal_title() -> io::Result<()> {
    if !stdout().is_terminal() {
        return Ok(());
    }

    execute!(stdout(), SetWindowTitle(String::new()))
}

#[derive(Debug, Clone)]
struct SetWindowTitle(String);

impl Command for SetWindowTitle {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        // 匹配 crossterm 的 SetTitle 命令，并用 BEL 终止 OSC 0。
        // 某些终端标题集成会在进程
        // 装饰中暴露 ST 终止符，即使它们另外接受标题更新。
        write!(f, "\x1b]0;{}\x07", self.0)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(std::io::Error::other(
            "tried to execute SetWindowTitle using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

/// 将不可信的标题文本规范化为单个有界显示行。
///
/// 这会移除终端控制字符、剥离不可见/bidi 格式化
/// 字符、将任何空白序列折叠为单个 ASCII 空格，并在
/// [`MAX_TERMINAL_TITLE_CHARS`] 个发出字符之后截断。
fn sanitize_terminal_title(title: &str) -> String {
    let mut sanitized = String::new();
    let mut chars_written = 0;
    let mut pending_space = false;

    for ch in title.chars() {
        if ch.is_whitespace() {
            // 仅在已写入内容时设置待处理空格；这
            // 无需额外 trim 即可剥离前导空白。
            pending_space = !sanitized.is_empty();
            continue;
        }

        if is_disallowed_terminal_title_char(ch) {
            continue;
        }

        if pending_space {
            let remaining = MAX_TERMINAL_TITLE_CHARS.saturating_sub(chars_written);
            if remaining > 1 {
                sanitized.push(' ');
                chars_written += 1;
                pending_space = false;
            }
        }

        if chars_written >= MAX_TERMINAL_TITLE_CHARS {
            break;
        }

        sanitized.push(ch);
        chars_written += 1;
    }

    sanitized
}

/// 返回 `ch` 是否应从终端标题输出中丢弃。
///
/// 这包括普通控制字符和一组精选的不可见
/// 格式化码点。此处的 bidi 条目涵盖 Trojan-Source 风格的
/// 文本重排控制，它们可以使标题相对于其
/// 底层字节序列渲染出误导性内容。
fn is_disallowed_terminal_title_char(ch: char) -> bool {
    if ch.is_control() {
        return true;
    }

    // 剥离与 Trojan-Source 相关的 bidi 控制以及常见的非渲染
    // 格式化字符，使标题文本无法走私终端控制
    // 语义或视觉误导性的内容。
    matches!(
        ch,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{E0100}'..='\u{E01EF}'
    )
}

#[cfg(test)]
mod tests {
    use super::MAX_TERMINAL_TITLE_CHARS;
    use super::SetWindowTitle;
    use super::sanitize_terminal_title;
    use crossterm::Command;
    use pretty_assertions::assert_eq;

    #[test]
    fn sanitizes_terminal_title() {
        let sanitized =
            sanitize_terminal_title("  Project\t|\nWorking\x1b\x07\u{009D}\u{009C} |  Thread  ");
        assert_eq!(sanitized, "Project | Working | Thread");
    }

    #[test]
    fn strips_invisible_format_chars_from_terminal_title() {
        let sanitized = sanitize_terminal_title(
            "Pro\u{202E}j\u{2066}e\u{200F}c\u{061C}t\u{200B} \u{FEFF}T\u{2060}itle",
        );
        assert_eq!(sanitized, "Project Title");
    }

    #[test]
    fn truncates_terminal_title() {
        let input = "a".repeat(MAX_TERMINAL_TITLE_CHARS + 10);
        let sanitized = sanitize_terminal_title(&input);
        assert_eq!(sanitized.len(), MAX_TERMINAL_TITLE_CHARS);
    }

    #[test]
    fn truncation_prefers_visible_char_over_pending_space() {
        let input = format!("{} b", "a".repeat(MAX_TERMINAL_TITLE_CHARS - 1));
        let sanitized = sanitize_terminal_title(&input);
        assert_eq!(sanitized.len(), MAX_TERMINAL_TITLE_CHARS);
        assert_eq!(sanitized.chars().last(), Some('b'));
    }

    #[test]
    fn writes_osc_title_with_bel_terminator() {
        let mut out = String::new();
        SetWindowTitle("hello".to_string())
            .write_ansi(&mut out)
            .expect("encode terminal title");
        assert_eq!(out, "\x1b]0;hello\x07");
    }
}
