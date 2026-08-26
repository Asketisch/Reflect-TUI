//! `reflect_shell_command` 的最小化移植版本。
//!
//! 为常见的 `bash -lc "script"` / `zsh -lc "script"` / `sh -c "script"`
//! 形态实现 `extract_shell_command`(TUI 在 exec-command 显示时
//! 涉及的形态)。上游完整解析器(tree-sitter bash + PowerShell)未移植;
//! 未知形态返回 `None`,调用方回退到 `escape_command`。

pub mod parse_command {
    /// 若 `command` 是已知 POSIX shell 的 `[shell, "-lc" | "-c", script]`
    /// 调用形式,返回脚本字符串;否则返回 `None`。
    pub fn extract_shell_command(command: &[String]) -> Option<String> {
        extract_bash_command(command).or_else(|| extract_powershell_command(command))
    }

    fn extract_bash_command(command: &[String]) -> Option<String> {
        let [shell, flag, script] = command else {
            return None;
        };
        if !matches!(flag.as_str(), "-lc" | "-c") {
            return None;
        }
        let is_posix_shell = {
            let name = std::path::Path::new(shell)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(shell.as_str());
            matches!(name, "bash" | "zsh" | "sh" | "dash" | "ksh" | "fish")
        };
        if !is_posix_shell {
            return None;
        }
        Some(script.clone())
    }

    fn extract_powershell_command(command: &[String]) -> Option<String> {
        // `pwsh -Command "script"` / `powershell -Command "script"` 形式。
        let [shell, flag, script] = command else {
            return None;
        };
        let name = std::path::Path::new(shell)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(shell.as_str());
        if !matches!(name, "pwsh" | "powershell") {
            return None;
        }
        if !matches!(
            flag.as_str(),
            "-Command" | "-command" | "/Command" | "/command"
        ) {
            return None;
        }
        Some(script.clone())
    }
}

pub mod bash {
    /// 为仍按移植前签名导入的 vendored 调用者保留的向后兼容垫片。
    /// 对 `[shell, "-lc", script]` 形式的调用返回提取出的脚本字符串;否则返回 `None`。
    pub fn extract_bash_command(command: &[String]) -> Option<String> {
        super::parse_command::extract_shell_command(command)
    }
}
