pub const DEFAULT_OLLAMA_PORT: u16 = 11434;
pub const DEFAULT_LMSTUDIO_PORT: u16 = 1234;
pub const OLLAMA_OSS_PROVIDER_ID: &str = "ollama";
#[derive(Debug, Clone, Default)]
pub struct ModelProviderInfo;
#[derive(Debug, Clone, Default)]
pub struct ModelProviderAwsAuthInfo;
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum WireApi {
    #[default]
    Chat,
    Responses,
}
