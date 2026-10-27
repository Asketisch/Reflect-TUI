//! 路由事件的人读格式化(v1.0 多 Provider 路由 / v1.7 跨模型降级)。
//!
//! 背景:`zhuye_protocol::EventMsg::Routing` 描述「换 credential / 进冷却 /
//! 全部失败 / 跨模型降级」。协议侧文档明写「TUI 收到后画一行 status」,
//! 但本 TUI 此前把整个变体丢弃 —— 于是用户配了 `[routing] fallbacks` 后,
//! provider 挂了自动降级到备用模型,**界面上零提示**,看起来像
//! 「模型没变却又成功了」。
//!
//! 本模块只做纯格式化(无 IO、无全局状态),便于单测覆盖。

use zhuye_protocol::event_msg::{RoutingEvent, RoutingEventKind};

/// 把一条路由事件格式化成一行人读文本(进对话流的 Notice 行)。
///
/// 形态示例:
/// - `↻ 路由切换 · main: cred-a → cred-b(rate_limited)`
/// - `⏳ 冷却中 · main: cred-a(剩余 120s)`
/// - `✓ 冷却解除 · main: cred-a`
/// - `✗ 路由失败 · main: 无可用 credential(last error)`
/// - `↘ 模型降级 · main: openai/gpt-4 → anthropic/claude-3`
pub fn format_routing_event(ev: &RoutingEvent) -> String {
    let role = if ev.role.is_empty() {
        "main"
    } else {
        ev.role.as_str()
    };
    let from = ev.from_credential.as_deref().unwrap_or("-");
    let to = ev.to_credential.as_deref().unwrap_or("-");
    match ev.kind {
        RoutingEventKind::Switched => {
            // 同池内换 credential:from/to 都是凭据 label。
            format!("↻ 路由切换 · {role}: {from} → {to}({})", ev.reason)
        }
        RoutingEventKind::CooldownStarted => {
            let cooldown = match ev.cooldown_until_ms {
                Some(ms) => format!("(剩余 {}s)", ms.div_ceil(1000)),
                None => String::new(),
            };
            format!("⏳ 冷却中 · {role}: {from}{cooldown}")
        }
        RoutingEventKind::CooldownCleared => {
            format!("✓ 冷却解除 · {role}: {from}")
        }
        RoutingEventKind::FailedOver => {
            format!("✗ 路由失败 · {role}: 无可用 credential({})", ev.reason)
        }
        RoutingEventKind::SpecFallback => {
            // 跨模型降级:from/to 装的是 `provider/model` 形式的 spec。
            format!("↘ 模型降级 · {role}: {from} → {to}({})", ev.reason)
        }
    }
}

/// 路由事件是否值得打扰用户(进对话流)。
///
/// `CooldownCleared` 是恢复类信号,单独一行价值不大(用户此时更关心
/// 「请求成功了」),因此不单独打扰;其余四类都是「发生了什么意外」的
/// 关键信息,必须让用户看见。
pub fn is_noteworthy(ev: &RoutingEvent) -> bool {
    !matches!(ev.kind, RoutingEventKind::CooldownCleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: RoutingEventKind, reason: &str) -> RoutingEvent {
        RoutingEvent {
            kind,
            role: "main".into(),
            from_credential: Some("cred-a".into()),
            to_credential: Some("cred-b".into()),
            reason: reason.into(),
            cooldown_until_ms: None,
        }
    }

    #[test]
    fn switched_shows_both_credentials_and_reason() {
        let s = format_routing_event(&ev(RoutingEventKind::Switched, "rate_limited"));
        assert_eq!(s, "↻ 路由切换 · main: cred-a → cred-b(rate_limited)");
    }

    #[test]
    fn cooldown_started_shows_remaining_seconds() {
        let mut e = ev(RoutingEventKind::CooldownStarted, "429");
        e.cooldown_until_ms = Some(120_000);
        let s = format_routing_event(&e);
        assert_eq!(s, "⏳ 冷却中 · main: cred-a(剩余 120s)");
    }

    #[test]
    fn cooldown_cleared_is_not_noteworthy() {
        let e = ev(RoutingEventKind::CooldownCleared, "ok");
        assert!(!is_noteworthy(&e));
        assert_eq!(format_routing_event(&e), "✓ 冷却解除 · main: cred-a");
    }

    #[test]
    fn failed_over_is_noteworthy() {
        let e = ev(RoutingEventKind::FailedOver, "all_exhausted");
        assert!(is_noteworthy(&e));
        assert!(format_routing_event(&e).contains("无可用 credential"));
    }

    #[test]
    fn spec_fallback_shows_model_specs() {
        let mut e = ev(RoutingEventKind::SpecFallback, "no_credential");
        e.from_credential = Some("openai/gpt-4".into());
        e.to_credential = Some("anthropic/claude-3".into());
        let s = format_routing_event(&e);
        assert_eq!(
            s,
            "↘ 模型降级 · main: openai/gpt-4 → anthropic/claude-3(no_credential)"
        );
        assert!(is_noteworthy(&e));
    }

    #[test]
    fn missing_credentials_render_as_dash() {
        let mut e = ev(RoutingEventKind::Switched, "x");
        e.from_credential = None;
        e.to_credential = None;
        assert_eq!(format_routing_event(&e), "↻ 路由切换 · main: - → -(x)");
    }

    #[test]
    fn empty_role_falls_back_to_main() {
        let mut e = ev(RoutingEventKind::Switched, "x");
        e.role = String::new();
        assert!(format_routing_event(&e).contains("main:"));
    }
}
