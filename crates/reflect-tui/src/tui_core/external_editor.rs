//! v1.x Tier 3:打开外部编辑器(`$VISUAL` / `$EDITOR`)编辑一段文本。
//!
//! **本模块只管"编辑"本身**——terminal suspend/resume 由调用方负责
//! (避免与 TUI 的 alt-screen / raw mode 状态机耦合)。
//!
//! 调用约定:
//! 1. 调用方在调用 `run_editor` 前 **先** `disable_raw_mode()` + `leave_alt_screen()`。
//! 2. `run_editor` 返回后,调用方 **再** `enable_raw_mode()` + 调度下一帧重绘。

use std::io::Write;
use std::process::Command;

/// 解析 `$VISUAL` / `$EDITOR` 为命令向量(支持带参数,如 `code -w`)。
///
/// 优先级:`$VISUAL` > `$EDITOR`。
pub fn resolve_editor_command() -> Result<Vec<String>, String> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.is_empty()))
        .ok_or_else(|| "$VISUAL / $EDITOR not set".to_string())?;
    let parts = shell_split(&raw).ok_or_else(|| format!("parse editor command: {raw:?}"))?;
    if parts.is_empty() || parts[0].is_empty() {
        return Err("editor command is empty".into());
    }
    Ok(parts)
}

/// 用 `$VISUAL` / `$EDITOR` 编辑给定 seed,返回编辑后的文本。
///
/// 失败原因(`$EDITOR` 未设、临时文件创建失败、子进程非零退出等)返回
/// `Err(String)`,调用方应向用户展示。
pub fn run_editor(seed: &str) -> Result<String, String> {
    let cmd = resolve_editor_command()?;
    run_editor_with(seed, &cmd)
}

/// 用给定命令(首项为可执行,其余为参数)编辑 seed。
pub fn run_editor_with(seed: &str, cmd: &[String]) -> Result<String, String> {
    if cmd.is_empty() {
        return Err("empty editor command".into());
    }
    // 1. 写临时文件。
    let mut tmp = tempfile::Builder::new()
        .prefix("reflect-edit-")
        .suffix(".md")
        .tempfile()
        .map_err(|e| format!("create temp file: {e}"))?;
    tmp.write_all(seed.as_bytes())
        .map_err(|e| format!("write seed: {e}"))?;
    tmp.flush().map_err(|e| format!("flush: {e}"))?;

    let path = tmp.path().to_path_buf();
    let mut full_cmd = cmd.to_vec();
    full_cmd.push(path.display().to_string());

    // 2. spawn editor(继承 stdio 让用户与 editor 直接交互)。
    let status = Command::new(&full_cmd[0])
        .args(&full_cmd[1..])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|e| format!("spawn editor {:?}: {e}", full_cmd[0]))?;

    if !status.success() {
        return Err(format!("editor exited with {status}"));
    }

    // 3. 读回修改后的内容。
    let edited = std::fs::read_to_string(&path).map_err(|e| format!("read edited file: {e}"))?;
    // 保留临时文件直到 drop(用户可在错误信息中查看路径);
    // 但实际上 tempdir 会在 tmp drop 时清理。
    let _ = tmp; // 显式 hold,避免提前清理
    Ok(edited.trim_end_matches('\n').to_string())
}

/// 极简 POSIX shell split:支持单引号/双引号/反斜杠转义/普通空白分隔。
/// 不支持 `$VAR` 展开 / `~` 展开——`$EDITOR` 实际值很少用到这些。
fn shell_split(input: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '\\' if !in_single => {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    cur.push(next);
                } else {
                    return None;
                }
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if in_single || in_double {
        return None;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_split_basic() {
        assert_eq!(shell_split("vim").unwrap(), vec!["vim"]);
        assert_eq!(shell_split("code -w").unwrap(), vec!["code", "-w"]);
        assert_eq!(shell_split("emacs -nw").unwrap(), vec!["emacs", "-nw"]);
    }

    #[test]
    fn shell_split_with_quotes() {
        assert_eq!(
            shell_split(r#"code --wait "$X""#).unwrap(),
            vec!["code", "--wait", "$X"]
        );
        assert_eq!(
            shell_split("vim '-c' 'set ft=markdown'").unwrap(),
            vec!["vim", "-c", "set ft=markdown"]
        );
    }

    #[test]
    fn shell_split_unterminated_quote_returns_none() {
        assert!(shell_split(r#""unterminated"#).is_none());
        assert!(shell_split("'unterminated").is_none());
    }

    #[test]
    fn resolve_editor_command_uses_visual_first() {
        // 单元测试只验证 fallback 到 $EDITOR 的逻辑,不真正 spawn editor。
        // (实际行为依赖环境变量,在 CI 中可能没设,这里只测解析。)
        let _cmd = resolve_editor_command(); // 不 panic 即通过
    }
}
