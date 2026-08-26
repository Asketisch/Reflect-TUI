use super::*;

/// 内置权限配置文件的 ID，用作稳定标识符。
pub const BUILT_IN_PERMISSION_PROFILE_READ_ONLY: &str = ":read-only";
pub const BUILT_IN_PERMISSION_PROFILE_WORKSPACE: &str = ":workspace";
pub const BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS: &str = ":danger-full-access";

/// 助手消息的阶段。各提供商对该字段的发出并不一致，
/// 因此调用方必须将 `None` 视为“阶段未知”。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagePhase {
    #[default]
    Commentary,
    FinalAnswer,
}

/// 附加到权限配置文件上的文件系统权限条目。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileSystemPermissions {
    pub entries: Vec<crate::protocol_compat::permissions::FileSystemSandboxEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob_scan_max_depth: Option<usize>,
}

impl FileSystemPermissions {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 附加到权限配置文件上的网络权限标志。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NetworkPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl NetworkPermissions {
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
    }
}

/// 按命令 / 按轮次的权限叠加层。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AdditionalPermissionProfile {
    pub network: Option<NetworkPermissions>,
    pub filesystem: Option<FileSystemPermissions>,
}

/// 自定义权限配置文件的摘要，用于在 TUI 中显示。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PermissionProfileSummary {
    pub id: String,
    pub description: Option<String>,
    pub allowed: bool,
}

/// 受管的文件系统沙箱形态。stub 仅保留 TUI 需要渲染的
/// restricted（受限）形态；上游的其他分支都归并为 `Restricted`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ManagedFileSystemPermissions {
    Restricted {
        #[serde(default)]
        entries: Vec<crate::protocol_compat::permissions::FileSystemSandboxEntry>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        glob_scan_max_depth: Option<usize>,
    },
    FullAccess,
    Unrestricted,
}

/// 应用于会话的活动权限配置文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionProfile {
    Managed {
        file_system: ManagedFileSystemPermissions,
        network: crate::protocol_compat::permissions::NetworkSandboxPolicy,
    },
    WorkspaceWrite,
    ReadOnly,
    Unrestricted,
    Disabled,
    External {
        network: crate::protocol_compat::permissions::NetworkSandboxPolicy,
    },
}

impl Default for PermissionProfile {
    fn default() -> Self {
        Self::Managed {
            file_system: ManagedFileSystemPermissions::Restricted {
                entries: Vec::new(),
                glob_scan_max_depth: None,
            },
            network: crate::protocol_compat::permissions::NetworkSandboxPolicy::Restricted,
        }
    }
}

impl PermissionProfile {
    pub fn read_only() -> Self {
        Self::ReadOnly
    }

    pub fn workspace_write() -> Self {
        Self::WorkspaceWrite
    }

    pub fn workspace_write_with(
        _entries: &[crate::utils_absolute_path::AbsolutePathBuf],
        _network: crate::protocol_compat::permissions::NetworkSandboxPolicy,
        _exclude_tmpdir_env_var: bool,
        _exclude_slash_tmp: bool,
    ) -> Self {
        Self::WorkspaceWrite
    }
}

/// 生成某个 `PermissionProfile` 的命名 / 内置配置文件的元数据。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivePermissionProfile {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
}

impl ActivePermissionProfile {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            extends: None,
        }
    }

    pub fn read_only() -> Self {
        Self::new(BUILT_IN_PERMISSION_PROFILE_READ_ONLY)
    }
}

/// 为第 N 个本地图片附件渲染的标签。上游格式为
/// `<image.N>`；stub 保持相同的格式。
pub fn local_image_label_text(label_number: usize) -> String {
    format!("<image.{}>", label_number)
}
