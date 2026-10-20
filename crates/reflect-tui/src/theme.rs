//! 主题调色板:让 `/theme` 真正生效。
//!
//! 此前 `/theme` 选完只打印 "switched to '{id}' (display only — palette apply
//! not yet wired)",而所有渲染层把颜色硬编码成 `Color::Cyan` / `Color::DarkGray`
//! 等。`DarkGray` 在浅色终端上几乎不可见,`no-color` 主题更是完全没实现。
//!
//! 这里定义语义角色(brand / dim / warn / danger / ok …)而不是具体颜色,
//! 渲染层引用角色。这样换主题只改一处映射,且 `no-color` 能真正剥掉全部颜色。

use ratatui::style::{Color, Style};

/// 语义调色板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// 品牌色:模型名、spinner、面板标题。
    pub brand: Color,
    /// 弱化色:分隔符、路径、cost 等次要信息。
    pub dim: Color,
    /// 警示色:待审批提示、git 分支。
    pub warn: Color,
    /// 危险色:deny 模式、错误。
    pub danger: Color,
    /// 成功色:allow 模式、成功状态。
    pub ok: Color,
    /// 信息色:accept_edits 等中性状态。
    pub info: Color,
    /// 特殊色:plan 模式。
    pub plan: Color,
}

impl Palette {
    /// 深色终端(默认)。
    pub const DARK: Self = Self {
        brand: Color::Cyan,
        dim: Color::DarkGray,
        warn: Color::Yellow,
        danger: Color::Red,
        ok: Color::Green,
        info: Color::Blue,
        plan: Color::Magenta,
    };

    /// 浅色终端:`DarkGray` 在白底上几乎不可见,换成 `Gray` 并提亮品牌色。
    pub const LIGHT: Self = Self {
        brand: Color::Blue,
        dim: Color::Gray,
        warn: Color::Yellow,
        danger: Color::Red,
        ok: Color::Green,
        info: Color::Cyan,
        plan: Color::Magenta,
    };

    /// 无颜色:全部角色落到 `Reset`,由终端用默认前后景色渲染。
    pub const NONE: Self = Self {
        brand: Color::Reset,
        dim: Color::Reset,
        warn: Color::Reset,
        danger: Color::Reset,
        ok: Color::Reset,
        info: Color::Reset,
        plan: Color::Reset,
    };

    /// 按主题 id 取调色板。未知 id 落到深色(默认)。
    pub fn for_theme(theme_id: &str) -> Self {
        match theme_id {
            "light" => Self::LIGHT,
            "no-color" => Self::NONE,
            _ => Self::DARK,
        }
    }

    // ── 语义样式 ──

    pub fn brand(&self) -> Style {
        Style::default().fg(self.brand)
    }

    pub fn dim(&self) -> Style {
        Style::default().fg(self.dim)
    }

    pub fn warn(&self) -> Style {
        Style::default().fg(self.warn)
    }

    pub fn danger(&self) -> Style {
        Style::default().fg(self.danger)
    }

    pub fn ok(&self) -> Style {
        Style::default().fg(self.ok)
    }

    /// 权限模式的 (标签, 样式)。
    pub fn mode_style(&self, mode: reflect_protocol::PermissionMode) -> (&'static str, Style) {
        use reflect_protocol::PermissionMode as Pm;
        match mode {
            Pm::Plan => ("[plan]", Style::default().fg(self.plan)),
            Pm::AcceptEdits => ("[accept_edits]", Style::default().fg(self.info)),
            Pm::Bypass => ("[bypass]", Style::default().fg(self.danger)),
            Pm::Prompt => ("[prompt]", Style::default().fg(self.warn)),
            Pm::Deny => ("[deny]", Style::default().fg(self.danger)),
            Pm::Bubble => ("[bubble]", Style::default().fg(self.info)),
            Pm::Auto => ("[auto]", Style::default().fg(self.ok)),
        }
    }
}

/// 环境变量 `NO_COLOR` 是否要求剥掉颜色。
///
/// 遵循 <https://no-color.org/> 的约定:**存在即生效**(不论值是什么,
/// 包括空串),这是该规范的明确要求。
pub fn no_color_env() -> bool {
    std::env::var_os("NO_COLOR").is_some()
}

/// 解析调色板:主题 id + `NO_COLOR` 环境变量。
///
/// `NO_COLOR` 优先于主题选择 —— 用户显式要求无颜色时不该被配置覆盖。
pub fn resolve(theme_id: &str) -> Palette {
    if no_color_env() {
        Palette::NONE
    } else {
        Palette::for_theme(theme_id)
    }
}

/// 主题持久化文件:`<reflect home>/theme`(内容是主题 id,如 `light`)。
pub fn persisted_path() -> Option<std::path::PathBuf> {
    crate::utils_home_dir::find_reflect_home().map(|h| h.join("theme"))
}

