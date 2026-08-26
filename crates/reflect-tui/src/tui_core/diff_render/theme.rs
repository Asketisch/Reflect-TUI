//! diff 渲染的主题/调色板子系统：truecolor/256-color 调色板、DiffTheme/ColorLevel
//! 分类与背景解析函数。从 diff_render.rs 抽出，便于收敛单文件行数。
//!
//! 依赖父模块的 import（通过 `use super::*;`）与 tui_core::color / terminal_palette。

use super::*;

// -- Diff 背景调色板 --------------------------------------------------------
//
// 深色主题色调足够微妙，不会与语法高亮颜色冲突。
// 浅色主题值匹配 GitHub 的 diff 颜色以保持熟悉感。gutter（行号列）在浅色
// 背景上使用稍饱和的变体，使数字在柔和的行背景上仍然可读。
// Truecolor 调色板。
pub(super) const DARK_TC_ADD_LINE_BG_RGB: (u8, u8, u8) = (33, 58, 43); // #213A2B
pub(super) const DARK_TC_DEL_LINE_BG_RGB: (u8, u8, u8) = (74, 34, 29); // #4A221D
pub(super) const LIGHT_TC_ADD_LINE_BG_RGB: (u8, u8, u8) = (218, 251, 225); // #dafbe1
pub(super) const LIGHT_TC_DEL_LINE_BG_RGB: (u8, u8, u8) = (255, 235, 233); // #ffebe9
pub(super) const LIGHT_TC_ADD_NUM_BG_RGB: (u8, u8, u8) = (172, 238, 187); // #aceebb
pub(super) const LIGHT_TC_DEL_NUM_BG_RGB: (u8, u8, u8) = (255, 206, 203); // #ffcecb
pub(super) const LIGHT_TC_GUTTER_FG_RGB: (u8, u8, u8) = (31, 35, 40); // #1f2328

// 256 色调色板。
pub(super) const DARK_256_ADD_LINE_BG_IDX: u8 = 22;
pub(super) const DARK_256_DEL_LINE_BG_IDX: u8 = 52;
pub(super) const LIGHT_256_ADD_LINE_BG_IDX: u8 = 194;
pub(super) const LIGHT_256_DEL_LINE_BG_IDX: u8 = 224;
pub(super) const LIGHT_256_ADD_NUM_BG_IDX: u8 = 157;
pub(super) const LIGHT_256_DEL_NUM_BG_IDX: u8 = 217;
pub(super) const LIGHT_256_GUTTER_FG_IDX: u8 = 236;

/// 对 gutter 符号渲染和样式选择的 diff 行进行分类。
///
/// `Insert` 用 `+` 符号和绿色文本渲染，`Delete` 用 `-` 和红色文本渲染
///（语法高亮时加暗淡覆盖），`Context` 用空格和默认样式。
#[derive(Clone, Copy)]
pub(crate) enum DiffLineType {
    Insert,
    Delete,
    Context,
}

/// 控制 diff 渲染器使用的颜色调色板，用于背景和 gutter 样式。
///
/// 通过 [`diff_theme`] 在每个 `render_change` 调用中确定一次，它探测
/// 终端查询的背景颜色。当背景无法确定时（在 CI 或管道输出中常见），
/// `Dark` 用作安全的默认值。
#[derive(Clone, Copy, Debug)]
pub(super) enum DiffTheme {
    Dark,
    Light,
}

/// diff 渲染器将针对的调色板深度。
///
/// 这是*渲染器自己的*颜色深度概念，源自但不等同于 `supports-color` 报告的
/// 原始 [`StdoutColorLevel`]。间接层存在是因为一些终端（尤其是 Windows Terminal）
/// 只宣传 ANSI-16 支持，但实际上正确渲染 truecolor 序列；
/// [`diff_color_level_for_terminal`] 提升这些情况，使 diff 输出使用更丰富的调色板。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DiffColorLevel {
    TrueColor,
    Ansi256,
    Ansi16,
}

/// [`DiffColorLevel`] 中支持色调背景的子集。
///
/// ANSI-16 终端以粗体饱和调色板条目渲染背景，会压过语法标记。
/// 此类型编码不变量"我们有足够的颜色深度用于柔和色调"，这样生成背景的辅助函数
/// （`add_line_bg`、`del_line_bg`、`light_add_num_bg`、`light_del_num_bg`）
/// 永远不需要不可达的 ANSI-16 分支。
///
/// 通过 [`RichDiffColorLevel::from_diff_color_level`] 构造，它对 ANSI-16 返回
/// `None`——调用者在 `Option` 上分支，当为 `None` 时完全跳过背景。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RichDiffColorLevel {
    TrueColor,
    Ansi256,
}

impl RichDiffColorLevel {
    /// 提取高级别颜色等级，对 ANSI-16 返回 `None`。
    pub(super) fn from_diff_color_level(level: DiffColorLevel) -> Option<Self> {
        match level {
            DiffColorLevel::TrueColor => Some(Self::TrueColor),
            DiffColorLevel::Ansi256 => Some(Self::Ansi256),
            DiffColorLevel::Ansi16 => None,
        }
    }
}

