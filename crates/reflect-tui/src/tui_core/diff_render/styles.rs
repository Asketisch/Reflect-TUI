//! diff 渲染的样式辅助函数：根据主题/颜色等级派生 add/del/gutter 等 Style。
//! 从 diff_render.rs 抽出，依赖父模块 import 与 theme 子模块的类型/函数。

use super::*;

/// 可测试辅助：从显式背景采样判断 `DiffTheme`。
pub(super) fn diff_theme_for_bg(bg: Option<(u8, u8, u8)>) -> DiffTheme {
    if let Some(rgb) = bg
        && is_light(rgb)
    {
        return DiffTheme::Light;
    }
    DiffTheme::Dark
}

/// 探测终端背景并返回适当的 diff 调色板。
pub(super) fn diff_theme() -> DiffTheme {
    diff_theme_for_bg(default_bg())
}

/// 返回当前终端会话的 [`DiffColorLevel`]。
///
/// 这是环境读取适配器：采样运行时信号（`supports-color` 级别、终端名称、
/// `WT_SESSION` 和 `FORCE_COLOR`）并将它们转发到 [`diff_color_level_for_terminal`]。
///
/// 在薄包装器中保持环境读取使 [`diff_color_level_for_terminal`] 保持纯粹并易于单元测试。
pub(super) fn diff_color_level() -> DiffColorLevel {
    diff_color_level_for_terminal(
        stdout_color_level(),
        terminal_info().name,
        std::env::var_os("WT_SESSION").is_some(),
        has_force_color_override(),
    )
}

/// 返回 `FORCE_COLOR` 是否显式设置。
pub(super) fn has_force_color_override() -> bool {
    std::env::var_os("FORCE_COLOR").is_some()
}

/// 使用 Windows Terminal 特定的 truecolor 提升规则，
/// 将原始 [`StdoutColorLevel`] 映射到 [`DiffColorLevel`]。
///
/// 此辅助函数故意纯粹（无环境访问），所以测试可以通过传递显式输入验证策略表。
///
/// Windows Terminal 完全支持 24 位颜色，但 `supports-color` crate 在那里通常只报告 ANSI-16，
/// 因为没有设置 `COLORTERM` 变量。我们通过两种方法检测 Windows Terminal——通过 `terminal_name`
///（由 `terminal_info()` 从 `WT_SESSION` / `TERM_PROGRAM` 解析）和通过原始 `has_wt_session` 标志。
///
/// 这些信号故意不等价：`terminal_name` 是派生分类，具有 `TERM_PROGRAM` 优先级，
/// 所以 `WT_SESSION` 可以存在而 `terminal_name` 不是 `WindowsTerminal`。
///
/// 当 `WT_SESSION` 存在时，除非设置了 `FORCE_COLOR`，否则我们无条件提升到 truecolor。
/// 这保持 Windows Terminal 默认渲染丰富，同时保留显式 `FORCE_COLOR` 用户意图。
///
/// 在 `WT_SESSION` 之外，仅针对已识别的 `WindowsTerminal` 会话提升 ANSI-16；
/// `Unknown` 保持保守。
pub(super) fn diff_color_level_for_terminal(
    stdout_level: StdoutColorLevel,
    terminal_name: TerminalName,
    has_wt_session: bool,
    has_force_color_override: bool,
) -> DiffColorLevel {
    if has_wt_session && !has_force_color_override {
        return DiffColorLevel::TrueColor;
    }

    let base = match stdout_level {
        StdoutColorLevel::TrueColor => DiffColorLevel::TrueColor,
        StdoutColorLevel::Ansi256 => DiffColorLevel::Ansi256,
        StdoutColorLevel::Ansi16 | StdoutColorLevel::Unknown => DiffColorLevel::Ansi16,
    };

    // 在 `WT_SESSION` 之外，保持现有的 Windows Terminal ANSI-16 会话提升，
    // 这些会话可能支持 truecolor。
    if stdout_level == StdoutColorLevel::Ansi16
        && terminal_name == TerminalName::WindowsTerminal
        && !has_force_color_override
    {
        DiffColorLevel::TrueColor
    } else {
        base
    }
}

