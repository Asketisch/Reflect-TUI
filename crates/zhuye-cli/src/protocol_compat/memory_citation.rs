use super::*;

/// 指向记忆库文件区间的单条引用条目。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCitationEntry {
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub note: String,
}

/// 附加到代理消息上的记忆引用集合。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryCitation {
    pub entries: Vec<MemoryCitationEntry>,
    #[serde(default)]
    pub rollout_ids: Vec<String>,
}
