//! Markdown 本地链接解析子系统。从 markdown_render/mod.rs 抽出。

use super::*;

pub(super) fn is_local_path_like_link(dest_url: &str) -> bool {
    dest_url.starts_with("file://")
        || dest_url.starts_with('/')
        || dest_url.starts_with("~/")
        || dest_url.starts_with("./")
        || dest_url.starts_with("../")
        || dest_url.starts_with("\\\\")
        || matches!(
            dest_url.as_bytes(),
            [drive, b':', separator, ..]
                if drive.is_ascii_alphabetic() && matches!(separator, b'/' | b'\\')
        )
}

/// 将本地链接目标解析为规范化路径文本加上可选的位置后缀。
///
/// 此函数接受 Reflect 当前产出的路径形式：`file://` URL、绝对路径与相对路径、
/// `~/...`、Windows 路径，以及 `#L..C..` 或 `:line:col` 后缀。
pub(super) fn render_local_link_target(dest_url: &str, cwd: Option<&Path>) -> Option<String> {
    let (path_text, location_suffix) = parse_local_link_target(dest_url)?;
    let mut rendered = display_local_link_path(&path_text, cwd);
    if let Some(location_suffix) = location_suffix {
        rendered.push_str(&location_suffix);
    }
    Some(rendered)
}

/// 将本地链接的目标拆分为 `(normalized_path_text, location_suffix)`。
///
/// 返回的路径文本不包含尾部的 `#L..` 或 `:line[:col]` 后缀。路径规范化会在可行时展开
/// `~/...`，并将路径分隔符改写为显示稳定的前向斜杠。后缀若存在，则以规范化 markdown
/// 形式单独返回。
///
/// 仅当目标看起来像 `file://` URL 但无法解析为本地路径时返回 `None`。纯路径形式的输入
/// 即使是相对的，也始终返回 `Some(...)`。
pub(super) fn parse_local_link_target(dest_url: &str) -> Option<(String, Option<String>)> {
    if dest_url.starts_with("file://") {
        let url = Url::parse(dest_url).ok()?;
        let path_text = file_url_to_local_path_text(&url)?;
        let location_suffix = url
            .fragment()
            .and_then(normalize_hash_location_suffix_fragment);
        return Some((path_text, location_suffix));
    }

    let mut path_text = dest_url;
    let mut location_suffix = None;
    // 当两种形式同时存在时，优先选择 `#L..` 形式的片段，避免 `path#L10` 这类 URL
    // 被误解析为以 `:10` 结尾的纯路径。
    if let Some((candidate_path, fragment)) = dest_url.rsplit_once('#')
        && let Some(normalized) = normalize_hash_location_suffix_fragment(fragment)
    {
        path_text = candidate_path;
        location_suffix = Some(normalized);
    }
    if location_suffix.is_none()
        && let Some(suffix) = extract_colon_location_suffix(path_text)
    {
        let path_len = path_text.len().saturating_sub(suffix.len());
        path_text = &path_text[..path_len];
        location_suffix = Some(suffix);
    }

    let decoded_path_text =
        urlencoding::decode(path_text).unwrap_or(std::borrow::Cow::Borrowed(path_text));
    Some((expand_local_link_path(&decoded_path_text), location_suffix))
}

/// 将 `L12` 或 `L12C3-L14C9` 这类 hash 片段规范化为要渲染的显示后缀。
///
/// 对于非位置引用的片段返回 `None`。此处刻意忽略其他 `#...` 片段，使非位置的 hash
/// 保留在路径文本中。
pub(super) fn normalize_hash_location_suffix_fragment(fragment: &str) -> Option<String> {
    HASH_LOCATION_SUFFIX_RE
        .is_match(fragment)
        .then(|| format!("#{fragment}"))
        .and_then(|suffix| normalize_markdown_hash_location_suffix(&suffix))
}

/// 从纯路径形式的字符串中提取尾部的 `:line`、`:line:col` 或区间后缀。
///
/// 后缀必须出现在输入的末尾；路径中其他位置的冒号保持不变。这正是避免 Windows
/// 盘符（如 `C:/...`）被误读为位置的关键。
pub(super) fn extract_colon_location_suffix(path_text: &str) -> Option<String> {
    COLON_LOCATION_SUFFIX_RE
        .find(path_text)
        .filter(|matched| matched.end() == path_text.len())
        .map(|matched| matched.as_str().to_string())
}

/// 展开 home 相对路径并规范化分隔符以便显示。
///
/// 如果由于 home 目录不可用而无法展开 `~/...`，原文本仍会经过分隔符规范化，
/// 否则按原样返回。
pub(super) fn expand_local_link_path(path_text: &str) -> String {
    // 主动展开 `~/...`，让 home 相对链接与绝对链接走相同的规范化
    // 以及基于 cwd 的缩短路径。
    if let Some(rest) = path_text.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return normalize_local_link_path_text(&home.join(rest).to_string_lossy());
    }

    normalize_local_link_path_text(path_text)
}

