use super::*;

/// 对外公布的“fast”路由的服务层级（service-tier）别名。
pub const SPEED_TIER_FAST: &str = "fast";

/// 模型对外公布的推理级别（reasoning effort）。上游这是一个
/// 经过非空字符串校验的 newtype；stub 保留相同的变体，并通过
/// `Custom` 接受未知值。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
    Custom(String),
}

impl ReasoningEffort {
    pub fn as_str(&self) -> &str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
            Self::Custom(effort) => effort,
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for ReasoningEffort {
    fn from(s: String) -> Self {
        match s.as_str() {
            "none" => Self::None,
            "minimal" => Self::Minimal,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            "max" => Self::Max,
            "ultra" => Self::Ultra,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl From<&str> for ReasoningEffort {
    fn from(s: &str) -> Self {
        match s {
            "none" => Self::None,
            "minimal" => Self::Minimal,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            "max" => Self::Max,
            "ultra" => Self::Ultra,
            other => Self::Custom(other.to_string()),
        }
    }
}

impl Serialize for ReasoningEffort {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReasoningEffort {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "none" => Self::None,
            "minimal" => Self::Minimal,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            "max" => Self::Max,
            "ultra" => Self::Ultra,
            other => Self::Custom(other.to_string()),
        })
    }
}

/// 模型对外公布的规范化用户输入模态（modality）标签。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputModality {
    Text,
    #[default]
    Image,
    Audio,
}

/// 为模型提供的推理级别选项。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningEffortPreset {
    pub effort: ReasoningEffort,
    pub description: String,
}

/// 目录（catalog）为模型对外公布的服务层级。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelServiceTier {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// 描述 Reflect 所支持模型的元数据。stub 仅保留
/// TUI 实际读取的字段；上游未知的字段会被丢弃。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPreset {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub default_reasoning_effort: ReasoningEffort,
    pub supported_reasoning_efforts: Vec<ReasoningEffortPreset>,
    #[serde(default)]
    pub supports_personality: bool,
    #[serde(default)]
    pub additional_speed_tiers: Vec<String>,
    #[serde(default)]
    pub service_tiers: Vec<ModelServiceTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_service_tier: Option<String>,
    pub is_default: bool,
    pub upgrade: Option<ModelUpgrade>,
    pub show_in_picker: bool,
    pub availability_nux: Option<ModelAvailabilityNux>,
    pub supported_in_api: bool,
    #[serde(default)]
    pub input_modalities: Vec<InputModality>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUpgrade {
    pub id: String,
    pub migration_config_key: String,
    pub model_link: Option<String>,
    pub upgrade_copy: Option<String>,
    pub migration_markdown: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAvailabilityNux {
    pub message: String,
}
