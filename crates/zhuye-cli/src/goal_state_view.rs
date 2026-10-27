//! 目标模式状态的人读渲染(v1.5 目标状态快照读口接线)。
//!
//! 背景:submodule 的 `AgentThread::goal_snapshot()` 返回
//! `zhuye_goal::GoalState` —— 目标文本、状态、累计 token、goal turn 数、
//! 连续 blocked 计数、每轮 LLM 判定历史。协议侧文档明写这是给 GUI 用的
//! 「权威读口」:此前 GUI 只能靠前端投影猜「目标模式是否活跃」,答不出
//! 「跑到第几轮 / LM 判定是什么 / 累计多少 token / 被什么卡住」。
//!
//! 本模块只做纯格式化(无 IO、无全局状态),便于单测覆盖。

use zhuye_goal::{GoalState, GoalStatus, GoalTurnRecord, GoalVerdict};

/// `/goal` 的用法提示(两条分发通道共用)。
pub const GOAL_USAGE: &str =
    "/goal: 用法 /goal <objective> [--verify <cmd>] [--budget <tokens>]  (如 /goal 重构鉴权模块)";

/// `/goal <objective>` 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalCommand {
    /// 目标文本(去掉选项后的剩余部分)。
    pub objective: String,
    /// 命令式校验命令(`--verify <cmd>`)—— 跑通才算达成目标。
    pub verify_command: Option<String>,
    /// token 预算上限(`--budget <n>`)。
    pub token_budget: Option<u64>,
}

/// 解析 `/goal <objective> [--verify <cmd>] [--budget <n>]`。
///
/// 返回 `None` = 没有目标文本(调用方应给用法提示)。
///
/// 选项语法刻意做成最小集:
/// - `--verify <cmd>` —— 校验命令(可含空格,取到下一个 `--` 之前的内容);
/// - `--budget <n>` —— token 预算(纯数字)。
///
/// 未知选项 → 忽略(当普通文本并入目标),不报错 —— slash 命令不应因为
/// 多打了几个字就整个失败。
pub fn parse_goal_command(input: &str) -> Option<GoalCommand> {
    let mut objective = String::new();
    let mut verify_command: Option<String> = None;
    let mut token_budget: Option<u64> = None;
    let mut rest = input.trim();
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix("--verify") {
            let tail = tail.trim_start();
            // `--verify=cmd` 与 `--verify cmd` 两种写法都认。
            let (cmd, remainder) = if let Some(cmd) = tail.strip_prefix('=') {
                let (cmd, rem) = split_at_next_flag(cmd);
                (cmd, rem)
            } else {
                let (cmd, rem) = split_at_next_flag(tail);
                (cmd, rem)
            };
            // 切分点落在下一个 `--` 之前,故 cmd 尾部常带一个空格 —— 必须
            // trim,否则校验命令会变成 `cargo test `。
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                verify_command = Some(cmd.to_string());
            }
            rest = remainder.trim_start();
            continue;
        }
        if let Some(tail) = rest.strip_prefix("--budget") {
            let tail = tail.trim_start();
            let (num, remainder) = if let Some(n) = tail.strip_prefix('=') {
                let (n, rem) = split_at_next_flag(n);
                (n, rem)
            } else {
                let (cmd, rem) = split_at_next_flag(tail);
                (cmd, rem)
            };
            if let Ok(n) = num.trim().parse::<u64>() {
                token_budget = Some(n);
            }
            rest = remainder.trim_start();
            continue;
        }
        // 普通文本 → 并入目标。
        if !objective.is_empty() {
            objective.push(' ');
        }
        let (word, remainder) = split_at_next_flag(rest);
        objective.push_str(word);
        rest = remainder.trim_start();
    }
    let objective = objective.trim().to_string();
    if objective.is_empty() {
        None
    } else {
        Some(GoalCommand {
            objective,
            verify_command,
            token_budget,
        })
    }
}

