#[derive(Debug)]
pub struct SleepInhibitor;
impl SleepInhibitor {
    pub fn new(_enabled: bool) -> Self {
        Self
    }
    pub fn inhibit(&self) {}
    pub fn release(&self) {}
}

impl SleepInhibitor {
    pub fn set_turn_running(&self, _running: bool) {}
}
