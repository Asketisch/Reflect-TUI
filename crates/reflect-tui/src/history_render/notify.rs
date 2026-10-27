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
    /// 本回合是否出现过工具调用 / 子代理活动 —— 用来判断「值得通知」。
    ///
    /// 对齐 claude code:不是每个回合结束都响。用户刚敲了回车、盯着屏幕等
    /// 结果时来一条 "Reflect finished." 是纯噪音;真正的价值是「你切去别的
    /// 窗口干活,这边跑完了」。
    pub did_work: bool,
}

/// 回合时长阈值(秒):超过它才在无工具调用时也通知。
///
/// 纯对话回合通常几秒内结束,用户还在看屏幕;跑很久的回合才需要把人叫回来。
pub const LONG_TURN_SECS: u64 = 30;

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
            // 工具调用 / 子代理活动说明这一回合干了实事,值得把人叫回来。
            UiEventKind::ToolStarted { .. }
            | UiEventKind::ToolCompleted { .. }
            | UiEventKind::SubagentStatusUpdate { .. } => self.did_work = true,
            _ => {}
        }
    }

    /// 优先级:错误 > 审批 > 回合完成(一次只发一条,避免连响)。
    ///
    /// 回合完成**不是**无条件通知:只有干了实事(工具调用 / 子代理)或跑得够久
    /// 才发。否则用户刚敲完回车盯着屏幕,每轮都响一声是纯噪音。
    pub fn message(&self) -> Option<String> {
        self.message_with(None)
    }

    /// 同 [`Self::message`],但受 `[notifications.tui]` 段的开关约束。
    ///
    /// v1.x 接线:submodule 的 `zhuye_config::TuiNotificationsSection` 定义了
    /// `enabled` / `on_turn_complete` / `on_approval` / `on_error` 四个开关,
    /// 此前 TUI 完全没读 —— 用户在配置里写了 `on_turn_complete = false`
    /// 也不会有任何效果(死配置)。
    ///
    /// `section = None`(配置段缺省)→ 全开(与内置默认一致)。
    pub fn message_with(
        &self,
        section: Option<&zhuye_config::TuiNotificationsSection>,
    ) -> Option<String> {
        if let Some(e) = &self.error {
            if !allows(section, NotifyTrigger::Error) {
                return None;
            }
            return Some(format!("Reflect error: {e}"));
        }
        if let Some(a) = &self.approval_needed {
            if !allows(section, NotifyTrigger::Approval) {
                return None;
            }
            return Some(format!("Reflect needs approval: {a}"));
        }
        if self.turn_completed && self.did_work {
            if !allows(section, NotifyTrigger::TurnComplete) {
                return None;
            }
            return Some("Reflect finished.".to_string());
        }
        None
    }

    /// 回合完成且**没有**工具活动时的通知判定(供主循环结合回合时长决定)。
    ///
    /// 返回 `true` 表示「这一轮虽然没调工具,但跑得够久,仍值得通知」。
    pub fn long_turn_without_work(&self, elapsed_secs: u64) -> bool {
        self.turn_completed && !self.did_work && elapsed_secs >= LONG_TURN_SECS
    }
}

/// 通知触发类型(对应 `[notifications.tui]` 的三个分项开关)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyTrigger {
    TurnComplete,
    Approval,
    Error,
}

/// 该触发类型是否允许发通知。
///
/// 段缺省 → 全开(与内置默认一致);`enabled = false` → 全关。
pub fn allows(
    section: Option<&zhuye_config::TuiNotificationsSection>,
    trigger: NotifyTrigger,
) -> bool {
    let Some(s) = section else {
        return true;
    };
    if !s.enabled {
        return false;
    }
    match trigger {
        NotifyTrigger::TurnComplete => s.on_turn_complete,
        NotifyTrigger::Approval => s.on_approval,
        NotifyTrigger::Error => s.on_error,
    }
}

/// 通知通道选择:`channels` 写了 `"osc9"` / `"bel"` 就按它来,否则自动探测。
///
/// v1.x 接线:`[notifications.tui].channels` 此前同样是死配置 —— 通道永远
/// 由终端探测自动决定,用户指定的后端不生效。
pub fn channel_method(
    section: Option<&zhuye_config::TuiNotificationsSection>,
) -> crate::config_compat::types::NotificationMethod {
    use crate::config_compat::types::NotificationMethod;
    let Some(s) = section else {
        return NotificationMethod::Auto;
    };
    let has = |k: &str| s.channels.iter().any(|c| c.eq_ignore_ascii_case(k));
    match (has("osc9"), has("bel")) {
        (true, false) => NotificationMethod::Osc9,
        (false, true) => NotificationMethod::Bel,
        // 两个都写 / 都没写 → 自动探测(与内置默认一致)。
        _ => NotificationMethod::Auto,
    }
}