// -- 样式辅助函数 ------------------------------------------------------------
//
// 每个 diff 行由三个视觉区域组成，分别渲染：
//
//   ┌──────────┬──────┬──────────────────────────────────────────┐
//   │  gutter   │ 符号 │              内容                       │
//   │ （行号）   │ +/-  │  （纯文本或语法高亮文本）               │
//   └──────────┴──────┴──────────────────────────────────────────┘
//
// 第四个全宽层 `line_bg` 通过 `RtLine::style()` 应用，
// 使背景色从最左列延伸到终端右边缘，包括内容之外的填充。
//
// 在深色终端上，符号和内容共享一个样式（有色前景 + 色调背景），
// gutter 只是变暗。在浅色终端上，符号和内容分开：符号只获得有色前景（无背景，因此行背景
// 显示出来），内容仅依赖行背景；gutter 获得不透明的、更饱和的背景，使行号在柔和的行色调上保持可读。

/// 应用于 `RtLine` 本身（非单个片段）的全宽背景。
/// 上下文行故意不设置背景，以便终端默认值显示出来。
pub(super) fn style_line_bg_for(
    kind: DiffLineType,
    diff_backgrounds: ResolvedDiffBackgrounds,
) -> Style {
    match kind {
        DiffLineType::Insert => diff_backgrounds
            .add
            .map_or_else(Style::default, |bg| Style::default().bg(bg)),
        DiffLineType::Delete => diff_backgrounds
            .del
            .map_or_else(Style::default, |bg| Style::default().bg(bg)),
        DiffLineType::Context => Style::default(),
    }
}

pub(super) fn style_context() -> Style {
    Style::default()
}

pub(super) fn add_line_bg(theme: DiffTheme, color_level: RichDiffColorLevel) -> Color {
    match (theme, color_level) {
        (DiffTheme::Dark, RichDiffColorLevel::TrueColor) => rgb_color(DARK_TC_ADD_LINE_BG_RGB),
        (DiffTheme::Dark, RichDiffColorLevel::Ansi256) => indexed_color(DARK_256_ADD_LINE_BG_IDX),
        (DiffTheme::Light, RichDiffColorLevel::TrueColor) => rgb_color(LIGHT_TC_ADD_LINE_BG_RGB),
        (DiffTheme::Light, RichDiffColorLevel::Ansi256) => indexed_color(LIGHT_256_ADD_LINE_BG_IDX),
    }
}

pub(super) fn del_line_bg(theme: DiffTheme, color_level: RichDiffColorLevel) -> Color {
    match (theme, color_level) {
        (DiffTheme::Dark, RichDiffColorLevel::TrueColor) => rgb_color(DARK_TC_DEL_LINE_BG_RGB),
        (DiffTheme::Dark, RichDiffColorLevel::Ansi256) => indexed_color(DARK_256_DEL_LINE_BG_IDX),
        (DiffTheme::Light, RichDiffColorLevel::TrueColor) => rgb_color(LIGHT_TC_DEL_LINE_BG_RGB),
        (DiffTheme::Light, RichDiffColorLevel::Ansi256) => indexed_color(LIGHT_256_DEL_LINE_BG_IDX),
    }
}

pub(super) fn light_gutter_fg(color_level: DiffColorLevel) -> Color {
    match color_level {
        DiffColorLevel::TrueColor => rgb_color(LIGHT_TC_GUTTER_FG_RGB),
        DiffColorLevel::Ansi256 => indexed_color(LIGHT_256_GUTTER_FG_IDX),
        DiffColorLevel::Ansi16 => Color::Black,
    }
}

pub(super) fn light_add_num_bg(color_level: RichDiffColorLevel) -> Color {
    match color_level {
        RichDiffColorLevel::TrueColor => rgb_color(LIGHT_TC_ADD_NUM_BG_RGB),
        RichDiffColorLevel::Ansi256 => indexed_color(LIGHT_256_ADD_NUM_BG_IDX),
    }
}

pub(super) fn light_del_num_bg(color_level: RichDiffColorLevel) -> Color {
    match color_level {
        RichDiffColorLevel::TrueColor => rgb_color(LIGHT_TC_DEL_NUM_BG_RGB),
        RichDiffColorLevel::Ansi256 => indexed_color(LIGHT_256_DEL_NUM_BG_IDX),
    }
}

/// 行号 gutter 样式。在浅色背景上，gutter 具有不透明的
/// 色调背景，使数字与柔和的行填充形成对比。在
/// 深色背景上，简单的 `DIM` 修饰符就足够了。
pub(super) fn style_gutter_for(
    kind: DiffLineType,
    theme: DiffTheme,
    color_level: DiffColorLevel,
) -> Style {
    match (
        theme,
        kind,
        RichDiffColorLevel::from_diff_color_level(color_level),
    ) {
        (DiffTheme::Light, DiffLineType::Insert, None) => {
            Style::default().fg(light_gutter_fg(color_level))
        }
        (DiffTheme::Light, DiffLineType::Delete, None) => {
            Style::default().fg(light_gutter_fg(color_level))
        }
        (DiffTheme::Light, DiffLineType::Insert, Some(level)) => Style::default()
            .fg(light_gutter_fg(color_level))
            .bg(light_add_num_bg(level)),
        (DiffTheme::Light, DiffLineType::Delete, Some(level)) => Style::default()
            .fg(light_gutter_fg(color_level))
            .bg(light_del_num_bg(level)),
        _ => style_gutter_dim(),
    }
}

