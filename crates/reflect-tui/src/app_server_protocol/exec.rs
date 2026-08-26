//! 命令执行 / 审查 / 用户输入。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandExecParams {
    pub command: Vec<String>,
    pub process_id: Option<String>,
    pub tty: bool,
    pub stream_stdin: bool,
    pub stream_stdout_stderr: bool,
    pub output_bytes_cap: Option<u64>,
    pub disable_output_cap: bool,
    pub disable_timeout: bool,
    pub timeout_ms: Option<u64>,
    pub cwd: Option<PathBuf>,
    pub env: Option<HashMap<String, String>>,
    pub size: Option<CommandExecTerminalSize>,
    pub sandbox_policy: Option<SandboxPolicy>,
    pub permission_profile: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandExecTerminalSize {
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

// ---------------------------------------------------------------------------
// 审查
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewTarget {
    UncommittedChanges,
    BaseBranch { branch: String },
    Commit { sha: String, title: Option<String> },
    Custom { instructions: String },
}

impl Default for ReviewTarget {
    fn default() -> Self {
        ReviewTarget::UncommittedChanges
    }
}

// ---------------------------------------------------------------------------
// 用户输入
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserInput {
    Text {
        text: String,
        text_elements: Vec<TextElement>,
    },
    Image {
        detail: Option<String>,
        url: String,
    },
    LocalImage {
        detail: Option<String>,
        path: PathBuf,
    },
    Audio {
        url: String,
    },
    LocalAudio {
        path: PathBuf,
    },
    Skill {
        name: String,
        path: PathBuf,
    },
    Mention {
        name: String,
        path: String,
    },
}

impl Default for UserInput {
    fn default() -> Self {
        UserInput::Text {
            text: String::new(),
            text_elements: Vec::new(),
        }
    }
}

impl UserInput {
    /// 存根
    pub fn text_char_count(&self) -> usize {
        match self {
            UserInput::Text { text, .. } => text.chars().count(),
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextElement {
    pub byte_range: ByteRange,
    pub placeholder: Option<String>,
}

impl From<crate::protocol_compat::user_input::TextElement> for TextElement {
    fn from(v: crate::protocol_compat::user_input::TextElement) -> Self {
        Self {
            byte_range: ByteRange {
                start: v.byte_range.start,
                end: v.byte_range.end,
            },
            placeholder: v.placeholder,
        }
    }
}

impl TextElement {
    /// 存根
    pub fn new(byte_range: ByteRange, placeholder: Option<String>) -> Self {
        Self {
            byte_range,
            placeholder,
        }
    }

    /// 存根
    pub fn set_placeholder(&mut self, placeholder: Option<String>) {
        self.placeholder = placeholder;
    }

    /// 存根
    pub fn placeholder(&self) -> Option<&str> {
        self.placeholder.as_deref()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}
