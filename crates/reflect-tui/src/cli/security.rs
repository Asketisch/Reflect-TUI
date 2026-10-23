//! `reflect-tui security audit ...` —— cargo audit 安全扫描。
//!
//! v1.x 接线:submodule 的 `reflect_integration::security_scan` 提供
//! `run_cargo_audit`(跑 `cargo audit --json`,带超时与管道防死锁)与
//! `print_report`(人读报告)。此前只有 `reflect-cli` 的 `reflect security`
//! 能触达,TUI 侧够不着。
//!
//! cargo-audit 不可用时降级为 `--version` 探测,报告里如实标注
//! `ran = false` —— 不静默假装扫过了。

use std::path::PathBuf;

/// `reflect-tui security audit [--workspace <path>]`。
pub fn audit(workspace: Option<PathBuf>) -> anyhow::Result<()> {
    println!("Reflect security audit");
    println!("======================");
    let report = reflect_integration::security_scan::run_cargo_audit(workspace.as_deref());
    reflect_integration::security_scan::print_report(&report);
    Ok(())
}
