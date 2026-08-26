//! legacy_core 模块的存根实现。
//!
//! 在 tui_compat 中，`legacy_core` 封装了配置与核心类型。Reflect 有自己的
//! 配置系统；此存根提供最小化的类型定义，使内联引入的 UI 代码能够编译通过。

pub mod config {
    use std::path::PathBuf;

    #[derive(Debug, Clone, Default)]
    pub struct NetworkProxySpec {
        pub url: String,
    }

    impl NetworkProxySpec {
        pub fn socks_enabled(&self) -> bool {
            false
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct ModelProviderConfig {
        pub base_url: String,
        pub requires_openai_auth: bool,
        pub name: String,
        pub wire_api: String,
    }

    #[derive(Debug, Clone, Default)]
    pub struct MultiAgentV2Config {
        pub max_concurrent_threads_per_session: usize,
    }

    impl ModelProviderConfig {
        pub fn is_openai(&self) -> bool {
            true
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct TuiNotificationsConfig {
        pub notifications: crate::config_compat::types::Notifications,
    }

    #[derive(Debug, Clone, Default)]
    pub struct Config {
        pub cwd: crate::utils_absolute_path::AbsolutePathBuf,
        pub reflect_home: PathBuf,
        pub model: Option<String>,
        pub approvals: ApprovalsConfig,
        pub permissions: Permissions,
        pub collaboration_mode: crate::protocol_compat::config_types::CollaborationMode,
        pub mcp_servers:
            std::collections::HashMap<String, crate::config_compat::types::McpServerConfig>,
        pub features: FeaturesConfig,
        pub animations: bool,
        pub tui_keymap: crate::config_compat::types::TuiKeymap,
        pub tui_terminal_title: Option<Vec<String>>,
        pub tui_pet_anchor: crate::config_compat::types::TuiPetAnchor,
        pub approval_policy: crate::app_server_protocol::AskForApproval,
        pub permission_profile: String,
        pub workspace_roots: Vec<crate::utils_absolute_path::AbsolutePathBuf>,
        pub notifications: crate::config_compat::types::Notifications,
        pub sandbox_mode: crate::config_compat::types::SandboxMode,
        pub resumes_cwd_mode: crate::config_compat::types::ResumeCwdMode,
        pub streaming_mode: String,
        pub reasoning_summary: String,
        pub hide_agent_token_usage: bool,
        pub hide_agent_reasoning: bool,
        pub model_reasoning_effort: Option<String>,
        pub show_raw_agent_markdown: bool,
        pub tools: ToolsConfig,
        pub hooks: HooksConfig,
        pub profile: String,
        pub network_proxy: Option<NetworkProxySpec>,
        pub agent_default_subagent_model: Option<String>,
        pub agent_default_subagent_reasoning_effort: Option<String>,
        pub agent_interrupt_message_enabled: String,
        pub agent_max_depth: String,
        pub agent_max_threads: Option<String>,
        pub agents_enabled: String,
        pub approvals_reviewer: crate::config_compat::types::ApprovalsReviewer,
        pub config_layer_stack: crate::config_compat::ConfigLayerStack,
        pub custom_permission_profiles:
            Vec<crate::protocol_compat::models::PermissionProfileSummary>,
        pub disable_paste_burst: bool,
        pub explicit_permission_profile_mode: bool,
        pub feedback_enabled: bool,
        pub forced_login_method: Option<crate::protocol_compat::config_types::ForcedLoginMethod>,
        pub memories: MemoriesConfig,
        pub model_context_window: String,
        pub model_provider: ModelProviderConfig,
        pub model_provider_id: String,
        pub model_reasoning_summary: String,
        pub notices: NoticesConfig,
        pub personality: Option<crate::protocol_compat::config_types::Personality>,
        pub plan_mode_reasoning_effort:
            Option<crate::protocol_compat::openai_models::ReasoningEffort>,
        pub service_tier: Option<String>,
        pub show_raw_agent_reasoning: bool,
        pub show_tooltips: bool,
        pub terminal_resize_reflow: String,
        pub tui_notifications: TuiNotificationsConfig,
        pub tui_pet: Option<String>,
        pub tui_raw_output_mode: bool,
        pub tui_status_line: Option<Vec<String>>,
        pub tui_status_line_use_colors: bool,
        pub tui_theme: Option<String>,
        pub tui_vim_mode_default: bool,
        pub multi_agent_v2: MultiAgentV2Config,
        pub hide_rate_limit_model_nudge: bool,
        pub hide_world_writable_warning: bool,
    }

    impl Config {
        /// 配置编辑器的存根实现。
        pub fn edit(&self) -> std::io::Result<()> {
            Ok(())
        }

        /// 返回生效的工作区根目录，为空时回退到当前工作目录。
        pub fn effective_workspace_roots(
            &self,
        ) -> Vec<crate::utils_absolute_path::AbsolutePathBuf> {
            if self.workspace_roots.is_empty() {
                vec![self.cwd.clone()]
            } else {
                self.workspace_roots.clone()
            }
        }

        pub fn is_permission_profile_allowed(
            &self,
            _id: &str,
            _profile: &crate::protocol_compat::models::PermissionProfile,
        ) -> bool {
            true
        }
    }

    /// Config 的构建器（用于测试）。
    #[derive(Debug, Clone, Default)]
    pub struct ConfigBuilder {
        config: Config,
    }

    impl ConfigBuilder {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn with_cwd(mut self, cwd: PathBuf) -> Self {
            self.config.cwd = cwd.into();
            self
        }
        pub fn with_reflect_home(mut self, home: PathBuf) -> Self {
            self.config.reflect_home = home;
            self
        }
        pub fn with_model(mut self, model: String) -> Self {
            self.config.model = Some(model);
            self
        }
        pub fn build(self) -> Config {
            self.config
        }
    }

    /// 审批配置段的存根实现。
    #[derive(Debug, Clone, Default)]
    pub struct ApprovalsConfig {
        pub policy: String,
    }

    /// 功能特性配置段。
    #[derive(Debug, Clone, Default)]
    pub struct FeaturesConfig {
        pub web_search: bool,
        pub apply_patch: bool,
        pub view_image: bool,
        pub enabled_features: std::collections::HashSet<crate::features::Feature>,
    }

    impl FeaturesConfig {
        pub fn enabled(&self, feature: crate::features::Feature) -> bool {
            self.enabled_features.contains(&feature)
        }

        pub fn is_enabled(&self, feature: crate::features::Feature) -> bool {
            self.enabled(feature)
        }

        pub fn get(&self) -> Self {
            self.clone()
        }

        pub fn set_enabled(
            &mut self,
            feature: crate::features::Feature,
            enabled: bool,
        ) -> Result<(), crate::config_compat::ConstraintError> {
            if enabled {
                self.enabled_features.insert(feature);
            } else {
                self.enabled_features.remove(&feature);
            }
            Ok(())
        }

        pub fn can_set(&self, _next: &Self) -> Result<(), crate::config_compat::ConstraintError> {
            Ok(())
        }
    }

    /// 动画配置段。
    #[derive(Debug, Clone, Default)]
    pub struct AnimationsConfig {
        pub pets: bool,
        pub shimmer: bool,
    }

    /// 记忆配置段。
    #[derive(Debug, Clone, Default)]
    pub struct MemoriesConfig {
        pub use_memories: bool,
        pub generate_memories: bool,
    }

    /// 工具配置段。
    #[derive(Debug, Clone, Default)]
    pub struct ToolsConfig {
        pub web_search: bool,
    }

    /// 提示信息配置段。
    #[derive(Debug, Clone, Default)]
    pub struct NoticesConfig {
        pub hide_world_writable_warning: Option<bool>,
        pub hide_rate_limit_model_nudge: Option<bool>,
        pub fast_default_opt_out: Option<bool>,
    }

    /// 钩子配置段。
    #[derive(Debug, Clone, Default)]
    pub struct HooksConfig {
        pub enabled: bool,
    }

    /// 权限配置段。
    #[derive(Debug, Clone, Default)]
    pub struct Permissions {
        pub file_system_sandbox_policy:
            crate::protocol_compat::permissions::FileSystemSandboxPolicy,
        pub network: Option<NetworkProxySpec>,
        pub approval_policy:
            crate::config_compat::ConstrainedWithSource<crate::app_server_protocol::AskForApproval>,
        pub approvals_reviewer: crate::config_compat::ConstrainedWithSource<
            crate::config_compat::types::ApprovalsReviewer,
        >,
        pub permission_profile: crate::config_compat::ConstrainedWithSource<String>,
        pub windows_sandbox_mode: String,
    }

    impl Permissions {
        pub fn effective_permission_profile(
            &self,
        ) -> crate::protocol_compat::models::PermissionProfile {
            crate::protocol_compat::models::PermissionProfile::default()
        }

        pub fn active_permission_profile(
            &self,
        ) -> Option<crate::protocol_compat::models::ActivePermissionProfile> {
            None
        }

        pub fn permission_profile(&self) -> crate::protocol_compat::models::PermissionProfile {
            crate::protocol_compat::models::PermissionProfile::default()
        }

        pub fn set_workspace_roots(
            &mut self,
            _roots: Vec<crate::utils_absolute_path::AbsolutePathBuf>,
        ) {
        }

        pub fn set_permission_profile_from_session_snapshot(
            &mut self,
            _snapshot: PermissionProfileSnapshot,
        ) -> Result<(), String> {
            Ok(())
        }

        pub fn replace_permission_profile_from_session_snapshot(
            &mut self,
            _snapshot: PermissionProfileSnapshot,
        ) -> Result<(), String> {
            Ok(())
        }

        pub fn can_set_permission_profile(
            &self,
            _profile: &crate::protocol_compat::models::PermissionProfile,
        ) -> Result<(), crate::config_compat::ConstraintError> {
            Ok(())
        }

        pub fn is_permission_profile_allowed(
            &self,
            _id: &str,
            _profile: &crate::protocol_compat::models::PermissionProfile,
        ) -> bool {
            true
        }
    }

    impl PermissionProfileSnapshot {
        pub fn from_session_snapshot(
            _profile: crate::protocol_compat::models::PermissionProfile,
            _active: Option<crate::app_server_protocol::ActivePermissionProfile>,
        ) -> Self {
            Self::default()
        }
    }

    /// 权限配置档状态的快照。
    #[derive(Debug, Clone, Default)]
    pub struct PermissionProfileSnapshot {
        pub active: String,
        pub profiles: Vec<String>,
    }

    /// 模块级的编辑函数。
    pub fn edit(_config: &Config) -> std::io::Result<()> {
        Ok(())
    }

    /// 配置编辑子模块。
    pub mod edit {
        use crate::config_compat::types::ResumeCwdMode;

        /// 配置编辑操作的错误类型。
        #[derive(Debug, Clone)]
        pub struct ConfigEditError {
            message: String,
        }

        impl ConfigEditError {
            pub fn new(message: impl Into<String>) -> Self {
                Self {
                    message: message.into(),
                }
            }
        }

        impl std::fmt::Display for ConfigEditError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.message)
            }
        }

        impl std::error::Error for ConfigEditError {}

        /// 配置编辑的构建器（存根实现）。
        #[derive(Debug, Clone, Default)]
        pub struct ConfigEditsBuilder {
            _resume_cwd: Option<ResumeCwdMode>,
        }

        impl ConfigEditsBuilder {
            pub fn for_config(_config: &crate::tui_core::legacy_core::config::Config) -> Self {
                Self::default()
            }

            pub fn new() -> Self {
                Self::default()
            }

            pub fn set_resume_cwd(mut self, mode: ResumeCwdMode) -> Self {
                self._resume_cwd = Some(mode);
                self
            }

            pub fn build(self) -> Vec<ConfigEdit> {
                Vec::new()
            }

            pub async fn apply(self) -> Result<(), ConfigEditError> {
                Ok(())
            }
        }

        #[derive(Debug, Clone, Default)]
        pub struct ConfigEdit {
            pub path: String,
            pub value: String,
        }
    }

