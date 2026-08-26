//! TUI 的 `/copy` 命令和 `Ctrl+O` 快捷键的剪贴板复制后端。
//!
//! 此模块根据当前环境决定*如何*将文本放入用户剪贴板。选择顺序为：
//!
//! 1. **SSH 会话**（设置了 `SSH_TTY` / `SSH_CONNECTION`）：优先使用 tmux 剪贴板
//!    集成，否则使用 OSC 52，因为本机剪贴板属于远程机器。
//! 2. **本地会话**：先尝试 `arboard`（原生剪贴板）。在 WSL 上，如果 `arboard` 失败，
//!    通过 PowerShell 回退到 Windows 剪贴板。最后，如果原生/WSL 路径都失败，
//!    回退到终端介导的复制。
//!
//! 在 Linux 上，X11 和部分 Wayland 合成器要求写入剪贴板的进程保持句柄打开。
//! `ClipboardLease` 包装 `arboard::Clipboard`，以便调用者可以在 TUI 的生存期内存储它。
//! 在其他平台上，租约始终为 `None`。
//!
//! 该模块故意保持狭窄：仅文本复制、面向用户的错误字符串、
//! 无可复用的剪贴板抽象。图像粘贴在 `clipboard_paste` 中。

use base64::Engine;
use std::io::Write;

/// 我们将 base64 编码到 OSC 52 序列中的最大原始字节数。
/// 在编码前拒绝大负载，以免压垮终端。
const OSC52_MAX_RAW_BYTES: usize = 100_000;
#[cfg(target_os = "macos")]
static STDERR_SUPPRESSION_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> =
    std::sync::OnceLock::new();

/// 将文本复制到系统剪贴板。
///
/// 通过 SSH 时，使用终端中转复制，使文本到达*本地*
/// 终端模拟器的剪贴板，而不是用户无法访问的远程 X11/Wayland 剪贴板。
/// 在本地会话中，先尝试 `arboard`（原生剪贴板），然后回退到 WSL PowerShell，
/// 再回退到终端中转复制（如需要）。
///
/// OSC 52 受 kitty、WezTerm、iTerm2、Ghostty 等终端支持。
pub(crate) fn copy_to_clipboard(text: &str) -> Result<Option<ClipboardLease>, String> {
    copy_to_clipboard_with(
        text,
        CopyEnvironment {
            ssh_session: is_ssh_session(),
            wsl_session: is_wsl_session(),
            tmux_session: is_tmux_session(),
        },
        tmux_clipboard_copy,
        osc52_copy,
        arboard_copy,
        wsl_clipboard_copy,
    )
}

/// 当后端需要时，保持平台剪贴板所有者存活。
///
/// 在 Linux/X11 和部分 Wayland 合成器上，剪贴板内容由拥有进程提供。
/// 在用户粘贴之前丢弃 `arboard::Clipboard` 会导致内容消失。
/// 在触发复制的部件上存储此租约，使句柄与 TUI 同寿命。
/// 在非 Linux 原生路径和 OSC 52 路径上，租约为 `None`——这些后端不需要进程生命周期所有权。
pub(crate) struct ClipboardLease {
    #[cfg(target_os = "linux")]
    _clipboard: Option<arboard::Clipboard>,
}

impl ClipboardLease {
    #[cfg(target_os = "linux")]
    fn native_linux(clipboard: arboard::Clipboard) -> Self {
        Self {
            _clipboard: Some(clipboard),
        }
    }

    #[cfg(test)]
    pub(crate) fn test() -> Self {
        Self {
            #[cfg(target_os = "linux")]
            _clipboard: None,
        }
    }
}

/// 核心复制逻辑，注入后端，支持确定性单元测试，
/// 无需接触真实剪贴板或终端 I/O。
#[derive(Clone, Copy)]
struct CopyEnvironment {
    ssh_session: bool,
    wsl_session: bool,
    tmux_session: bool,
}

