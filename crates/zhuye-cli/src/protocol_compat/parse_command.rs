use super::*;

/// 尽最大努力将 shell 命令归类为 read（读取）/ list（列出）/
/// search（搜索）/ unknown（未知）之一。上游借此实现了 exec
/// 单元格渲染器中的“只读命令”优化。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParsedCommand {
    Read {
        cmd: String,
        name: String,
        path: String,
    },
    ListFiles {
        cmd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Search {
        cmd: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Unknown {
        cmd: String,
    },
}

impl Default for ParsedCommand {
    fn default() -> Self {
        Self::Unknown { cmd: String::new() }
    }
}
