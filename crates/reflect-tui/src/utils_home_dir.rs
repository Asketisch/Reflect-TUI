use std::path::PathBuf;

/// 解析 Reflect 主目录。
///
/// 遵循上游优先级：`REFLECT_HOME` 环境变量优先级最高，
/// 其次回退到 `$HOME`。上游还会在 `$HOME` 后附加 .reflect
/// （Unix 上为 .reflect，Windows 上为 `%USERPROFILE%\.reflect`）——但移植版 TUI
/// 只需用于安装上下文检测的基础目录，因此我们保持
/// 解析后的路径不变，由调用方自行拼接子目录。
pub fn find_reflect_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("REFLECT_HOME") {
        return Some(PathBuf::from(home));
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Some(PathBuf::from(home).join(".reflect"));
    }
    None
}
