//! app link 视图 URL/host 校验辅助函数簇。从 app_link_view.rs 抽出。

use super::*;

pub(super) fn validate_external_url(url: &str, require_chatgpt_host: bool) -> Option<Url> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return None;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    if require_chatgpt_host && !is_allowed_chatgpt_auth_host(parsed.host_str()?) {
        return None;
    }
    Some(parsed)
}

pub(super) fn is_allowed_chatgpt_auth_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "chatgpt.com"
        || host == "chatgpt-staging.com"
        || host.ends_with(".chatgpt.com")
        || host.ends_with(".chatgpt-staging.com")
}