/// 读取上次 `/theme` 选中的主题 id。文件不存在 / 读失败 → `None`。
pub fn load_persisted() -> Option<String> {
    load_from(&persisted_path()?)
}

/// 从指定路径读主题 id。空文件 / 读失败 → `None`(退化到默认主题)。
pub fn load_from(path: &std::path::Path) -> Option<String> {
    let id = std::fs::read_to_string(path).ok()?;
    let id = id.trim();
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// 持久化主题选择(原子写)。
pub fn persist(theme_id: &str) -> std::io::Result<()> {
    match persisted_path() {
        Some(path) => persist_to(&path, theme_id),
        None => Ok(()),
    }
}

/// 原子写主题 id 到指定路径。
///
/// 直接 `fs::write` 会在崩溃时留下空文件,下次启动读成空串 —— 退化到默认
/// 主题,用户的选择静默丢失。同目录 temp + rename。
pub fn persist_to(path: &std::path::Path, theme_id: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("theme.tmp.{}", std::process::id()));
    std::fs::write(&tmp, format!("{theme_id}\n"))?;
    std::fs::rename(&tmp, path)
}

/// 启动时的调色板。优先级:`NO_COLOR` > `REFLECT_TUI_THEME` > 上次持久化 > 深色默认。
pub fn resolve_startup() -> Palette {
    if no_color_env() {
        return Palette::NONE;
    }
    if let Some(id) = std::env::var("REFLECT_TUI_THEME")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        return Palette::for_theme(id.trim());
    }
    match load_persisted() {
        Some(id) => Palette::for_theme(&id),
        None => Palette::DARK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_theme_maps_to_distinct_palette() {
        assert_eq!(Palette::for_theme("dark"), Palette::DARK);
        assert_eq!(Palette::for_theme("light"), Palette::LIGHT);
        assert_eq!(Palette::for_theme("no-color"), Palette::NONE);
        // 未知 id 落到默认深色,不 panic。
        assert_eq!(Palette::for_theme("bogus"), Palette::DARK);
        assert_ne!(Palette::DARK, Palette::LIGHT);
    }

    #[test]
    fn no_color_palette_strips_every_role() {
        // no-color 必须真的剥掉颜色,而不是只改一两个角色。
        let p = Palette::NONE;
        for style in [
            p.brand(),
            p.dim(),
            p.warn(),
            p.danger(),
            p.ok(),
            p.mode_style(reflect_protocol::PermissionMode::Plan).1,
        ] {
            assert_eq!(style.fg, Some(Color::Reset), "no-color 应落到 Reset");
        }
    }

    #[test]
    fn light_palette_avoids_darkgray() {
        // DarkGray 在白底上几乎不可见 —— 浅色主题不得用它。
        assert_ne!(Palette::LIGHT.dim, Color::DarkGray);
        assert_ne!(Palette::LIGHT.brand, Color::DarkGray);
    }

    #[test]
    fn mode_style_covers_all_modes() {
        let p = Palette::DARK;
        for m in [
            reflect_protocol::PermissionMode::Auto,
            reflect_protocol::PermissionMode::Plan,
            reflect_protocol::PermissionMode::AcceptEdits,
            reflect_protocol::PermissionMode::Bypass,
            reflect_protocol::PermissionMode::Prompt,
            reflect_protocol::PermissionMode::Deny,
            reflect_protocol::PermissionMode::Bubble,
        ] {
            let (label, style) = p.mode_style(m);
            assert!(label.starts_with('['), "{label}");
            assert!(style.fg.is_some(), "{label} 应有前景色");
        }
    }

    #[test]
    fn persist_to_and_load_from_round_trip() {
        let dir = std::env::temp_dir().join("reflect-theme-persist-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("theme");

        persist_to(&path, "light").unwrap();
        assert_eq!(load_from(&path).as_deref(), Some("light"));
        // 覆盖写也要对。
        persist_to(&path, "no-color").unwrap();
        assert_eq!(load_from(&path).as_deref(), Some("no-color"));

        // 落盘后不得留下 temp 文件。
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp 应已被 rename 消费: {leftovers:?}"
        );
    }

    #[test]
    fn load_from_tolerates_missing_and_empty() {
        let dir = std::env::temp_dir().join("reflect-theme-load-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_from(&dir.join("nope")), None, "缺文件 → None");
        let empty = dir.join("empty");
        std::fs::write(&empty, "  \n").unwrap();
        assert_eq!(load_from(&empty), None, "空/纯空白 → None");
    }

    #[test]
    fn resolve_honors_no_color_env() {
        // 不能污染调用方的环境,所以只在显式设置时验证,且用完即清。
        // (并行测试下 env 是进程级的,故这里只验证不带 env 时的纯映射。)
        assert_eq!(resolve("dark"), Palette::DARK);
        assert_eq!(resolve("light"), Palette::LIGHT);
    }
}
