use super::*;

/// 由代理编写、带阶段标记的内容载荷。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentMessageContent {
    Text { text: String },
}

/// 回合条目流中由助手编写的消息载荷。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessageItem {
    pub id: String,
    pub content: Vec<AgentMessageContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<crate::protocol_compat::models::MessagePhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_citation: Option<crate::protocol_compat::memory_citation::MemoryCitation>,
}

/// 回合条目流中由用户编写的消息载荷。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMessageItem {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default)]
    pub content: Vec<serde_json::Value>,
}
