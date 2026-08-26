//! TUI 语法高亮引擎。
//!
//! 基于 [syntect] 和 [two_face] 语法与主题包，提供约 250 种语言的语法高亮和 32 个内置颜色主题。
//! 该模块持有五个进程级全局单例：
//!
//! | 单例 | 类型 | 用途 |
//! |---|---|---|
//! | `SYNTAX_SET` | `OnceLock<SyntaxSet>` | 语法数据库，初始化后不可变 |
//! | `THEME` | `OnceLock<RwLock<Theme>>` | 活跃颜色主题，运行时可切换 |
//! | `THEME_REVISION` | `AtomicU64` | 主题切换时使已渲染缓存失效 |
//! | `THEME_OVERRIDE` | `OnceLock<Option<String>>` | 持久化用户偏好（仅写入一次） |
//! | `REFLECT_HOME` | `OnceLock<Option<PathBuf>>` | 自定义 `.tmTheme` 文件发现根目录 |
//!
//! **生命周期：**在启动时调用一次 [`set_theme_override`]（在最终配置解析之后）以持久化用户偏好
//! 并初始化 `THEME` 锁。此后，[`set_syntax_theme`] 和 [`current_syntax_theme`] 可用于
//! 实时预览切换/快照主题。所有高亮函数通过 `theme_lock()` 读取主题。
//!
//! **安全限制：**超过 512 KB 或 10 000 行的输入被提前拒绝（返回 `None`），
//! 防止异常的 CPU/内存使用。调用者应降级为纯文本。

use ratatui::style::Color as RtColor;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use syntect::easy::HighlightLines;
use syntect::highlighting::Color as SyntectColor;
use syntect::highlighting::FontStyle;
use syntect::highlighting::Highlighter;
use syntect::highlighting::Style as SyntectStyle;
use syntect::highlighting::Theme;
use syntect::highlighting::ThemeSet;
use syntect::parsing::Scope;
use syntect::parsing::SyntaxReference;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use two_face::theme::EmbeddedThemeName;

// -- 全局单例 ----------------------------------------------------------------

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<RwLock<Theme>> = OnceLock::new();
static THEME_REVISION: AtomicU64 = AtomicU64::new(0);
static THEME_OVERRIDE: OnceLock<Option<String>> = OnceLock::new();
static REFLECT_HOME: OnceLock<Option<PathBuf>> = OnceLock::new();

// Syntect/bat 在 alpha 通道编码 ANSI 调色板语义：
// `a=0` => 通过 RGB 载荷索引 ANSI 调色板，`a=1` => 终端默认。
const ANSI_ALPHA_INDEX: u8 = 0x00;
const ANSI_ALPHA_DEFAULT: u8 = 0x01;
const OPAQUE_ALPHA: u8 = 0xFF;

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(two_face::syntax::extra_newlines)
}

// 注：我们故意不在 ANSI 家族主题（ansi、base16、base16-256）缺少预期的 alpha 通道标记
// 编码时发出运行时诊断。如果上游 two_face/syntect 主题格式变更，
// `ansi_themes_use_only_ansi_palette_colors` 测试会在构建时捕获
// ——远在到达用户之前。运行时警告将是不可行动的噪音，因为用户无法修复上游主题。

/// 设置用户配置的高亮主题覆盖和 Reflect 主路径。
///
/// 使用**最终解析的配置**调用此函数（在引导、恢复和 fork 重新加载完成后）。
/// 首次调用将 `name` 和 `reflect_home` 持久化到 `OnceLock` 中，用于启动/默认主题解析。
///
/// 后续调用无法更改已持久化的 `OnceLock` 值，但仍会立即更新运行时主题以进行实时预览。
///
/// 对于可操作的配置问题返回用户可见的警告，如未知/无效主题名称或重复覆盖持久化。
pub(crate) fn set_theme_override(
    name: Option<String>,
    reflect_home: Option<PathBuf>,
) -> Option<String> {
    let warning = validate_theme_name(name.as_deref(), reflect_home.as_deref());
    let override_set_ok = THEME_OVERRIDE.set(name.clone()).is_ok();
    let reflect_home_set_ok = REFLECT_HOME.set(reflect_home.clone()).is_ok();
    if THEME.get().is_some() {
        set_syntax_theme(resolve_theme_with_override(
            name.as_deref(),
            reflect_home.as_deref(),
        ));
    }
    if !override_set_ok || !reflect_home_set_ok {
        // 实践中这不应该发生——set_theme_override 仅在启动时调用一次。
        // 保留此调试面包屑，以防将来添加第二个调用点。
        tracing::debug!("set_theme_override called more than once; OnceLock values unchanged");
    }
    warning
}

