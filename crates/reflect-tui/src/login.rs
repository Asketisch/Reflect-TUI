//! reflect_login crate 的桩。

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AuthMode {
    #[default]
    ApiKey,
    Chatgpt,
    ChatgptAuthTokens,
    Headers,
    AgentIdentity,
    PersonalAccessToken,
    BedrockApiKey,
}

impl AuthMode {
    pub fn has_chatgpt_account(&self) -> bool {
        matches!(self, Self::Chatgpt | Self::ChatgptAuthTokens)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LoginStatus {
    NotAuthenticated,
    AuthMode(AuthMode),
    #[default]
    NotLoggedIn,
    LoggingIn,
    LoggedIn,
    Error(String),
}

/// read_openai_api_key_from_env 的桩。
pub fn read_openai_api_key_from_env() -> Option<String> {
    std::env::var("OPENAI_API_KEY").ok()
}
