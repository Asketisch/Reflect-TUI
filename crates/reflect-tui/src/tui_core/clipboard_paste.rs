use std::path::Path;
use std::path::PathBuf;
use tempfile::Builder;

#[derive(Debug, Clone)]
pub enum PasteImageError {
    ClipboardUnavailable(String),
    NoImage(String),
    EncodeFailed(String),
    IoError(String),
}

impl std::fmt::Display for PasteImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasteImageError::ClipboardUnavailable(msg) => write!(f, "clipboard unavailable: {msg}"),
            PasteImageError::NoImage(msg) => write!(f, "no image on clipboard: {msg}"),
            PasteImageError::EncodeFailed(msg) => write!(f, "could not encode image: {msg}"),
            PasteImageError::IoError(msg) => write!(f, "io error: {msg}"),
        }
    }
}
impl std::error::Error for PasteImageError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedImageFormat {
    Png,
    Jpeg,
    Other,
}

impl EncodedImageFormat {
    pub fn label(self) -> &'static str {
        match self {
            EncodedImageFormat::Png => "PNG",
            EncodedImageFormat::Jpeg => "JPEG",
            EncodedImageFormat::Other => "IMG",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PastedImageInfo {
    pub width: u32,
    pub height: u32,
    pub encoded_format: EncodedImageFormat, // 目前始终为 PNG。
}

/// 从系统剪贴板获取图像，编码为 PNG，返回字节数据与信息。
#[cfg(not(target_os = "android"))]
pub fn paste_image_as_png() -> Result<(Vec<u8>, PastedImageInfo), PasteImageError> {
    let _span = tracing::debug_span!("paste_image_as_png").entered();
    tracing::debug!("attempting clipboard image read");
    let mut cb = arboard::Clipboard::new()
        .map_err(|e| PasteImageError::ClipboardUnavailable(e.to_string()))?;
    // 剪贴板中的图像有时以文件形式存在（例如从 Finder 复制粘贴），
    // 有时以图像数据形式存在（例如从 Chrome 粘贴）。
    // 两种情况都接受，如果两者都存在则优先使用文件。
    let files = cb
        .get()
        .file_list()
        .map_err(|e| PasteImageError::ClipboardUnavailable(e.to_string()));
    let dyn_img = if let Some(img) = files
        .unwrap_or_default()
        .into_iter()
        .find_map(|f| image::open(f).ok())
    {
        tracing::debug!(
            "clipboard image opened from file: {}x{}",
            img.width(),
            img.height()
        );
        img
    } else {
        let _span = tracing::debug_span!("get_image").entered();
        let img = cb
            .get_image()
            .map_err(|e| PasteImageError::NoImage(e.to_string()))?;
        let w = img.width as u32;
        let h = img.height as u32;
        tracing::debug!("clipboard image opened from image: {}x{}", w, h);

        let Some(rgba_img) = image::RgbaImage::from_raw(w, h, img.bytes.into_owned()) else {
            return Err(PasteImageError::EncodeFailed("invalid RGBA buffer".into()));
        };

        image::DynamicImage::ImageRgba8(rgba_img)
    };

    let mut png: Vec<u8> = Vec::new();
    {
        let span =
            tracing::debug_span!("encode_image", byte_length = tracing::field::Empty).entered();
        let mut cursor = std::io::Cursor::new(&mut png);
        dyn_img
            .write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| PasteImageError::EncodeFailed(e.to_string()))?;
        span.record("byte_length", png.len());
    }

    Ok((
        png,
        PastedImageInfo {
            width: dyn_img.width(),
            height: dyn_img.height(),
            encoded_format: EncodedImageFormat::Png,
        },
    ))
}

/// Android/Termux 不支持 arboard；返回明确的错误信息。
#[cfg(target_os = "android")]
pub fn paste_image_as_png() -> Result<(Vec<u8>, PastedImageInfo), PasteImageError> {
    Err(PasteImageError::ClipboardUnavailable(
        "clipboard image paste is unsupported on Android".into(),
    ))
}

/// 便捷方法：写入临时文件并返回文件路径与信息。
#[cfg(not(target_os = "android"))]
pub fn paste_image_to_temp_png() -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    // 首次尝试：通过 arboard 从系统剪贴板读取图像（本地路径或图像数据）。
    match paste_image_as_png() {
        Ok((png, info)) => {
            // 创建带有 .png 后缀的唯一临时文件，以避免冲突。
            let tmp = Builder::new()
                .prefix("reflect-clipboard-")
                .suffix(".png")
                .tempfile()
                .map_err(|e| PasteImageError::IoError(e.to_string()))?;
            std::fs::write(tmp.path(), &png)
                .map_err(|e| PasteImageError::IoError(e.to_string()))?;
            // 持久化文件（这样当句柄被丢弃后文件仍然存在）并返回其 PathBuf。
            let (_file, path) = tmp
                .keep()
                .map_err(|e| PasteImageError::IoError(e.error.to_string()))?;
            Ok((path, info))
        }
        Err(e) => {
            #[cfg(target_os = "linux")]
            {
                try_wsl_clipboard_fallback(&e).or(Err(e))
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err(e)
            }
        }
    }
}

