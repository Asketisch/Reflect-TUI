//! `zhuye-cli` headless 子命令 —— fork / rename / export / traces。
//!
//! zhuye-cli 顶层二进制(`src/bin/main.rs`)默认启动 TUI;带子命令时走
//! headless 路径(执行完即退出,不进 TUI)。这层复用子模块 zhuye-cli 的
//! 实现思路,但所有底层调用都走 zhuye-rollout / zhuye-telemetry 的公开
//! API —— 与 reflect-cli 行为对称,fork 后提示用 `--resume <child>` 续作。
//!
//! 参照:`reflect-agent/crates/runtime/reflect-cli/src/{session,traces}.rs`。

pub mod discussion;
pub mod security;
pub mod session;
pub mod traces;
pub mod workspace;
