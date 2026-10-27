//! 上游 `protocol` 模块中被 TUI 引用的最小子集。完整的
//! `EventMsg` / `Op` 枚举位于别处；此桩模块只暴露内置视图
//! 所需的少量常量和辅助函数。

use super::*;

/// 线程目标（objective）所允许的最大字符数。
pub const MAX_THREAD_GOAL_OBJECTIVE_CHARS: usize = 4_000;

/// TUI 在 hooks 浏览器中展示的钩子生命周期事件。上游枚举
/// 派生了 `EnumIter`；这里我们手动实现 `iter`，从而无需
/// 将 `strum` 接入该类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEventName {
    PreToolUse,
    PermissionRequest,
    PostToolUse,
    PreCompact,
    PostCompact,
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    SubagentStart,
    SubagentStop,
    Stop,
}

impl HookEventName {
    /// 按声明顺序遍历所有变体。对应内置 hooks 浏览器所依赖的
    /// `strum::EnumIter::iter` 实现。
    pub fn iter() -> impl Iterator<Item = Self> {
        [
            Self::PreToolUse,
            Self::PermissionRequest,
            Self::PostToolUse,
            Self::PreCompact,
            Self::PostCompact,
            Self::SessionStart,
            Self::SessionEnd,
            Self::UserPromptSubmit,
            Self::SubagentStart,
            Self::SubagentStop,
            Self::Stop,
        ]
        .into_iter()
    }
}

/// 桩事件消息体。真正的上游枚举非常庞大；TUI 只将该类型作为
/// 字段类型引用，因此单元形态的桩即可通过编译。
#[derive(Debug, Clone, Default)]
pub struct EventMsg;

#[derive(Debug, Clone, Default)]
pub struct TurnCompleteEvent;

#[derive(Debug, Clone, Default)]
pub struct TurnStartedEvent;