/// 尝试 WSL 剪贴板图像粘贴的降级方案。
///
/// 如果剪贴板不可用（在 WSL 中很常见，因为 arboard 无法访问 Windows 剪贴板），
/// 则尝试 WSL 降级方案，在 Windows 侧调用 PowerShell 将剪贴板图像写入临时文件，
/// 然后返回对应的 WSL 路径。
#[cfg(target_os = "linux")]
fn try_wsl_clipboard_fallback(
    error: &PasteImageError,
) -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    use PasteImageError::ClipboardUnavailable;
    use PasteImageError::NoImage;

    if !is_probably_wsl() || !matches!(error, ClipboardUnavailable(_) | NoImage(_)) {
        return Err(error.clone());
    }

    tracing::debug!("attempting Windows PowerShell clipboard fallback");
    let Some(win_path) = try_dump_windows_clipboard_image() else {
        return Err(error.clone());
    };

    tracing::debug!("powershell produced path: {}", win_path);
    let Some(mapped_path) = convert_windows_path_to_wsl(&win_path) else {
        return Err(error.clone());
    };

    let Ok((w, h)) = image::image_dimensions(&mapped_path) else {
        return Err(error.clone());
    };

    // 直接返回映射后的路径，无需复制。
    // 文件将在序列化期间被读取并进行 base64 编码。
    Ok((
        mapped_path,
        PastedImageInfo {
            width: w,
            height: h,
            encoded_format: EncodedImageFormat::Png,
        },
    ))
}

/// 尝试调用 Windows PowerShell 命令（多个常用名称）将剪贴板图像
/// 保存为临时 PNG 并返回该文件的 Windows 路径。
/// 如果所有命令均失败或未检测到图像则返回 None。
#[cfg(target_os = "linux")]
fn try_dump_windows_clipboard_image() -> Option<String> {
    // PowerShell 脚本：从剪贴板保存图像到临时 PNG 并输出路径。
    // 强制使用 UTF-8 输出，以避免 powershell.exe（默认 UTF-16LE）
    // 与 pwsh（默认 UTF-8）之间的编码问题。
    let script = r#"[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $img = Get-Clipboard -Format Image; if ($img -ne $null) { $p=[System.IO.Path]::GetTempFileName(); $p = [System.IO.Path]::ChangeExtension($p,'png'); $img.Save($p,[System.Drawing.Imaging.ImageFormat]::Png); Write-Output $p } else { exit 1 }"#;

    for cmd in ["powershell.exe", "pwsh", "powershell"] {
        match std::process::Command::new(cmd)
            .args(["-NoProfile", "-Command", script])
            .output()
        {
            // 执行 PowerShell 命令
            Ok(output) => {
                if output.status.success() {
                    // 解码为 UTF-8（由上方脚本强制指定）。
                    let win_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !win_path.is_empty() {
                        tracing::debug!("{} saved clipboard image to {}", cmd, win_path);
                        return Some(win_path);
                    }
                } else {
                    tracing::debug!("{} returned non-zero status", cmd);
                }
            }
            Err(err) => {
                tracing::debug!("{} not executable: {}", cmd, err);
            }
        }
    }
    None
}

#[cfg(target_os = "android")]
pub fn paste_image_to_temp_png() -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    // 保持与 paste_image_as_png 的错误信息一致。
    Err(PasteImageError::ClipboardUnavailable(
        "clipboard image paste is unsupported on Android".into(),
    ))
}

