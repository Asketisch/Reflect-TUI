//! 检测 Reflect/Reflect 二进制文件的安装方式。
//!
//! 对齐上游 `install-context` 的语义：检查当前可执行文件路径与
//! `REFLECT_HOME`，据此分类安装方式（npm/bun/pnpm/brew/standalone/其他）。
//! 驱动 `update_action::get_update_action()`,以便"有可用更新"提示能建议
//! 正确的升级命令。

use std::path::Path;

use crate::utils_absolute_path::AbsolutePathBuf;
use crate::utils_home_dir::find_reflect_home;

const STANDALONE_PACKAGES_DIRNAME: &str = "standalone";
const RELEASES_DIRNAME: &str = "releases";
const RESOURCES_DIRNAME: &str = "reflect-resources";

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct InstallContext {
    pub method: InstallMethod,
    pub package_layout: Option<PackageLayout>,
}

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub enum InstallMethod {
    #[default]
    Binary,
    Npm,
    Bun,
    Pnpm,
    Brew,
    Standalone {
        platform: StandalonePlatform,
        release_dir: AbsolutePathBuf,
        resources_dir: Option<AbsolutePathBuf>,
    },
    Other,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum StandalonePlatform {
    #[default]
    MacOS,
    Linux,
    Windows,
    Unix,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum PackageLayout {
    #[default]
    Default,
    Portable,
}

impl InstallContext {
    /// 检测当前运行二进制的安装方式。
    ///
    /// 优先级（对齐上游）：
    /// 1. `REFLECT_MANAGED_BY_PNPM` / `_NPM` / `_BUN` 环境变量覆盖。
    /// 2. Standalone 管理式安装（可执行文件位于 `$REFLECT_HOME/packages/standalone/releases`）。
    /// 3. macOS 上的 Homebrew 前缀（`/opt/homebrew` 或 `/usr/local`）。
    /// 4. `Other`（cargo run、应用包、任意路径）。
    pub fn current() -> Self {
        let method_override = if std::env::var_os("REFLECT_MANAGED_BY_PNPM").is_some() {
            Some(InstallMethod::Pnpm)
        } else if std::env::var_os("REFLECT_MANAGED_BY_NPM").is_some() {
            Some(InstallMethod::Npm)
        } else if std::env::var_os("REFLECT_MANAGED_BY_BUN").is_some() {
            Some(InstallMethod::Bun)
        } else {
            None
        };

        let current_exe = std::env::current_exe().ok();
        let method = if let Some(method) = method_override {
            method
        } else if let Some(exe_path) = current_exe.as_deref() {
            install_method_from_exe(exe_path)
        } else {
            InstallMethod::Other
        };

        Self {
            method,
            package_layout: None,
        }
    }
}

fn install_method_from_exe(exe_path: &Path) -> InstallMethod {
    if let Some(standalone) = standalone_install_method(exe_path) {
        return standalone;
    }

    if cfg!(target_os = "macos")
        && (exe_path.starts_with("/opt/homebrew") || exe_path.starts_with("/usr/local"))
    {
        return InstallMethod::Brew;
    }

    InstallMethod::Other
}

fn standalone_install_method(exe_path: &Path) -> Option<InstallMethod> {
    let reflect_home = find_reflect_home()?;
    let release_dir = exe_path.parent()?;
    let releases_root = reflect_home
        .join("packages")
        .join(STANDALONE_PACKAGES_DIRNAME)
        .join(RELEASES_DIRNAME);
    if !release_dir.starts_with(&releases_root) {
        return None;
    }

    let resources_dir = release_dir.join(RESOURCES_DIRNAME);
    Some(InstallMethod::Standalone {
        platform: standalone_platform(),
        release_dir: AbsolutePathBuf::new(release_dir.to_string_lossy().to_string()),
        resources_dir: resources_dir
            .is_dir()
            .then(|| AbsolutePathBuf::new(resources_dir.to_string_lossy().to_string())),
    })
}

fn standalone_platform() -> StandalonePlatform {
    if cfg!(windows) {
        StandalonePlatform::Windows
    } else if cfg!(target_os = "macos") {
        StandalonePlatform::MacOS
    } else if cfg!(target_os = "linux") {
        StandalonePlatform::Linux
    } else {
        StandalonePlatform::Unix
    }
}