/// 检查主题名称是否解析为内置主题或自定义 `.tmTheme` 文件。
/// 无法解析时返回用户可见的警告。
pub(crate) fn validate_theme_name(
    name: Option<&str>,
    reflect_home: Option<&Path>,
) -> Option<String> {
    let name = name?;
    let custom_theme_path_display = reflect_home
        .map(|home| custom_theme_path(name, home).display().to_string())
        .unwrap_or_else(|| format!("$REFLECT_HOME/themes/{name}.tmTheme"));
    // 内置主题始终可解析。
    if parse_theme_name(name).is_some() {
        return None;
    }
    // 自定义主题必须解析成功；不可读/无效文件仍应在启动时发出警告，以便用户诊断配置问题。
    if let Some(home) = reflect_home {
        let custom_path = custom_theme_path(name, home);
        if custom_path.is_file() {
            if load_custom_theme(name, home).is_some() {
                return None;
            }
            return Some(format!(
                "Custom theme \"{name}\" at {custom_theme_path_display} could not \
                 be loaded (invalid .tmTheme format). Falling back to the default theme."
            ));
        }
    }
    Some(format!(
        "Theme \"{name}\" not found. Using the default theme. \
         To use a custom theme, place a .tmTheme file at \
         {custom_theme_path_display}."
    ))
}

/// 将 kebab-case 主题名称映射到对应的 `EmbeddedThemeName`。
fn parse_theme_name(name: &str) -> Option<EmbeddedThemeName> {
    match name {
        "ansi" => Some(EmbeddedThemeName::Ansi),
        "base16" => Some(EmbeddedThemeName::Base16),
        "base16-eighties-dark" => Some(EmbeddedThemeName::Base16EightiesDark),
        "base16-mocha-dark" => Some(EmbeddedThemeName::Base16MochaDark),
        "base16-ocean-dark" => Some(EmbeddedThemeName::Base16OceanDark),
        "base16-ocean-light" => Some(EmbeddedThemeName::Base16OceanLight),
        "base16-256" => Some(EmbeddedThemeName::Base16_256),
        "catppuccin-frappe" => Some(EmbeddedThemeName::CatppuccinFrappe),
        "catppuccin-latte" => Some(EmbeddedThemeName::CatppuccinLatte),
        "catppuccin-macchiato" => Some(EmbeddedThemeName::CatppuccinMacchiato),
        "catppuccin-mocha" => Some(EmbeddedThemeName::CatppuccinMocha),
        "coldark-cold" => Some(EmbeddedThemeName::ColdarkCold),
        "coldark-dark" => Some(EmbeddedThemeName::ColdarkDark),
        "dark-neon" => Some(EmbeddedThemeName::DarkNeon),
        "dracula" => Some(EmbeddedThemeName::Dracula),
        "github" => Some(EmbeddedThemeName::Github),
        "gruvbox-dark" => Some(EmbeddedThemeName::GruvboxDark),
        "gruvbox-light" => Some(EmbeddedThemeName::GruvboxLight),
        "inspired-github" => Some(EmbeddedThemeName::InspiredGithub),
        "1337" => Some(EmbeddedThemeName::Leet),
        "monokai-extended" => Some(EmbeddedThemeName::MonokaiExtended),
        "monokai-extended-bright" => Some(EmbeddedThemeName::MonokaiExtendedBright),
        "monokai-extended-light" => Some(EmbeddedThemeName::MonokaiExtendedLight),
        "monokai-extended-origin" => Some(EmbeddedThemeName::MonokaiExtendedOrigin),
        "nord" => Some(EmbeddedThemeName::Nord),
        "one-half-dark" => Some(EmbeddedThemeName::OneHalfDark),
        "one-half-light" => Some(EmbeddedThemeName::OneHalfLight),
        "solarized-dark" => Some(EmbeddedThemeName::SolarizedDark),
        "solarized-light" => Some(EmbeddedThemeName::SolarizedLight),
        "sublime-snazzy" => Some(EmbeddedThemeName::SublimeSnazzy),
        "two-dark" => Some(EmbeddedThemeName::TwoDark),
        "zenburn" => Some(EmbeddedThemeName::Zenburn),
        _ => None,
    }
}

