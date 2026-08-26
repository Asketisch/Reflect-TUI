//! 为 tui_core:: 命名空间兼容性对 crate 级 terminal_probe 的再导出。
//!
//! 真正的 `terminal_probe` 模块位于 crate 根（`crate::terminal_probe`）。
//! 此再导出使使用 `crate::tui_core::terminal_probe::*` 的供应商上游代码
//! 能够正确解析。
pub use crate::terminal_probe::*;