fn copy_to_clipboard_with(
    text: &str,
    environment: CopyEnvironment,
    tmux_copy_fn: impl Fn(&str) -> Result<(), String>,
    osc52_copy_fn: impl Fn(&str) -> Result<(), String>,
    arboard_copy_fn: impl Fn(&str) -> Result<Option<ClipboardLease>, String>,
    wsl_copy_fn: impl Fn(&str) -> Result<(), String>,
) -> Result<Option<ClipboardLease>, String> {
    if environment.ssh_session {
        // 通过 SSH 时，原生剪贴板写入远程机器，那是没有用的。
        // 终端中介的复制会到达本地终端模拟器。
        return terminal_clipboard_copy_with(
            text,
            environment.tmux_session,
            &tmux_copy_fn,
            &osc52_copy_fn,
        )
        .map(|()| None)
        .map_err(|terminal_err| {
            tracing::warn!("terminal clipboard copy failed over SSH: {terminal_err}");
            if environment.tmux_session {
                format!("terminal clipboard copy failed over SSH: {terminal_err}")
            } else {
                format!("OSC 52 clipboard copy failed over SSH: {terminal_err}")
            }
        });
    }

    match arboard_copy_fn(text) {
        Ok(lease) => Ok(lease),
        Err(native_err) => {
            if environment.wsl_session {
                tracing::warn!(
                    "native clipboard copy failed: {native_err}, falling back to WSL PowerShell"
                );
                match wsl_copy_fn(text) {
                    Ok(()) => return Ok(None),
                    Err(wsl_err) => {
                        tracing::warn!(
                            "WSL PowerShell clipboard copy failed: {wsl_err}, falling back to terminal clipboard"
                        );
                        return terminal_clipboard_copy_with(
                            text,
                            environment.tmux_session,
                            &tmux_copy_fn,
                            &osc52_copy_fn,
                        )
                        .map(|()| None)
                        .map_err(|terminal_err| {
                            if environment.tmux_session {
                                format!(
                                    "native clipboard: {native_err}; WSL fallback: {wsl_err}; terminal fallback: {terminal_err}"
                                )
                            } else {
                                format!(
                                    "native clipboard: {native_err}; WSL fallback: {wsl_err}; OSC 52 fallback: {terminal_err}"
                                )
                            }
                        });
                    }
                }
            }
            tracing::warn!(
                "native clipboard copy failed: {native_err}, falling back to terminal clipboard"
            );
            terminal_clipboard_copy_with(
                text,
                environment.tmux_session,
                &tmux_copy_fn,
                &osc52_copy_fn,
            )
            .map(|()| None)
            .map_err(|terminal_err| {
                if environment.tmux_session {
                    format!("native clipboard: {native_err}; terminal fallback: {terminal_err}")
                } else {
                    format!("native clipboard: {native_err}; OSC 52 fallback: {terminal_err}")
                }
            })
        }
    }
}

/// 通过活动终端复制，优先使用 tmux 的原生剪贴板路径。
fn terminal_clipboard_copy_with(
    text: &str,
    tmux_session: bool,
    tmux_copy_fn: &impl Fn(&str) -> Result<(), String>,
    osc52_copy_fn: &impl Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    if tmux_session {
        match tmux_copy_fn(text) {
            Ok(()) => return Ok(()),
            Err(tmux_err) => {
                tracing::warn!("tmux clipboard copy failed: {tmux_err}, falling back to OSC 52");
                return osc52_copy_fn(text).map_err(|osc_err| {
                    format!("tmux clipboard: {tmux_err}; OSC 52 fallback: {osc_err}")
                });
            }
        }
    }

    osc52_copy_fn(text)
}

/// 检测当前进程是否运行在 SSH 会话中。
fn is_ssh_session() -> bool {
    std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CONNECTION").is_some()
}

/// 检测当前进程是否运行在 tmux 中。
fn is_tmux_session() -> bool {
    std::env::var_os("TMUX").is_some() || std::env::var_os("TMUX_PANE").is_some()
}

#[cfg(target_os = "linux")]
fn is_wsl_session() -> bool {
    crate::tui_core::clipboard_paste::is_probably_wsl()
}