/// 在下一个 `--` 选项(或字符串末尾)之前切一段文本。
fn split_at_next_flag(s: &str) -> (&str, &str) {
    match s.find("--") {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

/// 把目标状态快照格式化成给人看的多行文本。
///
/// 形态示例:
/// ```text
/// /goal: 目标模式 · active
///   目标: refactor auth module
///   进度: 3 turn(s) · 12.4k tokens
///   校验: 未达成(剩 2 项待办)
///   阻塞: 连 2 回合受阻(阈值 3)
/// ```
///
/// 无活跃目标时返回一句明确的说明(而不是空串)。
pub fn format_goal_state(state: &GoalState) -> String {
    let mut out = format!("/goal: 目标模式 · {}\n", status_label(state.status));
    out.push_str(&format!("  目标: {}\n", state.goal));
    out.push_str(&format!(
        "  进度: {} turn(s) · {} tokens",
        state.turn_count, state.tokens_used
    ));
    if let Some(cmd) = &state.verify_command {
        out.push_str(&format!("\n  校验命令: {cmd}"));
    }
    if let Some(budget) = state.token_budget {
        out.push_str(&format!(
            "\n  token 预算: {} / {}",
            state.tokens_used, budget
        ));
    }
    // 3-strike 计数:未达阈值时也要显示(让用户知道还差几次)。
    if state.consecutive_blocked_turns > 0 {
        out.push_str(&format!(
            "\n  阻塞: 连 {} 回合受阻(阈值 {})",
            state.consecutive_blocked_turns,
            zhuye_goal::BLOCKED_THRESHOLD
        ));
    }
    // 最近一轮判定:给出「LM 认为还差什么」。
    if let Some(last) = state.turns.last() {
        out.push_str(&format!("\n  最近判定: {}", format_verdict(&last.verdict)));
        if let Some(passed) = last.command_passed {
            out.push_str(&format!(
                " · 校验命令 {}",
                if passed { "通过" } else { "未通过" }
            ));
        }
    }
    out
}

/// 目标状态的人读标签。
pub fn status_label(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "进行中",
        GoalStatus::Paused => "已暂停",
        GoalStatus::Blocked => "受阻",
        GoalStatus::BudgetLimited => "预算耗尽",
        GoalStatus::Complete => "已完成",
    }
}

/// LLM 判定的人读形式。
pub fn format_verdict(v: &GoalVerdict) -> String {
    match v {
        GoalVerdict::Met { evidence } => {
            if evidence.is_empty() {
                "已达成".to_string()
            } else {
                format!("已达成({} 条证据)", evidence.len())
            }
        }
        GoalVerdict::Unmet { remaining } => {
            if remaining.is_empty() {
                "未达成".to_string()
            } else {
                format!("未达成(剩 {} 项)", remaining.len())
            }
        }
        GoalVerdict::Blocked { reason } => format!("受阻({reason})"),
    }
}

/// 单轮 goal 记录的一行摘要(供历史面板 / 日志)。
pub fn format_turn_record(r: &GoalTurnRecord) -> String {
    format!(
        "turn {} · {} · {}",
        r.turn_index,
        status_label(r.status_after),
        format_verdict(&r.verdict)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn state() -> GoalState {
        GoalState::new(
            "refactor auth module",
            Some("cargo test".into()),
            Some(50_000),
        )
    }

    #[test]
    fn active_state_shows_goal_and_progress() {
        let s = state();
        let out = format_goal_state(&s);
        assert!(out.contains("目标模式 · 进行中"));
        assert!(out.contains("refactor auth module"));
        assert!(out.contains("0 turn(s)"));
        assert!(out.contains("cargo test"));
        assert!(out.contains("token 预算: 0 / 50000"));
    }

    #[test]
    fn blocked_counter_is_shown_even_below_threshold() {
        let mut s = state();
        s.consecutive_blocked_turns = 2;
        let out = format_goal_state(&s);
        assert!(out.contains("连 2 回合受阻"));
        assert!(out.contains("阈值 3"));
    }

    #[test]
    fn last_verdict_is_surfaced() {
        let mut s = state();
        s.turns.push(GoalTurnRecord {
            turn_index: 1,
            status_after: GoalStatus::Active,
            verdict: GoalVerdict::Unmet {
                remaining: vec!["a".into(), "b".into()],
            },
            command_passed: Some(false),
            tokens_used_this_turn: 1234,
            at: Utc::now(),
        });
        let out = format_goal_state(&s);
        assert!(out.contains("最近判定: 未达成(剩 2 项)"));
        assert!(out.contains("校验命令 未通过"));
    }

    #[test]
    fn met_verdict_with_evidence_counts_items() {
        let v = GoalVerdict::Met {
            evidence: vec!["e1".into()],
        };
        assert_eq!(format_verdict(&v), "已达成(1 条证据)");
    }

    #[test]
    fn empty_evidence_renders_plain_met() {
        let v = GoalVerdict::Met { evidence: vec![] };
        assert_eq!(format_verdict(&v), "已达成");
    }

    #[test]
    fn blocked_verdict_includes_reason() {
        let v = GoalVerdict::Blocked {
            reason: "缺依赖".into(),
        };
        assert_eq!(format_verdict(&v), "受阻(缺依赖)");
    }

    #[test]
    fn status_labels_cover_all_variants() {
        assert_eq!(status_label(GoalStatus::Active), "进行中");
        assert_eq!(status_label(GoalStatus::Paused), "已暂停");
        assert_eq!(status_label(GoalStatus::Blocked), "受阻");
        assert_eq!(status_label(GoalStatus::BudgetLimited), "预算耗尽");
        assert_eq!(status_label(GoalStatus::Complete), "已完成");
    }

    #[test]
    fn turn_record_line_is_stable() {
        let r = GoalTurnRecord {
            turn_index: 2,
            status_after: GoalStatus::Blocked,
            verdict: GoalVerdict::Blocked { reason: "x".into() },
            command_passed: None,
            tokens_used_this_turn: 0,
            at: Utc::now(),
        };
        assert_eq!(format_turn_record(&r), "turn 2 · 受阻 · 受阻(x)");
    }

    #[test]
    fn parse_plain_objective() {
        let c = parse_goal_command("refactor auth module").unwrap();
        assert_eq!(c.objective, "refactor auth module");
        assert!(c.verify_command.is_none());
        assert!(c.token_budget.is_none());
    }

    #[test]
    fn parse_verify_and_budget() {
        let c = parse_goal_command("ship it --verify cargo test --budget 50000").unwrap();
        assert_eq!(c.objective, "ship it");
        assert_eq!(c.verify_command.as_deref(), Some("cargo test"));
        assert_eq!(c.token_budget, Some(50000));
    }

    #[test]
    fn parse_equals_syntax() {
        let c = parse_goal_command("ship it --verify=cargo+test --budget=1000").unwrap();
        assert_eq!(c.objective, "ship it");
        assert_eq!(c.verify_command.as_deref(), Some("cargo+test"));
        assert_eq!(c.token_budget, Some(1000));
    }

    #[test]
    fn parse_rejects_empty_objective() {
        assert!(parse_goal_command("").is_none());
        assert!(parse_goal_command("   ").is_none());
        assert!(parse_goal_command("--verify cargo test").is_none());
    }

    #[test]
    fn parse_keeps_multiword_objective() {
        let c = parse_goal_command("make the build fast again").unwrap();
        assert_eq!(c.objective, "make the build fast again");
    }

    #[test]
    fn parse_invalid_budget_is_ignored() {
        let c = parse_goal_command("ship it --budget abc").unwrap();
        assert_eq!(c.objective, "ship it");
        assert!(c.token_budget.is_none());
    }
}