/// 规范化粘贴文本，用于单行搜索查询。
pub(crate) fn normalize_pasted_search_query(pasted: &str) -> Option<String> {
    let normalized = pasted.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

/// 规范化可能表示文件路径的粘贴文本。
///
/// 支持：
/// - `file://` URL（转换为本地路径）
/// - Windows/UNC 路径
/// - shell 转义的单路径（通过 `shlex`）
pub fn normalize_pasted_path(pasted: &str) -> Option<PathBuf> {
    let pasted = pasted.trim();
    let unquoted = pasted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| pasted.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(pasted);

    // file:// URL → 文件系统路径
    if let Ok(url) = url::Url::parse(unquoted)
        && url.scheme() == "file"
    {
        return url.to_file_path().ok();
    }

// TODO：后续会视情况逐步完善实现与单元测试。
    // 可能使用 typed-path（参见上游 PR 中的先例）。
    //
    // 检测未加引号的 Windows 路径并绕过 POSIX shlex（后者会
    // 将反斜杠视为转义字符，例如 C:\Users\Alice\file.png）。
    // 同时处理 UNC 路径（\\server\share\path）。
    if let Some(path) = normalize_windows_path(unquoted) {
        return Some(path);
    }

    // shell-转义的单路径 → 去除转义
    let parts: Vec<String> = shlex::Shlex::new(pasted).collect();
    if parts.len() == 1 {
        let part = parts.into_iter().next()?;
        if let Some(path) = normalize_windows_path(&part) {
            return Some(path);
        }
        return Some(PathBuf::from(part));
    }

    None
}

#[cfg(target_os = "linux")]
pub(crate) fn is_probably_wsl() -> bool {
    // 主要方式：检查 /proc/version 是否包含 "microsoft" 或 "WSL"（对标准 WSL 最可靠）。
    if let Ok(version) = std::fs::read_to_string("/proc/version") {
        let version_lower = version.to_lowercase();
        if version_lower.contains("microsoft") || version_lower.contains("wsl") {
            return true;
        }
    }

// 降级方案：检查 WSL 环境变量。这能处理一些边缘情况，比如
    // 在 WSL 中安装了自定义 Linux 内核，此时 /proc/version 可能不包含
    // "microsoft" 或 "WSL"。
    std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some()
}

#[cfg(target_os = "linux")]
fn convert_windows_path_to_wsl(input: &str) -> Option<PathBuf> {
    if input.starts_with("\\\\") {
        return None;
    }

    let drive_letter = input.chars().next()?.to_ascii_lowercase();
    if !drive_letter.is_ascii_lowercase() {
        return None;
    }

    if input.get(1..2) != Some(":") {
        return None;
    }

    let mut result = PathBuf::from(format!("/mnt/{drive_letter}"));
    for component in input
        .get(2..)?
        .trim_start_matches(['\\', '/'])
        .split(['\\', '/'])
        .filter(|component| !component.is_empty())
    {
        result.push(component);
    }

    Some(result)
}

fn normalize_windows_path(input: &str) -> Option<PathBuf> {
    // 驱动器字母路径：C:\ 或 C:/
    let drive = input
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic())
        .unwrap_or(false)
        && input.get(1..2) == Some(":")
        && input
            .get(2..3)
            .map(|s| s == "\\" || s == "/")
            .unwrap_or(false);
    // UNC 路径：\\server\share
    let unc = input.starts_with("\\\\");
    if !drive && !unc {
        return None;
    }

    #[cfg(target_os = "linux")]
    {
        if is_probably_wsl()
            && let Some(converted) = convert_windows_path_to_wsl(input)
        {
            return Some(converted);
        }
    }

    Some(PathBuf::from(input))
}

/// 根据给定路径的文件扩展名推断图像格式。
pub fn pasted_image_format(path: &Path) -> EncodedImageFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => EncodedImageFormat::Png,
        Some("jpg") | Some("jpeg") => EncodedImageFormat::Jpeg,
        _ => EncodedImageFormat::Other,
    }
}

#[cfg(test)]
mod pasted_search_query_tests {
    use super::*;

    #[test]
    fn collapses_whitespace() {
        assert_eq!(
            normalize_pasted_search_query("  alpha\n\tbeta\r\n gamma  "),
            Some(String::from("alpha beta gamma"))
        );
    }
}

