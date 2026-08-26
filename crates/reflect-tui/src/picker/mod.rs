//! v1.x Tier 4.5: 各种 picker overlay widgets。
//!
//! 所有 picker 复用 `plan_approval_block` 的居中覆盖层布局(由各文件内
//! `centered_rect` helper 实现),由 `tui/mod.rs::draw` 在主布局之上调用。

pub mod checkpoint_overlay;
pub mod copy_history;
pub mod fork_rewind;
pub mod image_picker;
pub mod keymap_picker;
pub mod mcp_overlay;
pub mod plugin_overlay;
pub mod skills_hub;
pub mod statusline_picker;
pub mod theme_picker;
pub mod traces_overlay;