/// 构建自定义主题文件的预期路径。
fn custom_theme_path(name: &str, reflect_home: &Path) -> PathBuf {
    reflect_home.join("themes").join(format!("{name}.tmTheme"))
}

/// 尝试从 `{reflect_home}/themes/{name}.tmTheme` 加载自定义 `.tmTheme` 文件。
fn load_custom_theme(name: &str, reflect_home: &Path) -> Option<Theme> {
    ThemeSet::get_theme(custom_theme_path(name, reflect_home)).ok()
}

fn adaptive_default_theme_selection() -> (EmbeddedThemeName, &'static str) {
    match crate::tui_core::terminal_palette::default_bg() {
        Some(bg) if crate::tui_core::color::is_light(bg) => {
            (EmbeddedThemeName::CatppuccinLatte, "catppuccin-latte")
        }
        _ => (EmbeddedThemeName::CatppuccinMocha, "catppuccin-mocha"),
    }
}

fn adaptive_default_embedded_theme_name() -> EmbeddedThemeName {
    adaptive_default_theme_selection().0
}

/// 返回根据终端背景亮度选择的自适应默认语法主题的 kebab-case 名称。
pub(crate) fn adaptive_default_theme_name() -> &'static str {
    adaptive_default_theme_selection().1
}

/// 从当前覆盖/默认主题设置构建主题。
/// 从旧的 `theme()` 初始化闭包中提取以便复用。
fn resolve_theme_with_override(name: Option<&str>, reflect_home: Option<&Path>) -> Theme {
    let ts = two_face::theme::extra();

    // 优先使用用户配置的主题（如果有效）。
    if let Some(name) = name {
        // 1. 尝试按 kebab-case 名称匹配内置主题。
        if let Some(theme_name) = parse_theme_name(name) {
            return ts.get(theme_name).clone();
        }
        // 2. 尝试从磁盘加载 {REFLECT_HOME}/themes/{name}.tmTheme。
        if let Some(home) = reflect_home
            && let Some(theme) = load_custom_theme(name, home)
        {
            return theme;
        }
        tracing::debug!("Theme \"{name}\" not recognized; using default theme");
    }

    ts.get(adaptive_default_embedded_theme_name()).clone()
}

/// 根据当前覆盖/默认主题设置构建主题。
/// 从旧的 `theme()` 初始化闭包中提取，以便可以重用。
fn build_default_theme() -> Theme {
    let name = THEME_OVERRIDE.get().and_then(|name| name.as_deref());
    let reflect_home = REFLECT_HOME
        .get()
        .and_then(|reflect_home| reflect_home.as_deref());
    resolve_theme_with_override(name, reflect_home)
}

fn theme_lock() -> &'static RwLock<Theme> {
    THEME.get_or_init(|| RwLock::new(build_default_theme()))
}

/// 在运行时交换活动语法主题并使渲染内容缓存失效。
pub(crate) fn set_syntax_theme(theme: Theme) {
    let mut guard = match theme_lock().write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = theme;
    THEME_REVISION.fetch_add(1, Ordering::Release);
}

/// 返回活动语法主题的修订版本，用于已渲染内容缓存。
pub(crate) fn syntax_theme_revision() -> u64 {
    THEME_REVISION.load(Ordering::Acquire)
}

