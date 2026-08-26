//! `reflect-tui` headless 子命令 —— fork / rename / export / traces。
//!
//! reflect-tui 顶层二进制(`src/bin/main.rs`)默认启动 TUI;带子命令时走
//! headless 路径(执行完即退出,不进 TUI)。这层复用子模块 reflect-cli 的
//! 实现思路,但所有底层调用都走 reflect-rollout / reflect-telemetry 的公开
//! API —— 与 reflect-cli 行为对称,fork 后提示用 `--resume <child>` 续作。
//!
//! 参照:`reflect-agent/crates/runtime/reflect-cli/src/{session,traces}.rs`。

pub mod session;
pub mod traces;