/// 发送通知(封装后端调用,失败静默——通知是非关键体验,不应打断主循环)。
pub fn notify(backend: &mut crate::tui_core::notifications::DesktopNotificationBackend, msg: &str) {
    let _ = backend.notify(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(
        enabled: bool,
        on_turn_complete: bool,
        on_approval: bool,
        on_error: bool,
    ) -> zhuye_config::TuiNotificationsSection {
        zhuye_config::TuiNotificationsSection {
            enabled,
            on_turn_complete,
            on_approval,
            on_error,
            channels: vec![],
        }
    }

    #[test]
    fn missing_section_allows_everything() {
        // 段缺省 → 全开(与内置默认一致)。
        for t in [
            NotifyTrigger::TurnComplete,
            NotifyTrigger::Approval,
            NotifyTrigger::Error,
        ] {
            assert!(allows(None, t));
        }
    }

    #[test]
    fn disabled_section_silences_everything() {
        let s = section(false, true, true, true);
        for t in [
            NotifyTrigger::TurnComplete,
            NotifyTrigger::Approval,
            NotifyTrigger::Error,
        ] {
            assert!(!allows(Some(&s), t));
        }
    }

    #[test]
    fn per_trigger_switches_are_honored() {
        let s = section(true, false, true, false);
        assert!(!allows(Some(&s), NotifyTrigger::TurnComplete));
        assert!(allows(Some(&s), NotifyTrigger::Approval));
        assert!(!allows(Some(&s), NotifyTrigger::Error));
    }

    #[test]
    fn message_respects_turn_complete_switch() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::TurnCompleted);
        t.observe(&UiEventKind::ToolStarted {
            call_id: "c".into(),
            name: "bash".into(),
            args: None,
        });
        // 默认开 → 有消息。
        assert!(t.message_with(None).is_some());
        // 关掉 on_turn_complete → 无消息。
        let s = section(true, false, true, true);
        assert!(t.message_with(Some(&s)).is_none());
    }

    #[test]
    fn message_respects_error_switch() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::Error("boom".into()));
        let s = section(true, true, true, false);
        assert!(t.message_with(Some(&s)).is_none());
    }

    #[test]
    fn channel_method_reads_channels_list() {
        use crate::config_compat::types::NotificationMethod;
        let mut s = section(true, true, true, true);
        s.channels = vec!["osc9".into()];
        assert_eq!(channel_method(Some(&s)), NotificationMethod::Osc9);
        s.channels = vec!["bel".into()];
        assert_eq!(channel_method(Some(&s)), NotificationMethod::Bel);
        s.channels = vec![];
        assert_eq!(channel_method(Some(&s)), NotificationMethod::Auto);
        assert_eq!(channel_method(None), NotificationMethod::Auto);
    }

    #[test]
    fn empty_triggers_have_no_message() {
        let t = NotifyTriggers::new();
        assert!(!t.any());
        assert!(t.message().is_none());
    }

    #[test]
    fn turn_completed_notifies_only_when_real_work_happened() {
        // 回归:此前每个回合结束都发 "Reflect finished." —— 用户刚敲完回车
        // 盯着屏幕等结果时,这是纯噪音。对齐 claude code:只在干了实事
        // (工具调用 / 子代理)或跑得够久时才通知。
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::TurnCompleted);
        assert_eq!(t.message(), None, "纯对话回合不应通知");

        // 有工具调用 → 通知。
        t.observe(&UiEventKind::ToolStarted {
            call_id: "c1".into(),
            name: "bash".into(),
            args: None,
        });
        assert_eq!(t.message().as_deref(), Some("Reflect finished."));

        // 长回合例外:没调工具但跑得够久,仍值得把人叫回来。
        let mut long = NotifyTriggers::new();
        long.observe(&UiEventKind::TurnCompleted);
        assert!(!long.long_turn_without_work(LONG_TURN_SECS - 1));
        assert!(long.long_turn_without_work(LONG_TURN_SECS));
        assert!(long.long_turn_without_work(LONG_TURN_SECS + 600));
    }

    #[test]
    fn error_and_approval_outrank_turn_completed() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::TurnCompleted);
        t.observe(&UiEventKind::ToolStarted {
            call_id: "c".into(),
            name: "read".into(),
            args: None,
        });
        t.observe(&UiEventKind::Error("boom".into()));
        assert_eq!(t.message().as_deref(), Some("Reflect error: boom"));

        let mut t2 = NotifyTriggers::new();
        t2.observe(&UiEventKind::ApprovalNeeded {
            id: "a".into(),
            summary: "Bash: rm -rf /".into(),
            kind: zhuye_protocol::ApprovalKind::Tool {
                tool_name: "bash".into(),
                args: serde_json::json!({"cmd": "rm -rf /"}),
            },
        });
        assert_eq!(
            t2.message().as_deref(),
            Some("Reflect needs approval: Bash: rm -rf /")
        );
    }

    #[test]
    fn subagent_activity_counts_as_work() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::TurnCompleted);
        t.observe(&UiEventKind::SubagentStatusUpdate { children: vec![] });
        assert_eq!(t.message().as_deref(), Some("Reflect finished."));
    }

    #[test]
    fn turn_completed_triggers_message() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::TurnCompleted);
        assert!(t.any());
        // 纯对话回合不再通知(见 turn_completed_notifies_only_when_real_work_happened);
        // 这里只断言触发集被记录,以及有工具活动时才产出消息。
        assert_eq!(t.message(), None);
        t.observe(&UiEventKind::ToolStarted {
            call_id: "c1".into(),
            name: "bash".into(),
            args: None,
        });
        assert_eq!(t.message().as_deref(), Some("Reflect finished."));
    }

    #[test]
    fn approval_triggers_message() {
        let mut t = NotifyTriggers::new();
        t.observe(&UiEventKind::ApprovalNeeded {
            id: "r1".into(),
            summary: "run Bash".into(),
            kind: zhuye_protocol::ApprovalKind::Tool {
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
            kind: zhuye_protocol::ApprovalKind::Tool {
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
            kind: zhuye_protocol::ApprovalKind::Tool {
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
