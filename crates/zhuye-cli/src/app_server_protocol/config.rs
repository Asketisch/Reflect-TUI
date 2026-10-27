//! 配置 / 外部 agent 迁移。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigBatchWriteParams {
    pub edits: Vec<ConfigEdit>,
    pub file_path: Option<String>,
    pub expected_version: Option<String>,
    pub reload_user_config: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigEdit {
    pub key_path: String,
    pub value: serde_json::Value,
    pub merge_strategy: MergeStrategy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MergeStrategy {
    Replace,
    #[default]
    Upsert,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConfigWriteResponse {
    pub status: WriteStatus,
    pub version: String,
    pub file_path: PathBuf,
    pub overridden_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WriteStatus {
    #[default]
    Ok,
    OkOverridden,
}

// ---------------------------------------------------------------------------
// 外部 agent 配置迁移
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalAgentConfigMigrationItem {
    pub item_type: ExternalAgentConfigMigrationItemType,
    pub description: String,
    pub cwd: Option<PathBuf>,
    pub details: Option<MigrationDetails>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ExternalAgentConfigMigrationItemType {
    #[default]
    Plugins,
    Skill,
    Session,
    McpServer,
    Hook,
    ExternalMcpServer,
    ExternalAgent,
    Sessions,
    Subagents,
    Skills,
    Memory,
    McpServerConfig,
    Hooks,
    Config,
    Commands,
    AgentsMd,
    PluginsMigration,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationDetails {
    pub marketplace_name: Option<String>,
    pub plugin_names: Vec<String>,
    pub skill_name: Option<String>,
    pub mcp_server_name: Option<String>,
    pub hook_key: Option<String>,
    pub commands: Vec<MigrationCommandInfo>,
    pub hooks: Vec<MigrationHookInfo>,
    pub mcp_servers: Vec<MigrationMcpServerInfo>,
    pub memory: Vec<String>,
    pub plugins: Vec<PluginsMigration>,
    pub sessions: Vec<MigrationChatSessionInfo>,
    pub skills: Vec<MigrationSkillInfo>,
    pub subagents: Vec<MigrationSubagentInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationSkillInfo {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationMcpServerInfo {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationSubagentInfo {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationHookInfo {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationCommandInfo {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationChatSessionInfo {
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginsMigration {
    pub marketplace_name: String,
    pub plugin_names: Vec<String>,
}
