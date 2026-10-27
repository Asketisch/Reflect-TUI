//! Windows 沙箱等级解析与设置状态(TUI 侧消费面)。
//!
//! 引擎在 Windows 上尚无真实沙箱后端(`zhuye-sandbox::detect_backend`
//! 返回 `NoBackend`),本模块只负责把配置中的沙箱模式映射为 UI 使用的
//! 等级,并提供设置完成标记、日志路径约定与 world-writable 扫描入口。
//!
//! 移植期边界:
//! - Everyone-ACL 的 world-writable 扫描与 deny 注入尚未实现,扫描恒
//!   通过、不触发警告弹窗(见
//!   [`apply_world_writable_scan_and_denies_for_permissions`]);
//! - elevated 设置完成标记由未来的设置流程写入
//!   [`setup_complete_marker`],当前仅有读取侧。

use std::path::{Path, PathBuf};

use crate::protocol_compat::config_types::WindowsSandboxLevel;
use crate::protocol_compat::models::PermissionProfile;
use crate::tui_core::legacy_core::config::Config;

/// 由配置解析 Windows 沙箱等级。
///
/// - `permissions.windows_sandbox_mode = "elevated"` →
///   [`WindowsSandboxLevel::Elevated`]
/// - `"unelevated"`(非管理员降级)→ [`WindowsSandboxLevel::RestrictedToken`]
/// - 未设置或无法识别 → [`WindowsSandboxLevel::Disabled`]
pub fn level_from_config(config: &Config) -> WindowsSandboxLevel {
    match config.permissions.windows_sandbox_mode.as_str() {
        "elevated" => WindowsSandboxLevel::Elevated,
        "unelevated" => WindowsSandboxLevel::RestrictedToken,
        _ => WindowsSandboxLevel::Disabled,
    }
}

/// elevated 沙箱的一次性设置完成标记(`zhuye_home` 下)。
/// 设置流程成功执行后创建;缺失即视为尚未完成设置。
fn setup_complete_marker(zhuye_home: &Path) -> PathBuf {
    zhuye_home.join("windows-sandbox").join("setup-complete")
}

/// elevated 沙箱的一次性设置是否已完成。
pub fn sandbox_setup_is_complete(zhuye_home: &Path) -> bool {
    setup_complete_marker(zhuye_home).is_file()
}

/// Windows 沙箱日志文件路径(反馈附件收集用)。
///
/// 移植期:路径约定先行固定,写入方(设置流程)尚未落地。
pub fn current_log_file_path_for_zhuye_home(zhuye_home: &Path) -> PathBuf {
    zhuye_home.join("windows-sandbox").join("sandbox-log.txt")
}

/// 由权限档案解析出的 world-writable 扫描范围。
#[derive(Debug, Clone, Default)]
pub struct ResolvedWindowsSandboxPermissions {
    workspace_roots: Vec<PathBuf>,
}

impl ResolvedWindowsSandboxPermissions {
    /// 从权限档案与 workspace 根列表解析扫描范围。
    ///
    /// 移植期:`PermissionProfile` 的文件系统策略尚为存根,这里直接采用
    /// 传入的 workspace 根;档案为 `Disabled`(完全访问)时返回 `Err`,
    /// 表示无沙箱保护需要核验,调用方可跳过警告。
    pub fn try_from_permission_profile_for_workspace_roots<P: AsRef<Path>>(
        profile: &PermissionProfile,
        workspace_roots: &[P],
    ) -> Result<Self, String> {
        if matches!(profile, PermissionProfile::Disabled) {
            return Err("full access mode; no sandbox protection to verify".to_string());
        }
        Ok(Self {
            workspace_roots: workspace_roots
                .iter()
                .map(|root| root.as_ref().to_path_buf())
                .collect(),
        })
    }
}

/// 对工作区做 world-writable 扫描,并为权限档案追加相应 deny 规则。
///
/// 移植期:Everyone(S-1-1-0)ACL 写位扫描与 deny 注入尚未实现,恒返回
/// `Ok(())`(即「扫描通过,不弹警告」)。真实实现需借助 windows-sys
/// 查询各根目录 DACL,发现 Everyone 可写目录时返回 `Err` 并由调用方
/// 弹出警告确认。
pub fn apply_world_writable_scan_and_denies_for_permissions(
    _zhuye_home: &Path,
    _cwd: &Path,
    _env_map: &std::collections::HashMap<String, String>,
    _permissions: &ResolvedWindowsSandboxPermissions,
    _zhuye_home_hint: Option<&Path>,
) -> Result<(), String> {
    Ok(())
}