/// 将 `file://` URL 转换为用于转写渲染的规范化本地路径文本。
///
/// 标准 file URL 优先使用 `Url::to_file_path()`。当它拒绝 Windows 风格的编码时，
/// 我们从 host/path 部分重建显示路径，使 UNC 路径和盘符 URL 也能合理渲染。
pub(super) fn file_url_to_local_path_text(url: &Url) -> Option<String> {
    if let Ok(path) = url.to_file_path() {
        return Some(normalize_local_link_path_text(&path.to_string_lossy()));
    }

    // 对 `to_file_path()` 拒绝的情况回退到字符串重建，尤其是 URL 形式的 UNC 风格
    // host 与 Windows 盘符路径。
    let mut path_text = url.path().to_string();
    if let Some(host) = url.host_str()
        && !host.is_empty()
        && host != "localhost"
    {
        path_text = format!("//{host}{path_text}");
    } else if matches!(
        path_text.as_bytes(),
        [b'/', drive, b':', b'/', ..] if drive.is_ascii_alphabetic()
    ) {
        path_text.remove(0);
    }

    Some(normalize_local_link_path_text(&path_text))
}

/// 将本地路径文本规范化成转写显示形式。
///
/// 显示规范化刻意保持为词法层面：它不触碰文件系统、不解析符号链接，也不折叠 `.` / `..`。
/// 它只将分隔符转换为前向斜杠，并把 UNC 风格的 `\\\\server\\share` 输入改写为
/// `//server/share`，使后续的前缀检查能在稳定的表示上进行。
pub(super) fn normalize_local_link_path_text(path_text: &str) -> String {
    // 始终以前向斜杠渲染所有本地链接路径，使显示和前缀剥离在混合 Windows 与 Unix
    // 风格的输入下保持稳定。
    if let Some(rest) = path_text.strip_prefix("\\\\") {
        format!("//{}", rest.replace('\\', "/").trim_start_matches('/'))
    } else {
        path_text.replace('\\', "/")
    }
}

pub(super) fn is_absolute_local_link_path(path_text: &str) -> bool {
    path_text.starts_with('/')
        || path_text.starts_with("//")
        || matches!(
            path_text.as_bytes(),
            [drive, b':', b'/', ..] if drive.is_ascii_alphabetic()
        )
}

/// 移除本地路径的尾部分隔符，但不破坏根目录语义。
///
/// 像 `/`、`//`、`C:/` 这样的根保持原样，使调用者仍能区分“根目录本身”和“根目录下的路径”。
pub(super) fn trim_trailing_local_path_separator(path_text: &str) -> &str {
    if path_text == "/" || path_text == "//" {
        return path_text;
    }
    if matches!(path_text.as_bytes(), [drive, b':', b'/'] if drive.is_ascii_alphabetic()) {
        return path_text;
    }
    path_text.trim_end_matches('/')
}

/// 当 `path_text` 严格位于 `cwd_text` 之下时，从其起始处剥离 `cwd_text`。
///
/// 返回不带前导斜杠的相对剩余部分。如果路径恰好等于 cwd，则返回 `None`，
/// 让调用者继续渲染完整路径，而不是折叠为空字符串。
pub(super) fn strip_local_path_prefix<'a>(path_text: &'a str, cwd_text: &str) -> Option<&'a str> {
    let path_text = trim_trailing_local_path_separator(path_text);
    let cwd_text = trim_trailing_local_path_separator(cwd_text);
    if path_text == cwd_text {
        return None;
    }

    // 特殊处理文件系统根，使 `/` 下的 `/tmp/x` 变为 `tmp/x`，
    // 而不是被通用前缀剥离分支原样保留。
    if cwd_text == "/" || cwd_text == "//" {
        return path_text.strip_prefix('/');
    }

    path_text
        .strip_prefix(cwd_text)
        .and_then(|rest| rest.strip_prefix('/'))
}

/// 在规范化之后选择本地链接的可见路径文本。
///
/// 相对路径保持相对；绝对路径仅在词法上位于 `cwd` 之下时才被缩短，否则保留绝对路径。
/// 这只是显示逻辑，并非文件系统规范化。
pub(super) fn display_local_link_path(path_text: &str, cwd: Option<&Path>) -> String {
    let path_text = normalize_local_link_path_text(path_text);
    if !is_absolute_local_link_path(&path_text) {
        return path_text;
    }

    if let Some(cwd) = cwd {
        // 仅缩短位于所提供的会话 cwd 之下的绝对路径；其他情况保留原绝对目标，
        // 以保证清晰可读。
        let cwd_text = normalize_local_link_path_text(&cwd.to_string_lossy());
        if let Some(stripped) = strip_local_path_prefix(&path_text, &cwd_text) {
            return stripped.to_string();
        }
    }

    path_text
}
