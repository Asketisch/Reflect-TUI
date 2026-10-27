//! MCP / 工具请求用户输入。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolRequestUserInputOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolRequestUserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub is_other: bool,
    pub is_secret: bool,
    pub options: Option<Vec<ToolRequestUserInputOption>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolRequestUserInputParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub questions: Vec<ToolRequestUserInputQuestion>,
    pub auto_resolution_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRequestUserInputAnswer {
    pub answers: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRequestUserInputResponse {
    pub answers: HashMap<String, ToolRequestUserInputAnswer>,
}

// ---------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpAuthStatus {
    #[default]
    Unsupported,
    NotLoggedIn,
    BearerToken,
    OAuth,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpServerStatusDetail {
    #[default]
    Full,
    ToolsAndAuthOnly,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpResource {
    pub name: String,
    pub uri: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpResourceTemplate {
    pub name: String,
    pub uri_template: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServerStatus {
    pub name: String,
    pub server_info: Option<serde_json::Value>,
    pub tools: HashMap<String, serde_json::Value>,
    pub resources: Vec<McpResource>,
    pub resource_templates: Vec<McpResourceTemplate>,
    pub auth_status: McpAuthStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpServerStartupState {
    #[default]
    Starting,
    Ready,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServerStatusUpdatedNotification {
    pub thread_id: Option<String>,
    pub name: String,
    pub status: McpServerStartupState,
    pub error: Option<String>,
    pub failure_reason: Option<McpServerStartupFailureReason>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpServerStartupFailureReason {
    #[default]
    ReauthenticationRequired,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpServerElicitationAction {
    #[default]
    Accept,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServerElicitationRequestParams {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub server_name: String,
    pub request: McpServerElicitationRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerElicitationRequest {
    Form {
        meta: Option<serde_json::Value>,
        message: String,
        requested_schema: McpElicitationSchema,
    },
    OpenAiForm {
        meta: Option<serde_json::Value>,
        message: String,
        requested_schema: serde_json::Value,
    },
    Url {
        meta: Option<serde_json::Value>,
        message: String,
        url: String,
        elicitation_id: String,
    },
}

impl Default for McpServerElicitationRequest {
    fn default() -> Self {
        McpServerElicitationRequest::Url {
            meta: None,
            message: String::new(),
            url: String::new(),
            elicitation_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpElicitationSchema {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema_uri: Option<String>,
    #[serde(rename = "type", default)]
    pub type_: McpElicitationObjectType,
    #[serde(default)]
    pub properties: std::collections::BTreeMap<String, McpElicitationPrimitiveSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpElicitationObjectType {
    #[default]
    Object,
}

/// `McpElicitationStringSchema`（`"type": "string"`）的判别字段。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpElicitationStringType {
    #[default]
    String,
}

/// `McpElicitationNumberSchema`（`"type": "number"`/`"integer"`）的判别字段。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpElicitationNumberType {
    #[default]
    Number,
    Integer,
}

/// `McpElicitationBooleanSchema`（`"type": "boolean"`）的判别字段。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpElicitationBooleanType {
    #[default]
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpElicitationStringSchema {
    #[serde(rename = "type")]
    pub type_: McpElicitationStringType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpElicitationNumberSchema {
    #[serde(rename = "type")]
    pub type_: McpElicitationNumberType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpElicitationBooleanSchema {
    #[serde(rename = "type")]
    pub type_: McpElicitationBooleanType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpElicitationEnumSchemaFields {
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "default")]
    pub default: Option<String>,
    #[serde(rename = "enum")]
    pub enum_: Vec<String>,
    #[serde(rename = "enumNames")]
    pub enum_names: Option<Vec<String>>,
    #[serde(rename = "oneOf")]
    pub one_of: Vec<McpElicitationEnumOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpElicitationEnumOption {
    pub title: Option<String>,
    #[serde(rename = "const")]
    pub const_: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum McpElicitationPrimitiveSchema {
    Enum(McpElicitationEnumSchema),
    Boolean(McpElicitationBooleanSchema),
    Number(McpElicitationNumberSchema),
    String(McpElicitationStringSchema),
}

impl Default for McpElicitationPrimitiveSchema {
    fn default() -> Self {
        McpElicitationPrimitiveSchema::String(McpElicitationStringSchema {
            type_: McpElicitationStringType::String,
            title: None,
            description: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpElicitationEnumSchema {
    SingleSelect(McpElicitationSingleSelectEnumSchema),
    MultiSelect(McpElicitationEnumSchemaFields),
    Legacy(McpElicitationEnumSchemaFields),
}

impl Default for McpElicitationEnumSchema {
    fn default() -> Self {
        McpElicitationEnumSchema::MultiSelect(McpElicitationEnumSchemaFields {
            title: None,
            description: None,
            default: None,
            enum_: Vec::new(),
            enum_names: None,
            one_of: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpElicitationSingleSelectEnumSchema {
    Untitled(McpElicitationEnumSchemaFields),
    Titled(McpElicitationEnumSchemaFields),
}

impl Default for McpElicitationSingleSelectEnumSchema {
    fn default() -> Self {
        McpElicitationSingleSelectEnumSchema::Untitled(McpElicitationEnumSchemaFields {
            title: None,
            description: None,
            default: None,
            enum_: Vec::new(),
            enum_names: None,
            one_of: Vec::new(),
        })
    }
}
