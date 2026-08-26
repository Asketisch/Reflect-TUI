use super::*;

/// 顶层的网络沙箱开关。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkSandboxPolicy {
    #[default]
    Restricted,
    Enabled,
}

impl NetworkSandboxPolicy {
    pub fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

/// 文件系统条目的访问模式。
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum FileSystemAccessMode {
    #[default]
    Read,
    Write,
    #[serde(alias = "none")]
    Deny,
}

impl FileSystemAccessMode {
    pub fn can_read(self) -> bool {
        !matches!(self, Self::Deny)
    }

    pub fn can_write(self) -> bool {
        matches!(self, Self::Write)
    }
}

/// 权限条目中的 `:special_path` 令牌。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileSystemSpecialPath {
    Root,
    Minimal,
    #[serde(alias = "current_working_directory")]
    ProjectRoots {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subpath: Option<String>,
    },
    Tmpdir,
    SlashTmp,
    Unknown {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subpath: Option<String>,
    },
}

impl FileSystemSpecialPath {
    pub fn project_roots(subpath: Option<String>) -> Self {
        Self::ProjectRoots { subpath }
    }

    pub fn unknown(path: impl Into<String>, subpath: Option<String>) -> Self {
        Self::Unknown {
            path: path.into(),
            subpath,
        }
    }
}

/// 权限条目内的路径目标。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileSystemPath {
    Path { path: String },
    GlobPattern { pattern: String },
    Special { value: FileSystemSpecialPath },
}

impl Default for FileSystemPath {
    fn default() -> Self {
        Self::Path {
            path: String::new(),
        }
    }
}

/// 文件系统沙箱策略表中的一个条目。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileSystemSandboxEntry {
    pub path: FileSystemPath,
    pub access: FileSystemAccessMode,
}

/// 文件系统沙箱策略的粗粒度形态。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileSystemSandboxKind {
    #[default]
    Restricted,
    WorkspaceWrite,
    Unrestricted,
    ExternalSandbox,
}

/// 文件系统沙箱策略表。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSystemSandboxPolicy {
    pub kind: FileSystemSandboxKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob_scan_max_depth: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<FileSystemSandboxEntry>,
}

impl FileSystemSandboxPolicy {
    pub fn has_full_disk_write_access(&self) -> bool {
        matches!(
            self.kind,
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox
        )
    }

    pub fn can_write_path_with_cwd(&self, _cwd: &std::path::Path, _path: &std::path::Path) -> bool {
        matches!(
            self.kind,
            FileSystemSandboxKind::Unrestricted
                | FileSystemSandboxKind::WorkspaceWrite
                | FileSystemSandboxKind::ExternalSandbox
        )
    }

    pub fn get_writable_roots_with_cwd(&self, _cwd: &std::path::Path) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}
