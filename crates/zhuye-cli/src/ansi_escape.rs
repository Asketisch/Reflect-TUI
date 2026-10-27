//! 终端输出里的 ANSI 转义序列处理。
//!
//! `CommandOutput` 聚合的命令输出(`ls --color`、`git diff`、npm 等)普遍携带
//! CSI/OSC 序列。ratatui 按 unicode-width 渲染 cell,把 ESC 计 0 宽丢弃、
//! `[31m` 等参数字节按 1 列字面渲染,直接透传会导致列错位、花屏、颜色泄漏。
//! [`ansi_escape_line`] 把原始行解析成「文本 + 样式 Span」序列:
//! SGR(`ESC[…m`)映射为 ratatui Style,其余 CSI/OSC/控制序列剥离。
//!
//! 仅需处理「单行」:命令输出按 `\n` 切行后逐行进入本函数,因此 CUU/CUD
//! 等跨行光标移动序列没有意义,与 SGR 一样按剥离处理。

use ratatui::prelude::{Color, Line, Span, Style};

/// OSC 内容剥离上限(字符数):超过即放弃剥离按字面渲染,防止截断的超长
/// OSC(命令输出头尾保留场景)把省略标记等正文一并吞掉。
const MAX_OSC_CONTENT_CHARS: usize = 4096;

/// 把可能携带 ANSI 转义序列的一行原始文本解析为 ratatui Line。
///
/// - SGR 序列(`ESC[…m`)→ 对后续文本应用对应样式(前景/背景/加粗/斜体等)
/// - 其余 CSI(`ESC[…` 终字母)、OSC(`ESC]…BEL/ST`)、单字符转义、
///   C0 控制字符(`\r`、`\b` 等)→ 剥离
pub fn ansi_escape_line(text: &str) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut style = Style::default();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if c.is_control() && c != '\t' {
                // C0 控制字符(\r 回车覆盖、\b 退格等)无法在行模型里表达,剥离。
                continue;
            }
            plain.push(c);
            continue;
        }
        // 遇到转义序列:先把已累积文本按当前样式落成 span,再消费序列
        // (否则序列前后的文本混在同一个 buffer 里,只能拿到最终样式)。
        if !plain.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut plain), style));
        }
        // ESC:按下一字符分流。
        match chars.next() {
            Some('[') => {
                // CSI:参数字节 0x30–0x3F,中间字节 0x20–0x2F,终止字节 0x40–0x7E。
                let mut params = String::new();
                let mut intermediates = String::new();
                let terminator = loop {
                    match chars.peek() {
                        Some(&b) if ('\x40'..='\x7e').contains(&b) => {
                            chars.next();
                            break b;
                        }
                        Some(&b) if ('\x20'..='\x2f').contains(&b) => {
                            intermediates.push(b);
                            chars.next();
                        }
                        Some(&b) if ('\x30'..='\x3f').contains(&b) => {
                            params.push(b);
                            chars.next();
                        }
                        _ => break '\x00', // 序列被截断:丢弃已消费部分
                    }
                };
                if terminator == 'm' {
                    // 解析失败(截断/参数非法)不影响其余部分:返回 None 则维持原样式。
                    if let Some(sgrs) = parse_sgr_params(&params) {
                        for sgr in sgrs {
                            apply_sgr(&mut style, sgr);
                        }
                    }
                }
                // 非 SGR 的 CSI(CUU、ED 等)对单行模型无意义,剥离。
            }
            Some(']') => {
                // OSC:到 BEL(\x07)或 ST(ESC \)为止全部剥离。
                // 超长保护:命令输出截断(头尾保留 + 省略标记)可能把 OSC 的
                // 终止符留在省略段里,若无上限会把「… bytes omitted …」一并
                // 吞掉。内容超过上限即终止剥离 —— 已消费的 OSC 头部丢弃,
                // 其后的剩余内容(含省略标记)按字面渲染。
                let mut terminated = false;
                let mut scanned = 0usize;
                while let Some(&b) = chars.peek() {
                    if scanned >= MAX_OSC_CONTENT_CHARS {
                        break;
                    }
                    chars.next();
                    scanned += 1;
                    if b == '\x07' {
                        terminated = true;
                        break;
                    }
                    if b == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        terminated = true;
                        break;
                    }
                }
                if !terminated && scanned >= MAX_OSC_CONTENT_CHARS {
                    continue; // 放弃剥离,恢复外层逐字符解析
                }
                if !terminated {
                    break; // 截断的 OSC 吞掉剩余内容
                }
            }
            Some('(') | Some(')') => {
                // 字符集选择(ESC ( B):再消费一个字符。
                chars.next();
            }
            Some(_) => {
                // 其余单字符转义(ESC 7、ESC M、ESC P 截断…)剥离。
            }
            None => break, // 尾部孤立 ESC:剥离
        }
    }

    if !plain.is_empty() {
        spans.push(Span::styled(plain, style));
    }

    if spans.is_empty() {
        Line::from(Vec::new())
    } else {
        Line::from(spans)
    }
}