#[cfg(not(target_os = "linux"))]
fn is_wsl_session() -> bool {
    false
}

/// 在 stderr 被抑制的情况下运行 arboard。
///
/// 在 macOS 上，`arboard::Clipboard::new()` 初始化 `NSPasteboard` 会
/// 在 stderr 上触发 `os_log` / `NSLog` 输出。因为 TUI 拥有终端，
/// 这些杂散输出会破坏显示。我们临时将 fd 2 重定向到 `/dev/null` 以保持屏幕干净。
#[cfg(all(not(target_os = "android"), not(target_os = "linux")))]
fn arboard_copy(text: &str) -> Result<Option<ClipboardLease>, String> {
    #[cfg(target_os = "macos")]
    let _stderr_lock = STDERR_SUPPRESSION_MUTEX
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .map_err(|_| "stderr suppression lock poisoned".to_string())?;
    let _guard = SuppressStderr::new();
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("failed to set clipboard text: {e}"))?;
    Ok(None)
}

/// 抑制 stderr 运行 arboard。
///
/// 在 Linux/X11 和部分 Wayland 设置上，剪贴板内容由最后写入的进程提供。
/// 保持 `Clipboard` 存活，使复制的文本在 TUI 运行期间仍然可以粘贴。
#[cfg(target_os = "linux")]
fn arboard_copy(text: &str) -> Result<Option<ClipboardLease>, String> {
    let _guard = SuppressStderr::new();
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
    clipboard
        .set_text(text)
        .map_err(|e| format!("failed to set clipboard text: {e}"))?;
    Ok(Some(ClipboardLease::native_linux(clipboard)))
}

#[cfg(target_os = "android")]
fn arboard_copy(_text: &str) -> Result<Option<ClipboardLease>, String> {
    Err("native clipboard unavailable on Android".to_string())
}

/// 从 WSL 进程将文本复制到 Windows 剪贴板。
#[cfg(target_os = "linux")]
fn wsl_clipboard_copy(text: &str) -> Result<(), String> {
    let mut child = std::process::Command::new("powershell.exe")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .args([
            "-NoProfile",
            "-Command",
            "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; $ErrorActionPreference = 'Stop'; $text = [Console]::In.ReadToEnd(); Set-Clipboard -Value $text",
        ])
        .spawn()
        .map_err(|e| format!("failed to spawn powershell.exe: {e}"))?;

    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("failed to open powershell.exe stdin".to_string());
    };

    if let Err(err) = stdin.write_all(text.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("failed to write to powershell.exe: {err}"));
    }

    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for powershell.exe: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            let status = output.status;
            Err(format!("powershell.exe exited with status {status}"))
        } else {
            Err(format!("powershell.exe failed: {stderr}"))
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn wsl_clipboard_copy(_text: &str) -> Result<(), String> {
    Err("WSL clipboard fallback unavailable on this platform".to_string())
}

/// 通过 tmux 的原生剪贴板集成复制文本。
///
/// `load-buffer -w -` 让 tmux 从 stdin 读取文本，保留匹配的 tmux
/// 粘贴缓冲区，并在不依赖 DCS 透传的情况下将内容转发到外部终端剪贴板。
fn tmux_clipboard_copy(text: &str) -> Result<(), String> {
    tmux_clipboard_copy_ready(
        || tmux_command_output(["show-options", "-gv", "set-clipboard"]),
        || tmux_command_output(["info"]),
    )?;

    let mut child = std::process::Command::new("tmux")
        .args(["load-buffer", "-w", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn tmux: {e}"))?;

    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("failed to open tmux stdin".to_string());
    };

    if let Err(err) = stdin.write_all(text.as_bytes()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("failed to write to tmux: {err}"));
    }

    drop(stdin);

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for tmux: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            let status = output.status;
            Err(format!("tmux exited with status {status}"))
        } else {
            Err(format!("tmux failed: {stderr}"))
        }
    }
}

