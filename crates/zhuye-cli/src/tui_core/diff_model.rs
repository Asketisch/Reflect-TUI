//! 供 TUI diff 渲染与审批预览使用的最小化文件变更模型。

use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum FileChange {
    Add {
        content: String,
    },
    Delete {
        content: String,
    },
    Update {
        unified_diff: String,
        move_path: Option<PathBuf>,
    },
}