/// 克隆当前语法主题（例如保存以用于取消恢复）。
pub(crate) fn current_syntax_theme() -> Theme {
    match theme_lock().read() {
        Ok(theme) => theme.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// 从语法主题 diff/markup 作用域中提取的原始 RGB 背景颜色。
///
/// 这些是主题提供的颜色，尚未适应任何特定的颜色
/// 深度。[`diff_render`](crate::tui_core::diff_render) 在决定是否
/// 输出 truecolor 或量化的 ANSI-256 后，通过 `color_from_rgb_for_level` 将它们转换为 ratatui
/// `Color` 值。
///
/// 当活动主题没有定义相关作用域
/// 背景时，两个字段都是 `None`，此时 diff 渲染器回退到硬编码的
/// 调色板。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DiffScopeBackgroundRgbs {
    pub inserted: Option<(u8, u8, u8)>,
    pub deleted: Option<(u8, u8, u8)>,
}

/// 查询活动语法主题的 diff 作用域背景颜色。
///
/// 优先使用 `markup.inserted` / `markup.deleted`（大多数 VS Code 主题使用的 TextMate 约定），
/// 回退到 `diff.inserted` / `diff.deleted`（一些旧的 `.tmTheme` 文件使用）。
pub(crate) fn diff_scope_background_rgbs() -> DiffScopeBackgroundRgbs {
    let theme = current_syntax_theme();
    diff_scope_background_rgbs_for_theme(&theme)
}

/// 纯提取辅助函数，与全局主题单例分离，以便测试
/// 可以传入任意主题。
fn diff_scope_background_rgbs_for_theme(theme: &Theme) -> DiffScopeBackgroundRgbs {
    let highlighter = Highlighter::new(theme);
    let inserted = scope_background_rgb(&highlighter, "markup.inserted")
        .or_else(|| scope_background_rgb(&highlighter, "diff.inserted"));
    let deleted = scope_background_rgb(&highlighter, "markup.deleted")
        .or_else(|| scope_background_rgb(&highlighter, "diff.deleted"));
    DiffScopeBackgroundRgbs { inserted, deleted }
}

/// 如果已定义，提取单个 TextMate 作用域的背景颜色。
fn scope_background_rgb(highlighter: &Highlighter<'_>, scope_name: &str) -> Option<(u8, u8, u8)> {
    let scope = Scope::new(scope_name).ok()?;
    let bg = highlighter.style_mod_for_stack(&[scope]).background?;
    Some((bg.r, bg.g, bg.b))
}

/// 查询活动语法主题，获取提供的 TextMate 作用域的第一个前景样式。
pub(crate) fn foreground_style_for_scopes(scope_names: &[&str]) -> Option<Style> {
    let theme = current_syntax_theme();
    foreground_style_for_scopes_with_theme(&theme, scope_names)
}

fn foreground_style_for_scopes_with_theme(theme: &Theme, scope_names: &[&str]) -> Option<Style> {
    let highlighter = Highlighter::new(theme);
    scope_names.iter().find_map(|scope_name| {
        let scope = Scope::new(scope_name).ok()?;
        let fg = highlighter.style_mod_for_stack(&[scope]).foreground?;
        convert_syntect_color(fg).map(|fg| Style::default().fg(fg))
    })
}

/// 如果配置解析成功，返回 kebab-case 主题名称；否则
/// 返回自适应自动检测的默认主题名称。
///
/// 这故意反映持久化配置/默认选择，而不是
/// 通过 `set_syntax_theme` 应用的临时运行时交换。
pub(crate) fn configured_theme_name() -> String {
    // 显式的用户覆盖？
    if let Some(Some(name)) = THEME_OVERRIDE.get() {
        if parse_theme_name(name).is_some() {
            return name.clone();
        }
        if let Some(Some(home)) = REFLECT_HOME.get()
            && load_custom_theme(name, home).is_some()
        {
            return name.clone();
        }
    }
    adaptive_default_theme_name().to_string()
}

/// 将主题名称解析为 `Theme`（内置或自定义）。当名称未知且没有匹配的 `.tmTheme` 文件时返回 `None`。
pub(crate) fn resolve_theme_by_name(name: &str, reflect_home: Option<&Path>) -> Option<Theme> {
    let ts = two_face::theme::extra();
    // 内置主题？
    if let Some(embedded) = parse_theme_name(name) {
        return Some(ts.get(embedded).clone());
    }
    // 自定义 .tmTheme 文件？
    if let Some(home) = reflect_home
        && let Some(theme) = load_custom_theme(name, home)
    {
        return Some(theme);
    }
    None
}

/// 选择器中可用的主题，内置或从 `{REFLECT_HOME}/themes/` 下的自定义
/// `.tmTheme` 文件加载。
pub(crate) struct ThemeEntry {
    /// 用于配置持久化和主题解析的 kebab-case 标识符。
    pub name: String,
    /// 当此条目是从磁盘上的 `.tmTheme` 文件发现（而非来自内嵌的 two-face 主题包）时为 `true`。
    pub is_custom: bool,
}

/// 列出所有可用的主题名称：内置主题 + 在 `{reflect_home}/themes/` 中找到的自定义 `.tmTheme` 文件。
pub(crate) fn list_available_themes(reflect_home: Option<&Path>) -> Vec<ThemeEntry> {
    let mut entries: Vec<ThemeEntry> = BUILTIN_THEME_NAMES
        .iter()
        .map(|name| ThemeEntry {
            name: name.to_string(),
            is_custom: false,
        })
        .collect();

    // 发现磁盘上的自定义主题，并与内置主题去重。
    if let Some(home) = reflect_home {
        let themes_dir = home.join("themes");
        if let Ok(read_dir) = std::fs::read_dir(&themes_dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("tmTheme")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    let name = stem.to_string();
                    let is_valid_theme = ThemeSet::get_theme(&path).is_ok();
                    if is_valid_theme && !entries.iter().any(|e| e.name == name) {
                        entries.push(ThemeEntry {
                            name,
                            is_custom: true,
                        });
                    }
                }
            }
        }
    }

    // 将自定义与内置主题一起不区分大小写地排序，
    // 使选择器顺序在不同平台/文件系统间保持稳定。
    entries.sort_by_cached_key(|entry| (entry.name.to_ascii_lowercase(), entry.name.clone()));

    entries
}

