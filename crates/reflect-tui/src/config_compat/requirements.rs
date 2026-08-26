use super::*;

/// 受管需求值的来源信息。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RequirementSource {
    #[default]
    Unknown,
    MdmManagedPreferences {
        domain: String,
        key: String,
    },
    Composite {
        sources: Vec<RequirementSource>,
    },
    EnterpriseManaged {
        id: String,
        name: String,
    },
    SystemRequirementsToml {
        file: PathBuf,
    },
    LegacyManagedConfigTomlFromFile {
        file: PathBuf,
    },
    LegacyManagedConfigTomlFromMdm,
}

impl RequirementSource {
    pub fn composite(sources: impl IntoIterator<Item = RequirementSource>) -> Self {
        let sources: Vec<_> = sources.into_iter().collect();
        match sources.len() {
            0 => Self::Unknown,
            1 => sources.into_iter().next().unwrap(),
            _ => Self::Composite { sources },
        }
    }
}

impl std::fmt::Display for RequirementSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => f.write_str("<unspecified>"),
            Self::MdmManagedPreferences { domain, key } => write!(f, "MDM {domain}:{key}"),
            Self::Composite { sources } => {
                write!(f, "requirements layers: ")?;
                for (i, src) in sources.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{src}")?;
                }
                Ok(())
            }
            Self::EnterpriseManaged { id, name } => {
                write!(f, "enterprise-managed requirements {name} ({id})")
            }
            Self::SystemRequirementsToml { file } => write!(f, "{}", file.display()),
            Self::LegacyManagedConfigTomlFromFile { file } => write!(f, "{}", file.display()),
            Self::LegacyManagedConfigTomlFromMdm => write!(f, "legacy managed_config.toml (MDM)"),
        }
    }
}

/// 受管环境允许的沙箱模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SandboxModeRequirement {
    #[default]
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
    ExternalSandbox,
}

impl std::fmt::Display for SandboxModeRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
            Self::ExternalSandbox => "external-sandbox",
        })
    }
}

/// 应用于云端模型的数据驻留需求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidencyRequirement {
    Us,
    Eu,
    Off,
}

impl Default for ResidencyRequirement {
    fn default() -> Self {
        Self::Off
    }
}

/// 受管环境允许的网页搜索模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum WebSearchModeRequirement {
    #[default]
    Disabled,
    Cached,
    Indexed,
    Live,
}

impl std::fmt::Display for WebSearchModeRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Disabled => "disabled",
            Self::Cached => "cached",
            Self::Indexed => "indexed",
            Self::Live => "live",
        })
    }
}

/// 叠加在配置层栈之上的、已解析的受管需求。
///
/// 所有字段均为存根；内置代码仅在调试渲染时读取字段名。
#[derive(Debug, Clone, Default)]
pub struct ConfigRequirements {
    pub approval_policy: ConstrainedWithSource<String>,
    pub approvals_reviewer: ConstrainedWithSource<String>,
    pub permission_profile: ConstrainedWithSource<String>,
    pub enforce_residency: ConstrainedWithSource<Option<ResidencyRequirement>>,
    pub web_search_mode: ConstrainedWithSource<String>,
    pub allow_managed_hooks_only: Option<Sourced<bool>>,
    pub allow_appshots: Option<Sourced<bool>>,
    pub allow_remote_control: Option<Sourced<bool>>,
    pub feature_requirements: Option<Sourced<FeatureRequirementsToml>>,
    pub network: Option<Sourced<NetworkConstraints>>,
    pub filesystem: Option<Sourced<FilesystemConstraints>>,
    pub mcp_servers: Option<Sourced<std::collections::BTreeMap<String, McpServerRequirement>>>,
    pub hooks: Option<Sourced<ManagedHooksRequirementsToml>>,
    pub guardian_policy_config_source: String,
    pub managed_hooks: Option<String>,
}

/// 从 `requirements.toml` / MDM 反序列化得到的原始需求。
#[derive(Debug, Clone, Default)]
pub struct ConfigRequirementsToml {
    pub allowed_approval_policies: Option<Vec<String>>,
    pub allowed_sandbox_modes: Option<Vec<SandboxModeRequirement>>,
    pub allowed_permission_profiles: Option<std::collections::BTreeMap<String, bool>>,
    pub default_permissions: Option<String>,
    pub allowed_web_search_modes: Option<Vec<WebSearchModeRequirement>>,
    pub allow_managed_hooks_only: Option<bool>,
    pub allow_appshots: Option<bool>,
    pub allow_remote_control: Option<bool>,
    pub enforce_residency: Option<ResidencyRequirement>,
    pub feature_requirements: Option<FeatureRequirementsToml>,
    pub hooks: Option<ManagedHooksRequirementsToml>,
    pub network: Option<NetworkConstraints>,
    pub allowed_approvals_reviewers: Option<crate::config_compat::types::ApprovalsReviewer>,
    pub guardian_policy_config: Option<String>,
    pub mcp_servers: String,
    pub rules: String,
}

