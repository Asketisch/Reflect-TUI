use super::*;

/// HTTP 响应返回非预期状态码时产生的错误。
///
/// 上游存储的是 `reqwest::StatusCode`；桩实现将其保留为普通
/// `u16`，使本 crate 不依赖 `reqwest`。
#[derive(Debug, Clone, Default)]
pub struct UnexpectedResponseError {
    pub status: u16,
    pub body: String,
    pub user_message: Option<String>,
    pub url: Option<String>,
    pub cf_ray: Option<String>,
    pub request_id: Option<String>,
    pub identity_authorization_error: Option<String>,
    pub identity_error_code: Option<String>,
}

impl fmt::Display for UnexpectedResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(user_message) = &self.user_message {
            f.write_str(user_message)
        } else {
            write!(f, "unexpected status {}: {}", self.status, self.body)
        }
    }
}

impl std::error::Error for UnexpectedResponseError {}
