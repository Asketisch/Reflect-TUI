#[derive(Debug, Clone, Default)]
pub struct StateRuntime;
pub fn log_db() -> StateRuntime {
    StateRuntime::default()
}