/// 所有 32 个内置主题名称，kebab-case，按字母顺序排列。
const BUILTIN_THEME_NAMES: &[&str] = &[
    "1337",
    "ansi",
    "base16",
    "base16-256",
    "base16-eighties-dark",
    "base16-mocha-dark",
    "base16-ocean-dark",
    "base16-ocean-light",
    "catppuccin-frappe",
    "catppuccin-latte",
    "catppuccin-macchiato",
    "catppuccin-mocha",
    "coldark-cold",
    "coldark-dark",
    "dark-neon",
    "dracula",
    "github",
    "gruvbox-dark",
    "gruvbox-light",
    "inspired-github",
    "monokai-extended",
    "monokai-extended-bright",
    "monokai-extended-light",
    "monokai-extended-origin",
    "nord",
    "one-half-dark",
    "one-half-light",
    "solarized-dark",
    "solarized-light",
    "sublime-snazzy",
    "two-dark",
    "zenburn",
];

// -- 样式转换（syntect -> ratatui）--------------------------------------------

/// 将低 ANSI 调色板索引（0–7）映射到 ratatui 的命名颜色变体，
/// 对索引 8–255 回退到 `Indexed(n)`。
///
/// 优先使用命名变体而不是 `Indexed(0)`…`Indexed(7)`，因为许多
/// 终端对命名颜色和索引颜色应用粗体/亮色的处理方式不同，ANSI 主题期望命名行为。
///
/// 这里明确允许 `clippy::disallowed_methods`，因为此辅助函数
/// 故意构造 `ratatui::style::Color::Indexed`。
#[allow(clippy::disallowed_methods)]
fn ansi_palette_color(index: u8) -> RtColor {
    match index {
        0x00 => RtColor::Black,
        0x01 => RtColor::Red,
        0x02 => RtColor::Green,
        0x03 => RtColor::Yellow,
        0x04 => RtColor::Blue,
        0x05 => RtColor::Magenta,
        0x06 => RtColor::Cyan,
        // ANSI 代码 37 表示白色，ratatui 中对应 `Gray`。
        0x07 => RtColor::Gray,
        n => RtColor::Indexed(n),
    }
}

