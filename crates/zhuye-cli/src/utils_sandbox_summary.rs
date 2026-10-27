//! 为状态卡片与权限界面渲染权限配置的可读摘要。
//!
//! 语义适配到本移植版简化的
//! `PermissionProfile` 枚举（它不再在每个变体上携带 writable-root /
//! exclude-tmpdir 细节）。两个带前缀的字符串——`read-only` 与
//! `workspace-write`——必须保持原样，因为
//! `chatwidget/status_surfaces::permissions_display` 会匹配它们。

use crate::protocol_compat::models::PermissionProfile;
use crate::protocol_compat::permissions::NetworkSandboxPolicy;
use crate::utils_absolute_path::AbsolutePathBuf;

fn network_suffix(network: NetworkSandboxPolicy) -> &'static str {
    if network.is_enabled() {
        " (network access enabled)"
    } else {
        ""
    }
}

pub fn summarize_permission_profile(
    profile: &PermissionProfile,
    cwd: &AbsolutePathBuf,
    workspace_roots: &[AbsolutePathBuf],
) -> String {
    match profile {
        PermissionProfile::ReadOnly => "read-only".to_string(),
        PermissionProfile::Unrestricted | PermissionProfile::Disabled => {
            "danger-full-access".to_string()
        }
        PermissionProfile::External { network } => {
            let mut summary = "external-sandbox".to_string();
            summary.push_str(network_suffix(*network));
            summary
        }
        PermissionProfile::Managed { network, .. } => {
            let mut summary = "workspace-write".to_string();
            let mut writable_entries = vec!["workdir".to_string()];
            writable_entries.push("/tmp".to_string());
            writable_entries.push("$TMPDIR".to_string());
            writable_entries.extend(
                workspace_roots
                    .iter()
                    .filter(|root| *root != cwd)
                    .map(|root| root.to_string_lossy().to_string()),
            );
            summary.push_str(&format!(" [{}]", writable_entries.join(", ")));
            summary.push_str(network_suffix(*network));
            summary
        }
        PermissionProfile::WorkspaceWrite => {
            let mut summary = "workspace-write".to_string();
            let mut writable_entries = vec!["workdir".to_string()];
            writable_entries.push("/tmp".to_string());
            writable_entries.push("$TMPDIR".to_string());
            writable_entries.extend(
                workspace_roots
                    .iter()
                    .filter(|root| *root != cwd)
                    .map(|root| root.to_string_lossy().to_string()),
            );
            summary.push_str(&format!(" [{}]", writable_entries.join(", ")));
            summary
        }
    }
}
