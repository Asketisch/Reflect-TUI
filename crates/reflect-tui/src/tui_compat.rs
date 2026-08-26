#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum ExitReason {
    #[default]
    Normal,
    Error,
}
#[derive(Debug, Clone, Default)]
pub struct AppExitInfo;
#[derive(Debug, Clone, Default)]
pub struct Cli;
pub fn run_main() -> Result<(), String> {
    Ok(())
}