/// 将 syntect 前景 `Color` 解码为 ratatui 颜色，尊重
/// bat 的 `ansi`、`base16` 和 `base16-256` 主题使用的 alpha 通道编码，
/// 用于表示 ANSI 调色板语义而非真正的 RGB。
///
/// 当颜色表示"使用终端默认前景"时返回 `None`，允许调用者完全省略前景属性。
///
/// 传递标准 RGB 主题（alpha 0xFF）的颜色返回 `Some(Rgb(..))`，
/// 因此此函数与非 ANSI 主题向后兼容。意外的中间 alpha 值被视为 RGB。
///
/// `clippy::disallowed_methods` 在此被明确允许，因为此辅助函数
/// 故意构造 `ratatui::style::Color::Rgb`。
#[allow(clippy::disallowed_methods)]
fn convert_syntect_color(color: SyntectColor) -> Option<RtColor> {
    match color.a {
        // `ansi`、`base16` 与 `base16-256` 使用的 bat 兼容编码：
        // alpha 0x00 表示 `r` 存储 ANSI 调色板索引而非 RGB 红色。
        ANSI_ALPHA_INDEX => Some(ansi_palette_color(color.r)),
        // alpha 0x01 表示“使用终端默认前景/背景”。
        ANSI_ALPHA_DEFAULT => None,
        OPAQUE_ALPHA => Some(RtColor::Rgb(color.r, color.g, color.b)),
        // 部分内置主题中出现非 ANSI 的 alpha 值；按普通 RGB 处理。
        _ => Some(RtColor::Rgb(color.r, color.g, color.b)),
    }
}

/// 将 syntect `Style` 转换为 ratatui `Style`。
///
/// 大多数主题产生 RGB 颜色。内置的 `ansi`/`base16`/`base16-256`
/// 主题在 alpha 通道中编码 ANSI 调色板语义，与 bat 匹配。
fn convert_style(syn_style: SyntectStyle) -> Style {
    let mut rt_style = Style::default();

    if let Some(fg) = convert_syntect_color(syn_style.foreground) {
        rt_style = rt_style.fg(fg);
    }
    // 有意跳过背景，避免覆盖终端背景。
    // 若将来支持背景，用 `convert_syntect_color` 解码，
    // 复用与前景相同的 alpha 标记语义。

    if syn_style.font_style.contains(FontStyle::BOLD) {
        rt_style.add_modifier |= Modifier::BOLD;
    }
    // 有意跳过斜体——许多终端对它的渲染很差或完全不渲染。
    // 有意跳过下划线——Dracula 等主题在类型作用域
    // （entity.name.type、support.class）上使用下划线，会在终端输出中的
    // 类型/模块名下方产生分散注意力的下划线。

    rt_style
}

// -- 语法查找 -----------------------------------------------------------------

/// 尝试为给定的语言标识符查找 syntect `SyntaxReference`。
///
/// two-face 的扩展语法集（~250 种语言）直接解析大多数名称和
/// 扩展。我们只修补它无法处理的少数别名。
fn find_syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let ss = syntax_set();

    // two-face 无法自行解析的别名。
    let normalized = lang.to_ascii_lowercase();
    let patched = match normalized.as_str() {
        "csharp" | "c-sharp" => "c#",
        "cppm" | "cxxm" | "ixx" => "cpp",
        "golang" => "go",
        "python3" => "python",
        "shell" => "bash",
        _ => lang,
    };

    // 尝试按 token 匹配（不区分大小写地匹配 file_extensions）。
    if let Some(s) = ss.find_syntax_by_token(patched) {
        return Some(s);
    }
    // 尝试按精确语法名匹配（例如 "Rust"、"Python"）。
    if let Some(s) = ss.find_syntax_by_name(patched) {
        return Some(s);
    }
    // 尝试不区分大小写的名称匹配（例如 "rust" -> "Rust"）。
    let lower = patched.to_ascii_lowercase();
    if let Some(s) = ss
        .syntaxes()
        .iter()
        .find(|s| s.name.to_ascii_lowercase() == lower)
    {
        return Some(s);
    }
    // 尝试把原始输入当作文件扩展名。
    if let Some(s) = ss.find_syntax_by_extension(lang) {
        return Some(s);
    }
    None
}

// -- 防护栏常量 ---------------------------------------------------------------

/// 跳过大于 512 KB 的输入的高亮，以避免过度的内存
/// 和 CPU 使用。调用者回退为纯文本。
const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;

/// 跳过超过 10,000 行的输入的高亮。
const MAX_HIGHLIGHT_LINES: usize = 10_000;

/// 检查输入是否超过安全高亮限制。
///
/// 在循环中高亮内容的调用者（例如每行 diff）应
/// 使用此函数预先检查聚合大小，当返回 `true` 时完全跳过高亮。
pub(crate) fn exceeds_highlight_limits(total_bytes: usize, total_lines: usize) -> bool {
    total_bytes > MAX_HIGHLIGHT_BYTES || total_lines > MAX_HIGHLIGHT_LINES
}

