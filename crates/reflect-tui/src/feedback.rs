//! reflect_feedback crate 的桩实现。

pub const REFLECT_APP_DIRECTORY_CACHE_ATTACHMENT_FILENAME: &str = "app_directory.json";
pub const REFLECT_APPS_TOOLS_CACHE_ATTACHMENT_FILENAME: &str = "apps_tools.json";
pub const DOCTOR_REPORT_ATTACHMENT_FILENAME: &str = "doctor_report.json";
pub const FEEDBACK_DIAGNOSTICS_ATTACHMENT_FILENAME: &str = "diagnostics.json";
pub const WINDOWS_SANDBOX_LOG_ATTACHMENT_FILENAME: &str = "windows_sandbox_log.txt";

#[derive(Debug, Clone, Default)]
pub struct ReflectFeedback;

#[derive(Debug, Clone, Default)]
pub struct FeedbackDiagnostics;

impl FeedbackDiagnostics {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone, Default)]
pub struct FeedbackDiagnostic {
    pub headline: String,
    pub details: Vec<String>,
}

impl FeedbackDiagnostic {
    pub fn headline(&self) -> String {
        self.headline.clone()
    }
}

#[derive(Debug, Clone, Default)]
pub struct FeedbackSubmission;

impl FeedbackDiagnostics {
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl FeedbackDiagnostics {
    pub fn diagnostics(&self) -> Vec<FeedbackDiagnostic> {
        Vec::new()
    }
}

impl ReflectFeedback {
    pub fn snapshot(&self) -> FeedbackDiagnostics {
        FeedbackDiagnostics::new()
    }
}