#[cfg(test)]
mod pasted_paths_tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn normalize_file_url() {
        let input = "file:///tmp/example.png";
        let result = normalize_pasted_path(input).expect("should parse file URL");
        assert_eq!(result, PathBuf::from("/tmp/example.png"));
    }

    #[test]
    fn normalize_file_url_windows() {
        let input = r"C:\Temp\example.png";
        let result = normalize_pasted_path(input).expect("should parse file URL");
        #[cfg(target_os = "linux")]
        let expected = if is_probably_wsl()
            && let Some(converted) = convert_windows_path_to_wsl(input)
        {
            converted
        } else {
            PathBuf::from(r"C:\Temp\example.png")
        };
        #[cfg(not(target_os = "linux"))]
        let expected = PathBuf::from(r"C:\Temp\example.png");
        assert_eq!(result, expected);
    }

    #[test]
    fn normalize_shell_escaped_single_path() {
        let input = "/home/user/My\\ File.png";
        let result = normalize_pasted_path(input).expect("should unescape shell-escaped path");
        assert_eq!(result, PathBuf::from("/home/user/My File.png"));
    }

    #[test]
    fn normalize_simple_quoted_path_fallback() {
        let input = "\"/home/user/My File.png\"";
        let result = normalize_pasted_path(input).expect("should trim simple quotes");
        assert_eq!(result, PathBuf::from("/home/user/My File.png"));
    }

    #[test]
    fn normalize_single_quoted_unix_path() {
        let input = "'/home/user/My File.png'";
        let result = normalize_pasted_path(input).expect("should trim single quotes via shlex");
        assert_eq!(result, PathBuf::from("/home/user/My File.png"));
    }

    #[test]
    fn normalize_multiple_tokens_returns_none() {
        // Shell 分割后产生两个标记 → 不是单一路径
        let input = "/home/user/a\\ b.png /home/user/c.png";
        let result = normalize_pasted_path(input);
        assert!(result.is_none());
    }

    #[test]
    fn pasted_image_format_png_jpeg_unknown() {
        assert_eq!(
            pasted_image_format(Path::new("/a/b/c.PNG")),
            EncodedImageFormat::Png
        );
        assert_eq!(
            pasted_image_format(Path::new("/a/b/c.jpg")),
            EncodedImageFormat::Jpeg
        );
        assert_eq!(
            pasted_image_format(Path::new("/a/b/c.JPEG")),
            EncodedImageFormat::Jpeg
        );
        assert_eq!(
            pasted_image_format(Path::new("/a/b/c")),
            EncodedImageFormat::Other
        );
        assert_eq!(
            pasted_image_format(Path::new("/a/b/c.webp")),
            EncodedImageFormat::Other
        );
    }

    #[test]
    fn normalize_single_quoted_windows_path() {
        let input = r"'C:\\Users\\Alice\\My File.jpeg'";
        let unquoted = r"C:\\Users\\Alice\\My File.jpeg";
        let result =
            normalize_pasted_path(input).expect("should trim single quotes on windows path");
        #[cfg(target_os = "linux")]
        let expected = if is_probably_wsl()
            && let Some(converted) = convert_windows_path_to_wsl(unquoted)
        {
            converted
        } else {
            PathBuf::from(unquoted)
        };
        #[cfg(not(target_os = "linux"))]
        let expected = PathBuf::from(unquoted);
        assert_eq!(result, expected);
    }

    #[test]
    fn normalize_double_quoted_windows_path() {
        let input = r#""C:\\Users\\Alice\\My File.jpeg""#;
        let unquoted = r"C:\\Users\\Alice\\My File.jpeg";
        let result =
            normalize_pasted_path(input).expect("should trim double quotes on windows path");
        #[cfg(target_os = "linux")]
        let expected = if is_probably_wsl()
            && let Some(converted) = convert_windows_path_to_wsl(unquoted)
        {
            converted
        } else {
            PathBuf::from(unquoted)
        };
        #[cfg(not(target_os = "linux"))]
        let expected = PathBuf::from(unquoted);
        assert_eq!(result, expected);
    }

    #[test]
    fn normalize_unquoted_windows_path_with_spaces() {
        let input = r"C:\\Users\\Alice\\My Pictures\\example image.png";
        let result = normalize_pasted_path(input).expect("should accept unquoted windows path");
        #[cfg(target_os = "linux")]
        let expected = if is_probably_wsl()
            && let Some(converted) = convert_windows_path_to_wsl(input)
        {
            converted
        } else {
            PathBuf::from(r"C:\\Users\\Alice\\My Pictures\\example image.png")
        };
        #[cfg(not(target_os = "linux"))]
        let expected = PathBuf::from(r"C:\\Users\\Alice\\My Pictures\\example image.png");
        assert_eq!(result, expected);
    }

    #[test]
    fn normalize_unc_windows_path() {
        let input = r"\\\\server\\share\\folder\\file.jpg";
        let result = normalize_pasted_path(input).expect("should accept UNC windows path");
        assert_eq!(
            result,
            PathBuf::from(r"\\\\server\\share\\folder\\file.jpg")
        );
    }

    #[test]
    fn pasted_image_format_with_windows_style_paths() {
        assert_eq!(
            pasted_image_format(Path::new(r"C:\\a\\b\\c.PNG")),
            EncodedImageFormat::Png
        );
        assert_eq!(
            pasted_image_format(Path::new(r"C:\\a\\b\\c.jpeg")),
            EncodedImageFormat::Jpeg
        );
        assert_eq!(
            pasted_image_format(Path::new(r"C:\\a\\b\\noext")),
            EncodedImageFormat::Other
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn normalize_windows_path_in_wsl() {
        // 此测试仅在真正的 WSL 系统上运行
        if !is_probably_wsl() {
            // 如果不是在 WSL 上则跳过测试
            return;
        }
        let input = r"C:\\Users\\Alice\\Pictures\\example image.png";
        let result = normalize_pasted_path(input).expect("should convert windows path on wsl");
        assert_eq!(
            result,
            PathBuf::from("/mnt/c/Users/Alice/Pictures/example image.png")
        );
    }
}
