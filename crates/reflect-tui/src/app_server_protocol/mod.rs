//! `app-server-protocol` 的本地协议存根（Reflect 本地重声明）。
//!
//! 镜像上游 `app_server_protocol` crate 的公开表面：Reflect TUI 表现层
//! 引用了约 146 个来自该协议的类型。为避免引入整个上游 crate 及其
//! `protocol_compat` / `schemars` / `ts_rs` 依赖图，本模块按领域重声明每个被
//! 引用的类型，形状最小化但足以编译 UI 代码。结构体字段尽量对齐上游布局，但
//! 可能简化（字段一律 `pub`，真实类型无法实现 `Default` 的字段统一包成
//! `Option`）。
//!
//! 所有类型派生 `Debug, Clone`，并尽量派生 `Default / PartialEq / Eq`。
//! 方法以 `// stub` 标注，体为空或返回 `Default::default()`。
//!
//! 来源：形状参照上游 `app-server-protocol/src/{rpc.rs,protocol/common.rs,
//! protocol/v2/*.rs}`（Apache-2.0）。详见仓库根 `THIRD_PARTY.md`。
//!
//! # 模块组织
//!
//! 原本为单文件 2806 行（240 个类型），现已按领域拆分为多个子模块，本
//! `mod.rs` 负责统一 `pub use` 再导出，保持对外公开 API 不变（crate 内仍以
//! `app_server_protocol::Foo` 引用）。

#![allow(dead_code)]
#![allow(clippy::module_inception)]

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

// ── 协议存根的领域子模块（按上游 app-server-protocol 的职责划分） ──
mod account;
mod approval;
mod config;
mod exec;
mod guardian;
mod hook;
mod jsonrpc;
mod mcp;
mod plugin;
mod protocol_enums;
mod thread_item;
mod turn;

// 对外保持扁平命名空间：所有类型直接以 `app_server_protocol::Foo` 可见。
pub use account::*;
pub use approval::*;
pub use config::*;
pub use exec::*;
pub use guardian::*;
pub use hook::*;
pub use jsonrpc::*;
pub use mcp::*;
pub use plugin::*;
pub use protocol_enums::*;
pub use thread_item::*;
pub use turn::*;
