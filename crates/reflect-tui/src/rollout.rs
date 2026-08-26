#[derive(Debug, Clone, Default)]
pub struct StateDbHandle;
pub fn state_db() -> StateDbHandle {
    StateDbHandle::default()
}
pub fn open_rollout_line_reader(_path: &std::path::Path) -> Option<String> {
    None
}