/// 一条已解析的 SGR 指令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sgr {
    Reset,
    Bold,
    Dim,
    Italic,
    Underline,
    Reverse,
    NoBold,
    NoItalic,
    NoUnderline,
    NoReverse,
    Fg(Color),
    Bg(Color),
    DefaultFg,
    DefaultBg,
}

/// 解析 CSI m 的参数串为 SGR 序列。空参数等价 `0`(reset)。
fn parse_sgr_params(params: &str) -> Option<Vec<Sgr>> {
    let mut out = Vec::new();
    let mut parts = params.split(';').peekable();
    if params.is_empty() {
        return Some(vec![Sgr::Reset]);
    }
    while let Some(part) = parts.next() {
        // 38/48 的扩展色彩有两种写法:
        //   分号形式 `38;5;N` / `38;2;R;G;B`(后续参数属于本指令,需消费);
        //   冒号形式 `38:5:N` / `38:2:R:G:B` / `38:2::R:G:B`(ITU-T T.416,
        //   全部信息在本 token 内,kitty/vim 风格输出常见)。
        match part {
            "38" | "48" => {
                let color = parse_extended_color(&mut parts)?;
                out.push(if part == "38" {
                    Sgr::Fg(color)
                } else {
                    Sgr::Bg(color)
                });
            }
            _ if part.starts_with("38:") || part.starts_with("48:") => {
                let color = parse_colon_extended_color(part)?;
                out.push(if part.starts_with("38:") {
                    Sgr::Fg(color)
                } else {
                    Sgr::Bg(color)
                });
            }
            _ => {
                let n: u16 = part.parse().ok()?;
                out.extend(parse_simple_sgr(n)?);
            }
        }
    }
    Some(out)
}

/// 解析分号形式扩展色彩的剩余参数(消费 `5;N` 或 `2;R;G;B`)。
fn parse_extended_color(
    parts: &mut std::iter::Peekable<std::str::Split<'_, char>>,
) -> Option<Color> {
    let mode = parts.next()?;
    if mode == "5" {
        let n: u8 = parts.next()?.parse().ok()?;
        Some(Color::Indexed(n))
    } else if mode == "2" {
        let r: u8 = parts.next()?.parse().ok()?;
        let g: u8 = parts.next()?.parse().ok()?;
        let b: u8 = parts.next()?.parse().ok()?;
        Some(Color::Rgb(r, g, b))
    } else {
        None
    }
}

/// 解析冒号形式的扩展色彩 token(`38:5:N` / `38:2:R:G:B` / `38:2::R:G:B`)。
fn parse_colon_extended_color(token: &str) -> Option<Color> {
    let mut it = token.split(':');
    it.next()?; // "38" / "48"
    let mode = it.next()?;
    match mode {
        "5" => {
            let n: u8 = it.next()?.parse().ok()?;
            Some(Color::Indexed(n))
        }
        "2" => {
            let mut nums: Vec<u8> = Vec::new();
            for seg in it {
                if seg.is_empty() {
                    continue; // 颜色空间 ID 留空的 `38:2::R:G:B` 形式
                }
                match seg.parse::<u8>() {
                    Ok(n) => nums.push(n),
                    Err(_) => break, // 非数字后缀(tolerance 位等)忽略
                }
            }
            // T.416 可能带颜色空间 ID 头:取最后 3 个分量作 RGB。
            if nums.len() < 3 {
                return None;
            }
            let n = nums.len();
            Some(Color::Rgb(nums[n - 3], nums[n - 2], nums[n - 1]))
        }
        _ => None,
    }
}

/// 基础 SGR 代码(不含 38/48 扩展色彩)。
fn parse_simple_sgr(n: u16) -> Option<Vec<Sgr>> {
    let sgr = match n {
        0 => Sgr::Reset,
        1 => Sgr::Bold,
        2 => Sgr::Dim,
        3 => Sgr::Italic,
        4 => Sgr::Underline,
        7 => Sgr::Reverse,
        22 => Sgr::NoBold,
        23 => Sgr::NoItalic,
        24 => Sgr::NoUnderline,
        27 => Sgr::NoReverse,
        30..=37 => Sgr::Fg(Color::Indexed((n - 30) as u8)),
        39 => Sgr::DefaultFg,
        40..=47 => Sgr::Bg(Color::Indexed((n - 40) as u8)),
        49 => Sgr::DefaultBg,
        90..=97 => Sgr::Fg(Color::Indexed((n - 90 + 8) as u8)),
        100..=107 => Sgr::Bg(Color::Indexed((n - 100 + 8) as u8)),
        // 未支持的代码(下划线变体 21/51-55 等):忽略而非整体失败。
        _ => return Some(Vec::new()),
    };
    Some(vec![sgr])
}

