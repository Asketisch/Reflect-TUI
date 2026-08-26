//! 终端通知(BEL / OSC9)触发逻辑。
//!
//! 对齐老 TUI `notifications.rs`:回合结束 / 需要审批 / 出错时发终端通知,
//! 让用户切到别的窗口也能被提醒。后端复用 vendored 的 Reflect
//! `tui_core::notifications`(OSC9/BEL + 终端探测自动选)。
//!
//! 触发判定与文案构建是纯函数(可单测);实际的 escape-sequence 发送在主循环里
//! 调 `DesktopNotificationBackend::notify`(写裸 stdout,与帧缓冲分离)。

use crate::adapter::UiEventKind;

/// 一个回合内捕获的通知触发(可能多条 event 合并成一次通知)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotifyTriggers {
    pub turn_completed: bool,
    pub approval_needed: Option<String>,
    pub error: Option<String>,
}

impl NotifyTriggers {
    pub fn new() -> Self {
        Self::default()
    }

    /// 是否有任意触发。
    pub fn any(&self) -> bool {
        self.turn_completed || self.approval_needed.is_some() || self.error.is_some()
    }

    /// 把一个 UiEventKind 累加进触发集。
    pub fn observe(&mut self, kind: &UiEventKind) {
        match kind {
            UiEventKind::TurnCompleted => self.turn_completed = true,
            UiEventKind::ApprovalNeeded { summary, .. } => {
                self.approval_needed = Some(summary.clone());
            }
            UiEventKind::Error(msg) => {
                self.error = Some(msg.clone());
            }
            _ => {}
        }
    }

    /// 优先级:错误 > 审批 > 回合完成(一次只发一条,避免连响)。
    pub fn message(&self) -> Option<String> {
        if let Some(e) = &self.error {
            return Some(format!("Reflect error: {e}"));
        }
        if let Some(a) = &self.approval_needed {
            return Some(format!("Reflect needs approval: {a}"));
        }
        if self.turn_completed {
            return Some("Reflect finished.".to_string());
        }
        None
    }
}

/// 发送通知(封装后端调用,失败静默——通知是非关键体验,不应打断主循环)。
pub fn notify(backend: &mut crate::tui_core::notifications::DesktopNotificationBackend, msg: &str) {
    let _ = backend.notify(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_triggers_have_no_message() {
        let t = NotifyTriggers::new();
        assert!(!t.any());
        assert!(t.message().is_none());
    }

    #[test]
    fn turn_completed_triggers_message() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::TurnCompleted);
        assert!(t.any());
        assert_eq!(t.message().as_deref(), Some("Reflect finished."));
    }

    #[test]
    fn approval_triggers_message() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::ApprovalNeeded {
            id: "r1".into(),
            summary: "run Bash".into(),
            kind: reflect_protocol::ApprovalKind::Tool {
                tool_name: "bash".into(),
                args: serde_json::Value::Null,
            },
        });
        assert_eq!(
            t.message().as_deref(),
            Some("Reflect needs approval: run Bash")
        );
    }

    #[test]
    fn error_triggers_message() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::Error("boom".into()));
        assert_eq!(t.message().as_deref(), Some("Reflect error: boom"));
    }

    #[test]
    fn error_takes_priority_over_approval_and_turn() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::TurnCompleted);
        t.observe(&UiEventKind::ApprovalNeeded {
            id: "r2".into(),
            summary: "x".into(),
            kind: reflect_protocol::ApprovalKind::Tool {
                tool_name: "x".into(),
                args: serde_json::Value::Null,
            },
        });
        t.observe(&UiEventKind::Error("fatal".into()));
        assert_eq!(t.message().as_deref(), Some("Reflect error: fatal"));
    }

    #[test]
    fn approval_takes_priority_over_turn() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::TurnCompleted);
        t.observe(&UiEventKind::ApprovalNeeded {
            id: "r3".into(),
            summary: "y".into(),
            kind: reflect_protocol::ApprovalKind::Tool {
                tool_name: "y".into(),
                args: serde_json::Value::Null,
            },
        });
        assert_eq!(t.message().as_deref(), Some("Reflect needs approval: y"));
    }

    #[test]
    fn non_trigger_events_are_ignored() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::AgentDelta("hi".into()));
        t.observe(&UiEventKind::Thinking("hmm".into()));
        t.observe(&UiEventKind::Notice("n".into()));
        assert!(!t.any());
        assert!(t.message().is_none());
    }
}
