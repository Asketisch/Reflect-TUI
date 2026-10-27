#[derive(Debug, Clone, Copy)]
pub enum ApprovalModeCliArg {
    Auto,
    Always,
    OnRequest,
}
#[derive(Debug, Clone, Default)]
pub struct CliConfigOverrides;
#[derive(Debug, Clone, Default)]
pub struct SharedCliOptions {
    pub approval_mode: Option<ApprovalModeCliArg>,
}
pub fn format_env_display() -> String {
    String::new()
}
pub fn resume_hint() -> Option<String> {
    None
}
