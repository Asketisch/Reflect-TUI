use crate::install_context::InstallContext;
#[cfg(all(test, feature = "tui-upstream-tests"))]
use crate::install_context::InstallMethod;
#[cfg(all(test, feature = "tui-upstream-tests"))]
use crate::install_context::StandalonePlatform;

/// TUI 退出后 CLI 应执行的更新动作。
///
/// Reflect 不提供上游安装路径，因此解析结果始终
/// 返回 `None`。保留这些变体是为了与
/// 调用方和测试保持结构兼容。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// 通过 npm 包更新。
    NpmGlobalLatest,
    /// 通过 bun 包更新。
    BunGlobalLatest,
    /// 通过 pnpm 包更新。
    PnpmGlobalLatest,
    /// 通过 Homebrew 更新。
    BrewUpgrade,
    /// 通过独立的 Unix 安装器更新。
    StandaloneUnix,
    /// 通过独立的 Windows 安装器更新。
    StandaloneWindows,
}

impl UpdateAction {
    pub(crate) fn from_install_context(_context: &InstallContext) -> Option<Self> {
        // Reflect 不发布上游安装/更新命令，因此
        // 永远不会有需要执行的自动更新动作。
        None
    }

    /// 返回调用更新所需的命令行参数列表。
    ///
    /// Reflect 不输出上游安装命令。此处返回一个中性的
    /// 占位结果，使该方法对结构化调用方仍然可用。
    pub fn command_args(self) -> (&'static str, &'static [&'static str]) {
        ("true", &[])
    }

    /// 返回调用更新的命令行参数的字符串表示。
    pub fn command_str(self) -> String {
        let (command, args) = self.command_args();
        shlex::try_join(std::iter::once(command).chain(args.iter().copied()))
            .unwrap_or_else(|_| format!("{command} {}", args.join(" ")))
    }
}

#[cfg(not(debug_assertions))]
pub fn get_update_action() -> Option<UpdateAction> {
    UpdateAction::from_install_context(&InstallContext::current())
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests {
    use super::*;
    use crate::utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;

    #[test]
    fn maps_install_context_to_update_action() {
        let native_release_dir =
            AbsolutePathBuf::from_absolute_path(std::env::temp_dir().join("native-release"))
                .expect("temp dir path should be absolute");

        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Other,
                package_layout: None,
            }),
            None
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Npm,
                package_layout: None,
            }),
            Some(UpdateAction::NpmGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Bun,
                package_layout: None,
            }),
            Some(UpdateAction::BunGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Pnpm,
                package_layout: None,
            }),
            Some(UpdateAction::PnpmGlobalLatest)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Brew,
                package_layout: None,
            }),
            Some(UpdateAction::BrewUpgrade)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Standalone {
                    platform: StandalonePlatform::Unix,
                    release_dir: native_release_dir.clone(),
                    resources_dir: Some(native_release_dir.join("reflect-resources")),
                },
                package_layout: None,
            }),
            Some(UpdateAction::StandaloneUnix)
        );
        assert_eq!(
            UpdateAction::from_install_context(&InstallContext {
                method: InstallMethod::Standalone {
                    platform: StandalonePlatform::Windows,
                    release_dir: native_release_dir.clone(),
                    resources_dir: Some(native_release_dir.join("reflect-resources")),
                },
                package_layout: None,
            }),
            Some(UpdateAction::StandaloneWindows)
        );
    }

    #[test]
    fn standalone_update_commands_return_neutral_placeholder() {
        // Reflect 不输出上游安装命令；所有变体
        // 都解析为中性的空操作占位结果。
        assert_eq!(
            UpdateAction::StandaloneUnix.command_args(),
            ("true", &[][..]),
        );
        assert_eq!(
            UpdateAction::StandaloneWindows.command_args(),
            ("true", &[][..]),
        );
    }
}