/// 插入行的符号字符（`+`）。在深色终端上，它继承
/// 完整内容样式（绿色前景 + 色调背景）。在浅色终端上，它只使用
/// 绿色前景，让行级背景显示出来。
pub(super) fn style_sign_add(
    theme: DiffTheme,
    color_level: DiffColorLevel,
    diff_backgrounds: ResolvedDiffBackgrounds,
) -> Style {
    match theme {
        DiffTheme::Light => Style::default().fg(Color::Green),
        DiffTheme::Dark => style_add(theme, color_level, diff_backgrounds),
    }
}

/// 删除行的符号字符（`-`）。[`style_sign_add`] 的镜像。
pub(super) fn style_sign_del(
    theme: DiffTheme,
    color_level: DiffColorLevel,
    diff_backgrounds: ResolvedDiffBackgrounds,
) -> Style {
    match theme {
        DiffTheme::Light => Style::default().fg(Color::Red),
        DiffTheme::Dark => style_del(theme, color_level, diff_backgrounds),
    }
}

/// 插入行的内容样式（纯文本、非语法高亮文本）。
///
/// ANSI-16 上仅前景色。在高级别上，使用 `diff_backgrounds` 中的
/// 预解析背景——当可用时是主题作用域颜色，否则是硬编码调色板。深色主题添加
/// 显式绿色前景，以便在色调背景上可读；
/// 浅色主题依赖默认（深色）前景与柔和色调的对比。
///
/// 当没有解析背景时（例如主题不定义 diff
/// 作用域且回退调色板为空），样式降级为仅前景色，使行仍然可读。
pub(super) fn style_add(
    theme: DiffTheme,
    color_level: DiffColorLevel,
    diff_backgrounds: ResolvedDiffBackgrounds,
) -> Style {
    match (theme, color_level, diff_backgrounds.add) {
        (_, DiffColorLevel::Ansi16, _) => Style::default().fg(Color::Green),
        (DiffTheme::Light, DiffColorLevel::TrueColor, Some(bg))
        | (DiffTheme::Light, DiffColorLevel::Ansi256, Some(bg)) => Style::default().bg(bg),
        (DiffTheme::Dark, DiffColorLevel::TrueColor, Some(bg))
        | (DiffTheme::Dark, DiffColorLevel::Ansi256, Some(bg)) => {
            Style::default().fg(Color::Green).bg(bg)
        }
        (DiffTheme::Light, DiffColorLevel::TrueColor, None)
        | (DiffTheme::Light, DiffColorLevel::Ansi256, None) => Style::default(),
        (DiffTheme::Dark, DiffColorLevel::TrueColor, None)
        | (DiffTheme::Dark, DiffColorLevel::Ansi256, None) => Style::default().fg(Color::Green),
    }
}

/// 删除行的内容样式（纯文本，非语法高亮文本）。
///
/// [`style_add`] 的镜像，具有红色前景和删除侧
/// 解析的背景。
pub(super) fn style_del(
    theme: DiffTheme,
    color_level: DiffColorLevel,
    diff_backgrounds: ResolvedDiffBackgrounds,
) -> Style {
    match (theme, color_level, diff_backgrounds.del) {
        (_, DiffColorLevel::Ansi16, _) => Style::default().fg(Color::Red),
        (DiffTheme::Light, DiffColorLevel::TrueColor, Some(bg))
        | (DiffTheme::Light, DiffColorLevel::Ansi256, Some(bg)) => Style::default().bg(bg),
        (DiffTheme::Dark, DiffColorLevel::TrueColor, Some(bg))
        | (DiffTheme::Dark, DiffColorLevel::Ansi256, Some(bg)) => {
            Style::default().fg(Color::Red).bg(bg)
        }
        (DiffTheme::Light, DiffColorLevel::TrueColor, None)
        | (DiffTheme::Light, DiffColorLevel::Ansi256, None) => Style::default(),
        (DiffTheme::Dark, DiffColorLevel::TrueColor, None)
        | (DiffTheme::Dark, DiffColorLevel::Ansi256, None) => Style::default().fg(Color::Red),
    }
}

pub(super) fn style_gutter_dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}