fn apply_sgr(style: &mut Style, sgr: Sgr) {
    match sgr {
        Sgr::Reset => *style = Style::default(),
        Sgr::Bold => style.add_modifier |= ratatui::prelude::Modifier::BOLD,
        Sgr::Dim => style.add_modifier |= ratatui::prelude::Modifier::DIM,
        Sgr::Italic => style.add_modifier |= ratatui::prelude::Modifier::ITALIC,
        Sgr::Underline => style.add_modifier |= ratatui::prelude::Modifier::UNDERLINED,
        Sgr::Reverse => style.add_modifier |= ratatui::prelude::Modifier::REVERSED,
        Sgr::NoBold => style.add_modifier.remove(ratatui::prelude::Modifier::BOLD),
        Sgr::NoItalic => style
            .add_modifier
            .remove(ratatui::prelude::Modifier::ITALIC),
        Sgr::NoUnderline => style
            .add_modifier
            .remove(ratatui::prelude::Modifier::UNDERLINED),
        Sgr::NoReverse => style
            .add_modifier
            .remove(ratatui::prelude::Modifier::REVERSED),
        Sgr::Fg(c) => style.fg = Some(c),
        Sgr::Bg(c) => style.bg = Some(c),
        Sgr::DefaultFg => style.fg = None,
        Sgr::DefaultBg => style.bg = None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::prelude::Modifier;

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.clone()).collect()
    }

    #[test]
    fn 纯文本原样返回() {
        let line = ansi_escape_line("hello 世界");
        assert_eq!(line_text(&line), "hello 世界");
        assert_eq!(line.spans.len(), 1);
    }

    #[test]
    fn sgr颜色映射为样式() {
        let line = ansi_escape_line("\x1b[31mred\x1b[0m plain");
        assert_eq!(line_text(&line), "red plain");
        assert_eq!(line.spans[0].style.fg, Some(Color::Indexed(1)));
        assert_eq!(line.spans[1].style.fg, None);
    }

    #[test]
    fn 加粗与下划线() {
        let line = ansi_escape_line("\x1b[1;4mbold under\x1b[22;24mnormal");
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(
            line.spans[0]
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        assert!(!line.spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn 非_sgr_csi与osc剥离() {
        let line = ansi_escape_line("a\x1b[2Kb\x1b]0;title\x07c\x1b]8;;url\x1b\\d");
        assert_eq!(line_text(&line), "abcd");
    }

    #[test]
    fn 回车退格等控制字符剥离() {
        let line = ansi_escape_line("50%\r  100%\u{8}");
        assert_eq!(line_text(&line), "50%  100%");
    }

    #[test]
    fn 色256() {
        let line = ansi_escape_line("\x1b[38;5;196mx\x1b[0m");
        assert_eq!(line.spans[0].style.fg, Some(Color::Indexed(196)));
    }

    #[test]
    fn rgb色() {
        let line = ansi_escape_line("\x1b[48;2;1;2;3mx\x1b[0m");
        assert_eq!(line.spans[0].style.bg, Some(Color::Rgb(1, 2, 3)));
    }

    #[test]
    fn 冒号形式扩展色() {
        // ITU-T T.416 冒号形式(kitty/vim 风格)。
        let line = ansi_escape_line("\x1b[38:5:196mx\x1b[0m");
        assert_eq!(line.spans[0].style.fg, Some(Color::Indexed(196)));
        let line = ansi_escape_line("\x1b[48:2:1:2:3mx\x1b[0m");
        assert_eq!(line.spans[0].style.bg, Some(Color::Rgb(1, 2, 3)));
        // 带颜色空间 ID 的留空形式。
        let line = ansi_escape_line("\x1b[38:2::10:20:30mx\x1b[0m");
        assert_eq!(line.spans[0].style.fg, Some(Color::Rgb(10, 20, 30)));
    }

    #[test]
    fn 截断序列不panic() {
        let line = ansi_escape_line("text \x1b[3");
        assert_eq!(line_text(&line), "text ");
        let line = ansi_escape_line("text \x1b");
        assert_eq!(line_text(&line), "text ");
        let line = ansi_escape_line("text \x1b]0;no-terminator");
        assert_eq!(line_text(&line), "text ");
    }

    #[test]
    fn 空行返回空行() {
        let line = ansi_escape_line("\x1b[31m\x1b[0m");
        assert!(line.spans.is_empty());
    }
}
