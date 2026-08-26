use super::*;

/// 模型提供商公布的订阅套餐类型。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanType {
    #[default]
    Free,
    Go,
    Plus,
    Pro,
    ProLite,
    Team,
    #[serde(rename = "self_serve_business_usage_based")]
    SelfServeBusinessUsageBased,
    Business,
    #[serde(rename = "enterprise_cbp_usage_based")]
    EnterpriseCbpUsageBased,
    Enterprise,
    Edu,
    #[serde(other)]
    Unknown,
}

impl PlanType {
    pub fn is_team_like(self) -> bool {
        matches!(self, Self::Team | Self::SelfServeBusinessUsageBased)
    }

    pub fn is_business_like(self) -> bool {
        matches!(self, Self::Business | Self::EnterpriseCbpUsageBased)
    }

    pub fn is_workspace_account(self) -> bool {
        matches!(
            self,
            Self::Team
                | Self::SelfServeBusinessUsageBased
                | Self::Business
                | Self::EnterpriseCbpUsageBased
                | Self::Enterprise
                | Self::Edu
        )
    }

    pub fn from_str_lossy(s: &str) -> Option<Self> {
        Some(match s {
            "free" => Self::Free,
            "go" => Self::Go,
            "plus" => Self::Plus,
            "pro" => Self::Pro,
            "pro_lite" => Self::ProLite,
            "team" => Self::Team,
            "self_serve_business_usage_based" => Self::SelfServeBusinessUsageBased,
            "business" => Self::Business,
            "enterprise_cbp_usage_based" => Self::EnterpriseCbpUsageBased,
            "enterprise" => Self::Enterprise,
            "edu" => Self::Edu,
            _ => return None,
        })
    }
}

/// 在适配为面向应用的线上类型之前返回的提供商账户快照。
/// 该桩类型只保留 tooltip 渲染所需的字段形状。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderAccount {
    pub email: Option<String>,
    pub plan_type: PlanType,
}

/// 展示给 TUI 的账户信息。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountInfo {
    pub plan_type: PlanType,
    pub email: Option<String>,
}
