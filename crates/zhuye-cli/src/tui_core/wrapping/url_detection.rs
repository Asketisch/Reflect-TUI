//! URL 检测辅助函数簇：token/url/host/port/domain 判定。从 wrapping.rs 抽出。

pub(super) fn text_has_mixed_url_and_non_url_tokens(text: &str) -> bool {
    let mut saw_url = false;
    let mut saw_non_url = false;

    for raw_token in text.split_ascii_whitespace() {
        if is_url_like_token(raw_token) {
            saw_url = true;
        } else if is_substantive_non_url_token(raw_token) {
            saw_non_url = true;
        }

        if saw_url && saw_non_url {
            return true;
        }
    }

    false
}

/// 判定单个空白分隔的标记是否像 URL。
///
/// 先剥离两侧的标点，然后检查是否为绝对 URL（含 `://`）
/// 或裸域名 URL（可识别的主机 + 路径/查询/片段）。
pub(super) fn is_url_like_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    !token.is_empty() && (is_absolute_url_like(token) || is_bare_url_like(token))
}

pub(super) fn is_substantive_non_url_token(raw_token: &str) -> bool {
    let token = trim_url_token(raw_token);
    if token.is_empty() || is_decorative_marker_token(raw_token, token) {
        return false;
    }

    token.chars().any(char::is_alphanumeric)
}

pub(super) fn is_decorative_marker_token(raw_token: &str, token: &str) -> bool {
    let raw = raw_token.trim();
    matches!(
        raw,
        "-" | "*"
            | "+"
            | "•"
            | "◦"
            | "▪"
            | ">"
            | "|"
            | "│"
            | "┆"
            | "└"
            | "├"
            | "┌"
            | "┐"
            | "┘"
            | "┼"
    ) || is_ordered_list_marker(raw, token)
}

pub(super) fn is_ordered_list_marker(raw_token: &str, token: &str) -> bool {
    token.chars().all(|c| c.is_ascii_digit())
        && (raw_token.ends_with('.') || raw_token.ends_with(')'))
}

pub(super) fn trim_url_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ','
                | '.'
                | ';'
                | ':'
                | '!'
                | '\''
                | '"'
        )
    })
}

/// 检查 `scheme://host` 模式。对于常见的 scheme 使用 `url::Url::parse`；
/// 对于 `url` crate 拒绝的自定义 scheme，回退到 `has_valid_scheme_prefix`。
pub(super) fn is_absolute_url_like(token: &str) -> bool {
    if !token.contains("://") {
        return false;
    }

    if let Ok(url) = url::Url::parse(token) {
        let scheme = url.scheme().to_ascii_lowercase();
        if matches!(
            scheme.as_str(),
            "http" | "https" | "ftp" | "ftps" | "ws" | "wss"
        ) {
            return url.host_str().is_some();
        }
        return true;
    }

    has_valid_scheme_prefix(token)
}

pub(super) fn has_valid_scheme_prefix(token: &str) -> bool {
    let Some((scheme, rest)) = token.split_once("://") else {
        return false;
    };
    if scheme.is_empty() || rest.is_empty() {
        return false;
    }

    let mut chars = scheme.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

/// 检查不带 scheme 的裸域名 URL：`host[:port]/path`、
/// `host[:port]?query` 或 `host[:port]#fragment`。
///
/// 要求主机是 `localhost`、IPv4 地址或合法域名。
/// 没有路径/查询/片段的裸 `host.tld` 仅当主机以 `www.` 开头时才接受。
///
/// 有意不处理 IPv6 方括号记法（`[::1]:8080`）。
pub(super) fn is_bare_url_like(token: &str) -> bool {
    let (host_port, has_trailer) = split_host_port_and_trailer(token);
    if host_port.is_empty() {
        return false;
    }

    // 裸主机必须有类似 URL 的尾部，除非标记以 www 开头。
    if !has_trailer && !host_port.to_ascii_lowercase().starts_with("www.") {
        return false;
    }

    let (host, port) = split_host_and_port(host_port);
    if host.is_empty() {
        return false;
    }
    if let Some(port) = port
        && !is_valid_port(port)
    {
        return false;
    }

    host.eq_ignore_ascii_case("localhost") || is_ipv4(host) || is_domain_name(host)
}

pub(super) fn split_host_port_and_trailer(token: &str) -> (&str, bool) {
    if let Some(idx) = token.find(['/', '?', '#']) {
        (&token[..idx], true)
    } else {
        (token, false)
    }
}

pub(super) fn split_host_and_port(host_port: &str) -> (&str, Option<&str>) {
    // 我们在第一轮判定中有意不把带方括号的 IPv6 视为 URL。
    if host_port.starts_with('[') {
        return (host_port, None);
    }

    if let Some((host, port)) = host_port.rsplit_once(':')
        && !host.is_empty()
        && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
    {
        return (host, Some(port));
    }

    (host_port, None)
}

pub(super) fn is_valid_port(port: &str) -> bool {
    if port.is_empty() || port.len() > 5 || !port.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    port.parse::<u16>().is_ok()
}

pub(super) fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return false;
    }

    parts
        .iter()
        .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

pub(super) fn is_domain_name(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if !host.contains('.') {
        return false;
    }

    let mut labels = host.split('.');
    let Some(tld) = labels.next_back() else {
        return false;
    };
    if !is_tld(tld) {
        return false;
    }

    labels.all(is_domain_label)
}

pub(super) fn is_tld(label: &str) -> bool {
    (2..=63).contains(&label.len()) && label.chars().all(|c| c.is_ascii_alphabetic())
}

pub(super) fn is_domain_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }

    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(last) = label.chars().next_back() else {
        return false;
    };

    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
