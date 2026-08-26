//! 技能 / 插件 / 市场。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillsListResponse {
    pub data: Vec<SkillsListEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillsListEntry {
    pub cwd: PathBuf,
    pub skills: Vec<SkillMetadata>,
    pub errors: Vec<SkillErrorInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillErrorInfo {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: Option<String>,
    pub short_description: Option<String>,
    pub interface: Option<SkillInterface>,
    pub dependencies: Option<serde_json::Value>,
    pub path: crate::utils_absolute_path::AbsolutePathBuf,
    pub scope: SkillScope,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillInterface {
    pub display_name: Option<String>,
    pub short_description: Option<String>,
    pub icon_small: Option<PathBuf>,
    pub icon_large: Option<PathBuf>,
    pub brand_color: Option<String>,
    pub default_prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SkillScope {
    #[default]
    User,
    Repo,
    System,
    Admin,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketplaceAddResponse {
    pub marketplace_name: String,
    pub installed_root: PathBuf,
    pub already_added: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketplaceRemoveResponse {
    pub marketplace_name: String,
    pub installed_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketplaceUpgradeResponse {
    pub selected_marketplaces: Vec<String>,
    pub upgraded_roots: Vec<PathBuf>,
    pub errors: Vec<MarketplaceUpgradeErrorInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketplaceUpgradeErrorInfo {
    pub marketplace_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginListResponse {
    pub marketplaces: Vec<PluginMarketplaceEntry>,
    pub marketplace_load_errors: Vec<MarketplaceLoadErrorInfo>,
    pub featured_plugin_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketplaceLoadErrorInfo {
    pub marketplace_path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginMarketplaceEntry {
    pub name: String,
    pub path: Option<PathBuf>,
    pub interface: Option<MarketplaceInterface>,
    pub plugins: Vec<PluginSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarketplaceInterface {
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginReadParams {
    pub marketplace_path: Option<PathBuf>,
    pub remote_marketplace_name: Option<String>,
    pub plugin_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginReadResponse {
    pub plugin: PluginDetail,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginDetail {
    pub marketplace_name: String,
    pub marketplace_path: Option<PathBuf>,
    pub summary: PluginSummary,
    pub share_url: Option<String>,
    pub description: Option<String>,
    pub skills: Vec<SkillMetadata>,
    pub hooks: Vec<HookMetadata>,
    pub apps: Vec<AppSummary>,
    pub app_templates: Vec<serde_json::Value>,
    pub mcp_servers: Vec<String>,
    pub scheduled_tasks: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginSummary {
    pub id: String,
    pub remote_plugin_id: Option<String>,
    pub version: Option<String>,
    pub local_version: Option<String>,
    pub name: String,
    pub config_name: String,
    pub description: Option<String>,
    pub has_skills: bool,
    pub mcp_server_names: Vec<String>,
    pub app_connector_ids: Vec<String>,
    pub share_context: Option<PluginShareContext>,
    pub source: PluginSource,
    pub installed: bool,
    pub enabled: bool,
    pub install_policy: PluginInstallPolicy,
    pub install_policy_source: Option<PluginInstallPolicySource>,
    pub must_show_installation_interstitial: Option<bool>,
    pub auth_policy: PluginAuthPolicy,
    pub availability: PluginAvailability,
    pub interface: Option<PluginInterface>,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginInterface {
    pub description: Option<String>,
    pub display_name: Option<String>,
    pub logo: Option<PathBuf>,
    pub logo_dark: Option<PathBuf>,
    pub logo_url: Option<String>,
    pub logo_url_dark: Option<String>,
    pub screenshots: Vec<PathBuf>,
    pub screenshot_urls: Vec<String>,
    pub long_description: Option<String>,
    pub short_description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginShareContext {
    pub remote_plugin_id: String,
    pub remote_version: Option<String>,
    pub discoverability: Option<PluginShareDiscoverability>,
    pub share_url: Option<String>,
    pub creator_account_user_id: Option<String>,
    pub creator_name: Option<String>,
    pub share_principals: Option<Vec<PluginSharePrincipal>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginShareDiscoverability {
    #[default]
    Listed,
    Unlisted,
    Private,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginSharePrincipal {
    pub principal_type: PluginSharePrincipalType,
    pub principal_id: String,
    pub role: PluginSharePrincipalRole,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginSharePrincipalType {
    #[default]
    User,
    Group,
    Workspace,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginSharePrincipalRole {
    #[default]
    Reader,
    Editor,
    Owner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSource {
    Local {
        path: PathBuf,
    },
    Git {
        url: String,
        path: Option<String>,
        ref_name: Option<String>,
        sha: Option<String>,
    },
    Npm {
        package: String,
        version: Option<String>,
        registry: Option<String>,
    },
    Remote,
}

impl Default for PluginSource {
    fn default() -> Self {
        PluginSource::Remote
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginInstallPolicy {
    #[default]
    NotAvailable,
    Available,
    InstalledByDefault,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginInstallPolicySource {
    #[default]
    WorkspaceSetting,
    ImplicitCanonicalApp,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginAuthPolicy {
    #[default]
    OnInstall,
    OnUse,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginAvailability {
    #[default]
    Available,
    DisabledByAdmin,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginInstallResponse {
    pub auth_policy: PluginAuthPolicy,
    pub apps_needing_auth: Vec<AppSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginUninstallResponse {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub install_url: Option<String>,
    pub category: Option<String>,
}
