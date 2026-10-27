use super::*;

/// 已配置模型提供方的认证模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    #[default]
    ApiKey,
    Chatgpt,
    #[serde(rename = "chatgptAuthTokens")]
    ChatgptAuthTokens,
    #[serde(rename = "headers")]
    Headers,
    #[serde(rename = "agentIdentity")]
    AgentIdentity,
    #[serde(rename = "personalAccessToken")]
    PersonalAccessToken,
    #[serde(rename = "bedrockApiKey")]
    BedrockApiKey,
}

impl AuthMode {
    pub fn has_chatgpt_account(self) -> bool {
        matches!(
            self,
            Self::Chatgpt | Self::ChatgptAuthTokens | Self::PersonalAccessToken
        )
    }

    pub fn uses_reflect_backend(self) -> bool {
        matches!(
            self,
            Self::Chatgpt
                | Self::ChatgptAuthTokens
                | Self::Headers
                | Self::AgentIdentity
                | Self::PersonalAccessToken
        )
    }
}
