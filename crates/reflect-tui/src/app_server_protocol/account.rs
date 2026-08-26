//! 鉴权 / 账户 / 登录流程。
//!
//! app_server_protocol 协议存根的子模块：按领域拆分自原 app_server_protocol.rs。
//! 此处仅最小化重声明以编译 UI。

use super::*;

/// 已配置模型提供方的认证模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    ApiKey,
    #[default]
    Chatgpt,
    ChatgptAuthTokens,
    Headers,
    AgentIdentity,
    PersonalAccessToken,
    BedrockApiKey,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountUpdatedNotification {
    pub auth_mode: Option<AuthMode>,
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountLoginCompletedNotification {
    pub login_id: Option<String>,
    pub error: String,
    pub success: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountTokenUsageSummary {
    pub lifetime_tokens: Option<i64>,
    pub peak_daily_tokens: Option<i64>,
    pub longest_running_turn_sec: Option<i64>,
    pub current_streak_days: Option<i64>,
    pub longest_streak_days: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountTokenUsageDailyBucket {
    pub start_date: String,
    pub tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GetWorkspaceMessagesResponse {
    pub feature_enabled: bool,
    pub messages: Vec<WorkspaceMessage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceMessage {
    pub message_id: String,
    pub message_type: WorkspaceMessageType,
    pub message_body: String,
    pub created_at: Option<i64>,
    pub archived_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkspaceMessageType {
    #[default]
    Headline,
    Announcement,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GetAccountTokenUsageResponse {
    pub summary: AccountTokenUsageSummary,
    pub daily_usage_buckets: Option<Vec<AccountTokenUsageDailyBucket>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AddCreditsNudgeCreditType {
    #[default]
    Credits,
    UsageLimit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AddCreditsNudgeEmailStatus {
    #[default]
    Sent,
    CooldownActive,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConsumeAccountRateLimitResetCreditOutcome {
    Reset,
    #[default]
    NothingToReset,
    NoCredit,
    AlreadyRedeemed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsumeAccountRateLimitResetCreditResponse {
    pub outcome: ConsumeAccountRateLimitResetCreditOutcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateLimitSnapshot {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub primary: Option<RateLimitWindow>,
    pub secondary: Option<RateLimitWindow>,
    pub credits: Option<CreditsSnapshot>,
    pub individual_limit: Option<SpendControlLimitSnapshot>,
    pub spend_control_reached: Option<bool>,
    pub plan_type: Option<String>,
    pub rate_limit_reached_type: Option<RateLimitReachedType>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GetAccountRateLimitsResponse {
    pub rate_limits: RateLimitSnapshot,
    pub rate_limits_by_limit_id: Option<HashMap<String, RateLimitSnapshot>>,
    pub rate_limit_reset_credits: Option<RateLimitResetCreditsSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateLimitResetCreditsSummary {
    pub available_count: i64,
    pub credits: Option<Vec<RateLimitResetCredit>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateLimitResetCredit {
    pub id: String,
    pub reset_type: RateLimitResetType,
    pub status: RateLimitResetCreditStatus,
    pub granted_at: i64,
    pub expires_at: Option<i64>,
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RateLimitResetType {
    #[default]
    ReflectRateLimits,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RateLimitResetCreditStatus {
    #[default]
    Available,
    Redeeming,
    Redeemed,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateLimitWindow {
    pub used_percent: i32,
    pub window_duration_mins: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpendControlLimitSnapshot {
    pub limit: String,
    pub used: String,
    pub remaining_percent: i32,
    pub resets_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RateLimitReachedType {
    #[default]
    RateLimitReached,
    WorkspaceOwnerCreditsDepleted,
    WorkspaceMemberCreditsDepleted,
    WorkspaceOwnerUsageLimitReached,
    WorkspaceMemberUsageLimitReached,
}

// ---------------------------------------------------------------------------
// 登录流程
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginAccountParams {
    ApiKey {
        api_key: String,
    },
    Chatgpt {
        reflect_streamlined_login: bool,
        use_hosted_login_success_page: bool,
        app_brand: Option<LoginAppBrand>,
    },
    ChatgptDeviceCode,
    ChatgptAuthTokens {
        access_token: String,
        chatgpt_account_id: String,
        chatgpt_plan_type: Option<String>,
    },
    AmazonBedrock {
        api_key: String,
        region: String,
    },
}

impl Default for LoginAccountParams {
    fn default() -> Self {
        LoginAccountParams::Chatgpt {
            reflect_streamlined_login: false,
            use_hosted_login_success_page: false,
            app_brand: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LoginAppBrand {
    #[default]
    Reflect,
    Chatgpt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LoginAccountResponse {
    #[default]
    ApiKey,
    Chatgpt {
        login_id: String,
        auth_url: String,
    },
    ChatgptDeviceCode {
        login_id: String,
        verification_url: String,
        user_code: String,
    },
    ChatgptAuthTokens,
    AmazonBedrock,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CancelLoginAccountParams {
    pub login_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CancelLoginAccountResponse {
    pub status: CancelLoginAccountStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CancelLoginAccountStatus {
    #[default]
    Canceled,
    NotFound,
}