// -- 核心高亮 ----------------------------------------------------------------

/// 核心高亮器，接受显式主题引用。
///
/// 这保持生产行为和测试行为在相同的代码路径上：
/// 生产调用者传递全局主题锁，而测试可以传递
/// 具体主题而不变化进程全局状态。
fn highlight_to_line_spans_with_theme(
    code: &str,
    lang: &str,
    theme: &Theme,
) -> Option<Vec<Vec<Span<'static>>>> {
    // 空输入没有可高亮的内容；回退到纯文本路径，
    // 它正确地产生一个空 Line。
    if code.is_empty() {
        return None;
    }

    // 过大输入提前退出，避免过度资源消耗。
    // 数实际行数（而非换行字节数），避免输入
    // 不以换行结尾时出现差一错误。
    if code.len() > MAX_HIGHLIGHT_BYTES || code.lines().count() > MAX_HIGHLIGHT_LINES {
        return None;
    }

    let syntax = find_syntax(lang)?;
    let mut h = HighlightLines::new(syntax, theme);
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();

    for line in LinesWithEndings::from(code) {
        let ranges = h.highlight_line(line, syntax_set()).ok()?;
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (style, text) in ranges {
            // 去除行尾换行符（LF 和 CR），因为换行由我们自己处理。
            // 否则 CRLF 输入会留下多余的 \r。
            let text = text.trim_end_matches(['\n', '\r']);
            if text.is_empty() {
                continue;
            }
            spans.push(Span::styled(text.to_string(), convert_style(style)));
        }
        if spans.is_empty() {
            spans.push(Span::raw(String::new()));
        }
        lines.push(spans);
    }

    Some(lines)
}

/// 使用 syntect 解析 `code`，针对 `lang` 返回每行样式化的片段。
/// 每个内部 Vec 代表一个源行。当语言
/// 未识别或输入超出安全限制时返回 None。
fn highlight_to_line_spans(code: &str, lang: &str) -> Option<Vec<Vec<Span<'static>>>> {
    let theme_guard = match theme_lock().read() {
        Ok(theme_guard) => theme_guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    highlight_to_line_spans_with_theme(code, lang, &theme_guard)
}

// -- 公共 API -----------------------------------------------------------------

/// 在任何受支持的语言中高亮代码，返回样式化的 ratatui `Line`。
///
/// 当语言未识别或输入超出安全防护栏时，回退为纯文本。
/// 调用者始终可以直接渲染结果——回退路径产生等效的纯文本行。
///
/// 被 `markdown_render` 用于围栏代码块，被 `exec_cell` 用于 bash
/// 命令高亮。
pub(crate) fn highlight_code_to_lines(code: &str, lang: &str) -> Vec<Line<'static>> {
    if let Some(line_spans) = highlight_to_line_spans(code, lang) {
        line_spans.into_iter().map(Line::from).collect()
    } else {
        // 回退：纯文本，每行源码对应一个 `Line`。
        // 使用 `lines()` 而非 `split('\n')`，避免输入以 '\n' 结尾时
        // 产生虚构的尾部空元素（pulldown-cmark 会这样输出）。
        let mut result: Vec<Line<'static>> =
            code.lines().map(|l| Line::from(l.to_string())).collect();
        if result.is_empty() {
            result.push(Line::from(String::new()));
        }
        result
    }
}

/// 向后兼容的 bash 高亮包装器，用于执行单元格。
pub(crate) fn highlight_bash_to_lines(script: &str) -> Vec<Line<'static>> {
    highlight_code_to_lines(script, "bash")
}

/// 高亮代码并返回每行样式化的 spans，用于 diff 集成。
///
/// 当语言未识别或输入超过防护栏时返回 `None`。调用者（`diff_render`）使用此信号回退到
/// 纯 diff 着色。
///
/// 每个内部 `Vec<Span>` 对应一个源行。样式派生
/// 自活动主题，但背景被故意省略，以便
/// 终端自己的背景显示出来。
pub(crate) fn highlight_code_to_styled_spans(
    code: &str,
    lang: &str,
) -> Option<Vec<Vec<Span<'static>>>> {
    highlight_to_line_spans(code, lang)
}

#[cfg(test)]
#[cfg(test)]
mod tests;