/// 预解析的插入和删除 diff 行背景颜色。
///
/// 在每个 `render_change` 调用中，从活动语法主题的作用域背景（通过 [`resolve_diff_backgrounds`]）
/// 计算一次，然后传递给每个样式辅助函数，这样单行不需要重新查询主题。
///
/// 当颜色等级为 ANSI-16 时，两个字段都是 `None`——调用者在这种情况下回退为仅前景样式。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ResolvedDiffBackgrounds {
    pub(super) add: Option<Color>,
    pub(super) del: Option<Color>,
}

/// diff 行样式的预计算渲染状态。
///
/// 绑定终端派生的主题和颜色深度以及主题解析的 diff 背景，这样渲染多行的调用者
/// 可以在每次渲染传递中计算一次，并在所有行调用中重用。
#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffRenderStyleContext {
    pub(super) theme: DiffTheme,
    pub(super) color_level: DiffColorLevel,
    pub(super) diff_backgrounds: ResolvedDiffBackgrounds,
}

/// 为生产渲染解析 diff 背景。
///
/// 查询活动语法主题的 `markup.inserted` / `markup.deleted`（和 `diff.*` 回退），
/// 然后委托给 [`resolve_diff_backgrounds_for`]。
pub(super) fn resolve_diff_backgrounds(
    theme: DiffTheme,
    color_level: DiffColorLevel,
) -> ResolvedDiffBackgrounds {
    resolve_diff_backgrounds_for(theme, color_level, diff_scope_background_rgbs())
}

/// 将当前终端环境快照为可重用的样式上下文。
///
/// 一次性查询 `diff_theme`、`diff_color_level` 和活动语法主题的作用域背景，
/// 将它们打包到 [`DiffRenderStyleContext`] 中，调用者在单次传递中的每行渲染调用传递它。
///
/// 在每个渲染帧的顶部调用此函数——而不是每行——这样即使用户在渲染过程中切换主题
///（主题选择器实时预览），diff 调色板在帧内仍然保持一致。
pub(crate) fn current_diff_render_style_context() -> DiffRenderStyleContext {
    let theme = diff_theme();
    let color_level = diff_color_level();
    let diff_backgrounds = resolve_diff_backgrounds(theme, color_level);
    DiffRenderStyleContext {
        theme,
        color_level,
        diff_backgrounds,
    }
}

/// 核心背景解析逻辑，保持纯函数以便于测试。
///
/// 从硬编码的回退调色板开始，然后在 (a) 颜色等级足够丰富且 (b) 主题定义了匹配作用域时，
/// 用主题作用域背景覆盖。这意味着回退调色板始终是基线，主题作用域严格是附加的。
pub(super) fn resolve_diff_backgrounds_for(
    theme: DiffTheme,
    color_level: DiffColorLevel,
    scope_backgrounds: DiffScopeBackgroundRgbs,
) -> ResolvedDiffBackgrounds {
    let mut resolved = fallback_diff_backgrounds(theme, color_level);
    let Some(level) = RichDiffColorLevel::from_diff_color_level(color_level) else {
        return resolved;
    };

    if let Some(rgb) = scope_backgrounds.inserted {
        resolved.add = Some(color_from_rgb_for_level(rgb, level));
    }
    if let Some(rgb) = scope_backgrounds.deleted {
        resolved.del = Some(color_from_rgb_for_level(rgb, level));
    }
    resolved
}

/// 硬编码的调色板背景，用于语法主题未提供 diff 特定作用域背景的情况。
/// 对 ANSI-16 返回空背景。
pub(super) fn fallback_diff_backgrounds(
    theme: DiffTheme,
    color_level: DiffColorLevel,
) -> ResolvedDiffBackgrounds {
    match RichDiffColorLevel::from_diff_color_level(color_level) {
        Some(level) => ResolvedDiffBackgrounds {
            add: Some(add_line_bg(theme, level)),
            del: Some(del_line_bg(theme, level)),
        },
        None => ResolvedDiffBackgrounds::default(),
    }
}

/// 将 RGB 三元组转换为给定高级颜色级别对应的 ratatui `Color`——
/// truecolor 直通，ANSI-256 量化。
pub(super) fn color_from_rgb_for_level(
    rgb: (u8, u8, u8),
    color_level: RichDiffColorLevel,
) -> Color {
    match color_level {
        RichDiffColorLevel::TrueColor => rgb_color(rgb),
        RichDiffColorLevel::Ansi256 => quantize_rgb_to_ansi256(rgb),
    }
}

/// 使用感知距离，查找最接近 `target` 的 ANSI-256 颜色（索引 16–255）。
///
/// 跳过前 16 个条目（系统颜色），因为它们的实际 RGB
/// 值取决于用户终端配置，对于距离计算不可靠。
pub(super) fn quantize_rgb_to_ansi256(target: (u8, u8, u8)) -> Color {
    let best_index = XTERM_COLORS
        .iter()
        .enumerate()
        .skip(16)
        .min_by(|(_, a), (_, b)| {
            perceptual_distance(**a, target).total_cmp(&perceptual_distance(**b, target))
        })
        .map(|(index, _)| index as u8);
    match best_index {
        Some(index) => indexed_color(index),
        None => indexed_color(DARK_256_ADD_LINE_BG_IDX),
    }
}
