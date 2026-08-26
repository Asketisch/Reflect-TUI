//! 审批 / 权限 / 文件系统 / 网络。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandExecutionApprovalDecision {
    #[default]
    Accept,
    AcceptForSession,
    AcceptWithExecpolicyAmendment {
        execpolicy_amendment: ExecPolicyAmendment,
    },
    ApplyNetworkPolicyAmendment {
        network_policy_amendment: NetworkPolicyAmendment,
    },
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeApprovalDecision {
    #[default]
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandExecutionRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub started_at_ms: i64,
    pub approval_id: Option<String>,
    pub environment_id: Option<String>,
    pub reason: Option<String>,
    pub network_approval_context: Option<NetworkApprovalContext>,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub command_actions: Option<Vec<CommandAction>>,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    pub proposed_network_policy_amendments: Option<Vec<NetworkPolicyAmendment>>,
    pub available_decisions: Option<Vec<CommandExecutionApprovalDecision>>,
}

impl CommandExecutionRequestApprovalParams {
    /// 桩实现
    pub fn strip_experimental_fields(&mut self) {
        self.additional_permissions = None;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandExecutionRequestApprovalResponse {
    pub decision: CommandExecutionApprovalDecision,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileChangeRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub started_at_ms: i64,
    pub reason: Option<String>,
    pub grant_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileChangeRequestApprovalResponse {
    pub decision: FileChangeApprovalDecision,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecPolicyAmendment {
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicyAmendment {
    pub host: String,
    pub action: NetworkPolicyRuleAction,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkPolicyRuleAction {
    #[default]
    Allow,
    Deny,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkApprovalContext {
    pub host: String,
    pub protocol: NetworkApprovalProtocol,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkApprovalProtocol {
    #[default]
    Http,
    Https,
    Socks5Tcp,
    Socks5Udp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestPermissionProfile {
    pub network: Option<AdditionalNetworkPermissions>,
    pub file_system: Option<AdditionalFileSystemPermissions>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdditionalPermissionProfile {
    pub network: Option<AdditionalNetworkPermissions>,
    pub file_system: Option<AdditionalFileSystemPermissions>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdditionalNetworkPermissions {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdditionalFileSystemPermissions {
    pub read: Option<Vec<String>>,
    pub write: Option<Vec<String>>,
    pub glob_scan_max_depth: Option<std::num::NonZeroUsize>,
    pub entries: Option<Vec<FileSystemSandboxEntry>>,
}

impl From<crate::protocol_compat::models::FileSystemPermissions>
    for AdditionalFileSystemPermissions
{
    fn from(v: crate::protocol_compat::models::FileSystemPermissions) -> Self {
        Self {
            read: None,
            write: None,
            glob_scan_max_depth: v.glob_scan_max_depth.and_then(std::num::NonZeroUsize::new),
            entries: Some(
                v.entries
                    .into_iter()
                    .map(FileSystemSandboxEntry::from)
                    .collect(),
            ),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantedPermissionProfile {
    pub network: Option<AdditionalNetworkPermissions>,
    pub file_system: Option<AdditionalFileSystemPermissions>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSystemSandboxEntry {
    pub path: FileSystemPath,
    pub access: FileSystemAccessMode,
}

impl From<crate::protocol_compat::permissions::FileSystemSandboxEntry> for FileSystemSandboxEntry {
    fn from(v: crate::protocol_compat::permissions::FileSystemSandboxEntry) -> Self {
        let path = match v.path {
            crate::protocol_compat::permissions::FileSystemPath::Path { path } => {
                FileSystemPath::Path { path }
            }
            crate::protocol_compat::permissions::FileSystemPath::GlobPattern { pattern } => {
                FileSystemPath::GlobPattern { pattern }
            }
            crate::protocol_compat::permissions::FileSystemPath::Special { value: _ } => {
                FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                }
            }
        };
        let access = match v.access {
            crate::protocol_compat::permissions::FileSystemAccessMode::Read => {
                FileSystemAccessMode::Read
            }
            crate::protocol_compat::permissions::FileSystemAccessMode::Write => {
                FileSystemAccessMode::Write
            }
            crate::protocol_compat::permissions::FileSystemAccessMode::Deny => {
                FileSystemAccessMode::Deny
            }
        };
        Self { path, access }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileSystemPath {
    Path { path: String },
    GlobPattern { pattern: String },
    Special { value: FileSystemSpecialPath },
}

impl Default for FileSystemPath {
    fn default() -> Self {
        FileSystemPath::Path {
            path: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileSystemAccessMode {
    #[default]
    Read,
    Write,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileSystemSpecialPath {
    Root,
    Minimal,
    ProjectRoots {
        subpath: Option<String>,
    },
    Tmpdir,
    SlashTmp,
    Unknown {
        path: String,
        subpath: Option<String>,
    },
}

impl Default for FileSystemSpecialPath {
    fn default() -> Self {
        FileSystemSpecialPath::Root
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionsRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub environment_id: Option<String>,
    pub started_at_ms: i64,
    pub cwd: PathBuf,
    pub reason: Option<String>,
    pub permissions: RequestPermissionProfile,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionsRequestApprovalResponse {
    pub permissions: GrantedPermissionProfile,
    pub scope: PermissionGrantScope,
    pub strict_auto_review: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PermissionGrantScope {
    #[default]
    Turn,
    Session,
}