/// 验证 tmux 是否配置为将剪贴板写入转发到外部终端。
fn tmux_clipboard_copy_ready(
    set_clipboard_fn: impl FnOnce() -> Result<String, String>,
    tmux_info_fn: impl FnOnce() -> Result<String, String>,
) -> Result<(), String> {
    let set_clipboard = set_clipboard_fn()?;
    if set_clipboard.trim() == "off" {
        return Err("tmux clipboard forwarding is disabled".to_string());
    }

    let tmux_info = tmux_info_fn()?;
    if tmux_info.lines().any(|line| line.contains("Ms: [missing]")) {
        return Err("tmux clipboard forwarding is unavailable: missing Ms capability".to_string());
    }

    Ok(())
}

fn tmux_command_output<const N: usize>(args: [&str; N]) -> Result<String, String> {
    let output = std::process::Command::new("tmux")
        .args(args)
        .output()
        .map_err(|e| format!("failed to spawn tmux: {e}"))?;

    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| format!("tmux output was not UTF-8: {e}"))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            let status = output.status;
            Err(format!("tmux exited with status {status}"))
        } else {
            Err(format!("tmux failed: {stderr}"))
        }
    }
}

/// RAII 守卫，在创建时将 stderr（fd 2）重定向到 `/dev/null`，
/// 在丢弃时恢复原始 fd。
#[cfg(target_os = "macos")]
struct SuppressStderr {
    saved_fd: Option<libc::c_int>,
}

#[cfg(target_os = "macos")]
impl SuppressStderr {
    fn new() -> Self {
        unsafe {
            // 保存当前的 stderr 文件描述符。
            let saved = libc::dup(2);
            if saved < 0 {
                return Self { saved_fd: None };
            }
            // 打开 /dev/null 并将 fd 2 指向它。
            let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
            if devnull < 0 {
                libc::close(saved);
                return Self { saved_fd: None };
            }
            if libc::dup2(devnull, 2) < 0 {
                libc::close(saved);
                libc::close(devnull);
                return Self { saved_fd: None };
            }
            libc::close(devnull);
            Self {
                saved_fd: Some(saved),
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for SuppressStderr {
    fn drop(&mut self) {
        if let Some(saved) = self.saved_fd {
            unsafe {
                libc::dup2(saved, 2);
                libc::close(saved);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
struct SuppressStderr;

#[cfg(not(target_os = "macos"))]
impl SuppressStderr {
    fn new() -> Self {
        Self
    }
}

/// 通过 OSC 52 终端转义序列将文本写入剪贴板。
fn osc52_copy(text: &str) -> Result<(), String> {
    let sequence = osc52_sequence(text, std::env::var_os("TMUX").is_some())?;
    #[cfg(unix)]
    {
        match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            Ok(tty) => match write_osc52_to_writer(tty, &sequence) {
                Ok(()) => return Ok(()),
                Err(err) => tracing::debug!(
                    "failed to write OSC 52 to /dev/tty: {err}; falling back to stdout"
                ),
            },
            Err(err) => {
                tracing::debug!("failed to open /dev/tty for OSC 52: {err}; falling back to stdout")
            }
        }
    }

    write_osc52_to_writer(std::io::stdout().lock(), &sequence)
}

fn write_osc52_to_writer(mut writer: impl Write, sequence: &str) -> Result<(), String> {
    writer
        .write_all(sequence.as_bytes())
        .map_err(|e| format!("failed to write OSC 52: {e}"))?;
    writer
        .flush()
        .map_err(|e| format!("failed to flush OSC 52: {e}"))
}

fn osc52_sequence(text: &str, tmux: bool) -> Result<String, String> {
    let raw_bytes = text.len();
    if raw_bytes > OSC52_MAX_RAW_BYTES {
        return Err(format!(
            "OSC 52 payload too large ({raw_bytes} bytes; max {OSC52_MAX_RAW_BYTES})"
        ));
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    if tmux {
        Ok(format!("\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\"))
    } else {
        Ok(format!("\x1b]52;c;{encoded}\x07"))
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
