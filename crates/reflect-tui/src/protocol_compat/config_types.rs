use super::*;

/// 单条用户提供的消息正文的保守上限。对应
/// 上游的 `MAX_USER_INPUT_TEXT_CHARS` 哨兵值，调用方
/// 在显示前用它截断过长的输入。
pub const SERVICE_TIER_DEFAULT_REQUEST_VALUE: &str = "default";

/// 模型是否应输出推理摘要。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    #[default]
    Auto,
    Concise,
    Detailed,
    None,
}

/// Reflect 会话的初始协作模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeKind {
    Plan,
    #[default]
    #[serde(alias = "code", alias = "custom")]
    Default,
    PairProgramming,
    Execute,
}

impl ModeKind {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Plan => "Plan",
            Self::Default => "Default",
            Self::PairProgramming => "Pair Programming",
            Self::Execute => "Execute",
        }
    }

    pub const fn is_tui_visible(self) -> bool {
        matches!(self, Self::Plan | Self::Default)
    }

    pub const fn allows_request_user_input(self) -> bool {
        matches!(self, Self::Plan)
    }
}

/// 由提供方声明的模型路由服务层级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTier {
    Fast,
    Flex,
}

impl ServiceTier {
    pub const fn request_value(self) -> &'static str {
        match self {
            Self::Fast => "priority",
            Self::Flex => "flex",
        }
    }

    pub fn from_request_value(value: &str) -> Option<Self> {
        match value {
            "fast" | "priority" => Some(Self::Fast),
            "flex" => Some(Self::Flex),
            _ => None,
        }
    }
}

/// 由谁来审核路由到人工/子代理审查的审批请求。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalsReviewer {
    #[default]
    #[serde(rename = "user")]
    User,
    #[serde(rename = "auto_review", alias = "guardian_subagent")]
    AutoReview,
}

impl ApprovalsReviewer {
    pub fn to_core(self) -> crate::config_compat::types::ApprovalsReviewer {
        match self {
            Self::User => crate::config_compat::types::ApprovalsReviewer::User,
            Self::AutoReview => crate::config_compat::types::ApprovalsReviewer::AutoReview,
        }
    }
}

/// Windows 专属的沙箱强制强度。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowsSandboxLevel {
    #[default]
    Disabled,
    RestrictedToken,
    Elevated,
}

/// 应用到模型指令的人格选项。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Personality {
    #[default]
    None,
    Friendly,
    Pragmatic,
}

/// 在多种登录方式可用时，由配置强制指定的登录方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForcedLoginMethod {
    Chatgpt,
    Api,
}

/// 支撑 `CollaborationMode` 的设置数据。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Settings {
    pub model: String,
    pub reasoning_effort: Option<crate::protocol_compat::openai_models::ReasoningEffort>,
    pub developer_instructions: Option<String>,
}

/// 协作模式：一个 `ModeKind` 及其模型/力度设置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CollaborationMode {
    pub mode: ModeKind,
    pub settings: Settings,
}

impl CollaborationMode {
    pub fn model(&self) -> &str {
        self.settings.model.as_str()
    }

    pub fn reasoning_effort(
        &self,
    ) -> Option<crate::protocol_compat::openai_models::ReasoningEffort> {
        self.settings.reasoning_effort.clone()
    }

    pub fn with_updates(
        &self,
        model: Option<String>,
        effort: Option<Option<crate::protocol_compat::openai_models::ReasoningEffort>>,
        developer_instructions: Option<Option<String>>,
    ) -> Self {
        let settings = &self.settings;
        let updated_settings = Settings {
            model: model.unwrap_or_else(|| settings.model.clone()),
            reasoning_effort: effort.unwrap_or_else(|| settings.reasoning_effort.clone()),
            developer_instructions: developer_instructions
                .unwrap_or_else(|| settings.developer_instructions.clone()),
        };
        CollaborationMode {
            mode: self.mode,
            settings: updated_settings,
        }
    }

    pub fn apply_mask(&self, mask: &CollaborationModeMask) -> Self {
        let settings = &self.settings;
        CollaborationMode {
            mode: mask.mode.unwrap_or(self.mode),
            settings: Settings {
                model: mask.model.clone().unwrap_or_else(|| settings.model.clone()),
                reasoning_effort: mask
                    .reasoning_effort
                    .clone()
                    .unwrap_or_else(|| settings.reasoning_effort.clone()),
                developer_instructions: mask
                    .developer_instructions
                    .clone()
                    .unwrap_or_else(|| settings.developer_instructions.clone()),
            },
        }
    }
}

/// 应用到 `CollaborationMode` 上的可选覆盖层。每个字段
/// 都是可选的，便于调用方单独修补某项设置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CollaborationModeMask {
    pub name: String,
    pub mode: Option<ModeKind>,
    pub model: Option<String>,
    pub reasoning_effort: Option<Option<crate::protocol_compat::openai_models::ReasoningEffort>>,
    pub developer_instructions: Option<Option<String>>,
}

/// 旧版配置使用的粗粒度沙箱模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    #[serde(rename = "read-only")]
    #[default]
    ReadOnly,
    #[serde(rename = "workspace-write")]
    WorkspaceWrite,
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}