    pub mod permissions {
        use std::path::PathBuf;

        #[derive(Debug, Clone, Default)]
        pub struct Permissions {
            pub file_system_sandbox_policy: FileSystemSandboxPolicy,
            pub network: Option<super::NetworkProxySpec>,
            pub approval_policy: crate::config_compat::ConstrainedWithSource<
                crate::app_server_protocol::AskForApproval,
            >,
            pub approvals_reviewer: crate::config_compat::ConstrainedWithSource<
                crate::config_compat::types::ApprovalsReviewer,
            >,
            pub permission_profile: crate::config_compat::ConstrainedWithSource<String>,
            pub windows_sandbox_mode: String,
        }

        #[derive(Debug, Clone, Default)]
        pub struct FileSystemSandboxPolicy;

        impl FileSystemSandboxPolicy {
            pub fn get_writable_roots_with_cwd(&self, _cwd: &std::path::Path) -> Vec<WritableRoot> {
                Vec::new()
            }
            pub fn has_full_disk_write_access(&self) -> bool {
                false
            }
        }

        #[derive(Debug, Clone, Default)]
        pub struct WritableRoot {
            pub root: PathBuf,
        }

        impl Permissions {
            pub fn effective_permission_profile(
                &self,
            ) -> crate::protocol_compat::models::PermissionProfile {
                crate::protocol_compat::models::PermissionProfile::default()
            }

            pub fn active_permission_profile(
                &self,
            ) -> crate::protocol_compat::models::ActivePermissionProfile {
                crate::protocol_compat::models::ActivePermissionProfile::default()
            }

            pub fn permission_profile(&self) -> crate::protocol_compat::models::PermissionProfile {
                crate::protocol_compat::models::PermissionProfile::default()
            }

            pub fn can_set_permission_profile(
                &self,
                _profile: &crate::protocol_compat::models::PermissionProfile,
            ) -> Result<(), crate::config_compat::ConstraintError> {
                Ok(())
            }

            pub fn is_permission_profile_allowed(
                &self,
                _id: &str,
                _profile: &crate::protocol_compat::models::PermissionProfile,
            ) -> bool {
                true
            }
        }
    }
}