/// 按功能划分的启用/禁用需求。
#[derive(Debug, Clone, Default)]
pub struct FeatureRequirementsToml {
    pub entries: std::collections::BTreeMap<String, bool>,
}

impl FeatureRequirementsToml {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 受管钩子需求（以允许/拒绝列表形式给出的钩子事件）。
#[derive(Debug, Clone, Default)]
pub struct ManagedHooksRequirementsToml {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub managed_dir: Option<std::path::PathBuf>,
    pub windows_managed_dir: Option<std::path::PathBuf>,
}

/// 文件系统访问约束。
#[derive(Debug, Clone, Default)]
pub struct FilesystemConstraints {
    pub deny: Vec<PathBuf>,
    pub deny_read: Vec<String>,
}

/// 网络访问约束。
#[derive(Debug, Clone, Default)]
pub struct NetworkConstraints {
    pub enabled: Option<bool>,
    pub http_port: Option<u16>,
    pub socks_port: Option<u16>,
    pub allow_upstream_proxy: Option<bool>,
    pub dangerously_allow_non_loopback_proxy: Option<bool>,
    pub dangerously_allow_all_unix_sockets: Option<bool>,
    pub domains: NetworkDomainPermissionsToml,
    pub managed_allowed_domains_only: Option<bool>,
    pub unix_sockets: NetworkUnixSocketPermissionsToml,
    pub allow_local_binding: Option<bool>,
}

/// 以域名 glob 为键的逐域网络权限条目。
#[derive(Debug, Clone, Default)]
pub struct NetworkDomainPermissionsToml {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub ask: Option<Vec<String>>,
}

/// TOML 中单个域名权限条目的形态。
#[derive(Debug, Clone, Default)]
pub struct NetworkDomainPermissionToml {
    pub domain: String,
    pub permission: String,
}

impl std::fmt::Display for NetworkDomainPermissionToml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.permission)
    }
}

impl NetworkDomainPermissionToml {
    #[allow(non_snake_case)]
    pub fn Allow(domain: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            permission: "allow".to_string(),
        }
    }

    #[allow(non_snake_case)]
    pub fn Deny(domain: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            permission: "deny".to_string(),
        }
    }
}

/// 以套接字路径 glob 为键的逐套接字 Unix 网络权限条目。
#[derive(Debug, Clone, Default)]
pub struct NetworkUnixSocketPermissionsToml {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub ask: Option<Vec<String>>,
}

/// TOML 中单个 Unix 套接字权限条目的形态。
#[derive(Debug, Clone, Default)]
pub struct NetworkUnixSocketPermissionToml {
    pub socket: String,
    pub permission: String,
}

impl std::fmt::Display for NetworkUnixSocketPermissionToml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.permission)
    }
}

impl NetworkUnixSocketPermissionToml {
    #[allow(non_snake_case)]
    pub fn Allow(socket: impl Into<String>) -> Self {
        Self {
            socket: socket.into(),
            permission: "allow".to_string(),
        }
    }

    #[allow(non_snake_case)]
    pub fn Deny(socket: impl Into<String>) -> Self {
        Self {
            socket: socket.into(),
            permission: "deny".to_string(),
        }
    }
}

/// 受管钩子需求所携带的钩子事件集合。
#[derive(Debug, Clone, Default)]
pub struct HookEventsToml {
    pub events: Vec<String>,
}

/// 单个钩子处理器的配置。
#[derive(Debug, Clone, Default)]
pub struct HookHandlerConfig {
    pub command: String,
    pub timeout: Option<u64>,
}

/// 用于选择钩子应用于哪些工具调用的匹配器组。
#[derive(Debug, Clone, Default)]
pub struct MatcherGroup {
    pub matchers: Vec<String>,
}

/// 受管 MCP 服务器需求的稳定标识。
#[derive(Debug, Clone, Default)]
pub struct McpServerIdentity {
    pub name: String,
}

/// 按 MCP 服务器划分的受管需求。
#[derive(Debug, Clone, Default)]
pub struct McpServerRequirement {
    pub allowed: Option<bool>,
}
