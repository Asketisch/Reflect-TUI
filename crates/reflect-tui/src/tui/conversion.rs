//! 事件转换与内容格式化辅助。从 tui/mod.rs 抽出。

use super::*;

pub(super) fn convert_event(event: reflect_protocol::Event) -> Option<UiEvent> {
    use reflect_protocol::event_msg::EventMsg::*;
    let kind = match event.msg {
        TurnStarted(ts) => UiEventKind::TurnStarted {
            turn_id: ts.turn_id,
        },
        TurnComplete(_) => UiEventKind::TurnCompleted,
        TurnAborted(_) => UiEventKind::TurnCompleted,
        AgentMessageDelta(d) => UiEventKind::AgentDelta(d.delta),
        AgentMessage(msg) => UiEventKind::AgentMessage(msg.text),
        ThinkingDelta(td) => UiEventKind::Thinking(td.delta),
        ToolCallBegin(tb) => UiEventKind::ToolStarted {
            call_id: tb.call_id,
            name: tb.tool_name,
        },
        ToolCallEnd(te) => UiEventKind::ToolCompleted {
            call_id: te.call_id.clone(),
            output: format_content_blocks(&te.output.content),
            error: te.is_error,
            elapsed_ms: te.elapsed_ms,
            // 旁路保留 unified_diff 原文(不经 format_content_blocks 截断),
            // 供 /diff 全屏视图。多个 Diff 块用 `\n` 拼接。
            diff: extract_diff(&te.output.content),
        },
        Error(e) => UiEventKind::Error(e.message),
        StreamError(e) => UiEventKind::Error(format!("stream error: {}", e.message)),
        PlanReady(pr) => UiEventKind::PlanReady {
            plan_id: pr.plan_id.to_string(),
            markdown: pr.markdown,
            path: pr.path,
        },
        PlanRequest(pr) => UiEventKind::PlanRequest {
            plan_id: pr.plan_id.to_string(),
            task: pr.task,
        },
        PlanApproved(pa) => UiEventKind::PlanApproved {
            plan_id: pa.plan_id.to_string(),
        },
        PlanRejected(pr) => UiEventKind::PlanRejected {
            plan_id: pr.plan_id.to_string(),
            reason: pr.reason,
        },
        // v1.x Plan mode 草稿预览:agent 写盘到 .reflect/plan/ 后即时推到
        // TUI 对话流,**不**触发 approval modal。多次写盘即多次刷新,
        // 让用户实时看到 plan 演化。
        PlanDraftUpdated(pdu) => UiEventKind::PlanDraftUpdated {
            draft_id: pdu.draft_id,
            markdown: pdu.markdown,
            path: pdu.path,
        },
        PermissionModeChanged(pmce) => UiEventKind::PermissionModeChanged {
            from: pmce.from,
            to: pmce.to,
        },
        ApprovalRequest(ar) => {
            let name = match &ar.kind {
                reflect_protocol::ApprovalKind::Tool { tool_name, .. } => tool_name.clone(),
                reflect_protocol::ApprovalKind::Hook { hook_name, .. } => hook_name.clone(),
                reflect_protocol::ApprovalKind::Plan { summary, .. } => summary.clone(),
            };
            UiEventKind::ApprovalNeeded {
                id: ar.request_id,
                summary: name,
                kind: ar.kind,
            }
        }
        AskUserQuestion(aq) => {
            let n = aq.questions.len();
            if n == 1 {
                UiEventKind::Notice(format!("Question: {}", aq.questions[0].question))
            } else {
                UiEventKind::Notice(format!("{n} questions asked"))
            }
        }
        AskUserInput(ai) => UiEventKind::Notice(format!("Agent asks: {}", ai.prompt)),
        TokenCount(tc) => UiEventKind::TokenCount {
            input_tokens: tc.input_tokens,
            cached_tokens: tc.cached_tokens,
            cost_usd: tc.cost_usd,
        },
        SessionConfigured(sc) => UiEventKind::ContextWindowConfigured {
            window_size: sc.context_window_size,
            // sc.model 已包含完整模型名(如 `minimax/minimax-m3`),直接用。
            model: Some(sc.model.clone()),
        },
        PlanStep(ps) => UiEventKind::PlanStep {
            index: ps.index,
            total: ps.total,
            status: map_plan_step_status(ps.status),
            title: ps.title,
        },
        CollabStarted(cs) => UiEventKind::CollabStarted {
            id: cs.id,
            participants: cs.participants,
        },
        CollabFinished(cf) => UiEventKind::CollabFinished {
            id: cf.id,
            outcome: cf.outcome,
            rounds: cf.rounds,
        },
        // v1.3 SDK 远程工具执行请求仅存在于 serve 模式(SDK 客户端经
        // Op::RegisterTools 注册远程工具后,core 请客户端本地执行并等回执)。
        // TUI 内置运行 agent、不注册远程工具,此事件按构造不可能出现,忽略。
        ToolExecutionRequest(_) | ShutdownComplete | PermissionBubble(_) | ContextCompacted(_)
        | ConfigReloaded(_) | TurnRewound(_) | CollabMessage(_) | McpServerStarted(_)
        | McpServerFailed(_) | McpToolInvoked(_) | LspServerStarted(_) | LspServerFailed(_)
        | PluginLoaded(_) | Routing(_) | QuotaExhausted(_) => return None,
        // v1.3 SDK:serve 在 per-turn 通道排空后发出的收尾标记,仅存在于
        // serve 模式的 SDK 客户端;TUI 内嵌运行不经 serve,按构造不可能收到。
        SubmissionClosed => return None,
    };
    Some(UiEvent { kind })
}

pub(super) fn format_content_blocks(blocks: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        if !out.is_empty() {
            out.push(' ');
        }
        match block {
            ContentBlock::Text { text } => out.push_str(text),
            ContentBlock::Image { mime_type, .. } => {
                out.push_str(&format!("[image: {mime_type}]"));
            }
            ContentBlock::Diff { unified_diff } => {
                if unified_diff.len() > 500 {
                    // 必须按 UTF-8 字符边界截断,否则多字节码点(CJK/emoji)中间切片会 panic。
                    // 与下方 2000 字节截断保持一致的 floor_char_boundary 模式。
                    let cut = unified_diff.floor_char_boundary(500);
                    out.push_str(&unified_diff[..cut]);
                    out.push('…');
                } else {
                    out.push_str(unified_diff);
                }
            }
            ContentBlock::ToolUse { name, .. } => {
                out.push_str(&format!("[tool_use: {name}]"));
            }
            ContentBlock::ToolResult { call_id, output } => {
                let inner = format_content_blocks(&output.content);
                out.push_str(&format!("[tool:{call_id}: {inner}]"));
            }
        }
    }
    if out.len() > 2000 {
        let max_len = 2000usize.saturating_sub("…".len());
        let cut = out.floor_char_boundary(max_len.min(out.len()));
        out.truncate(cut);
        out.push('…');
    }
    out
}

/// 从工具产出的 content blocks 里抽取 unified_diff 原文(不截断)。
///
/// 与 `format_content_blocks` 不同,这里保留完整 diff(可能跨多个 Diff 块,
/// 用 `\n` 拼接),供 /diff 全屏视图。无 Diff 块 → `None`。
pub(super) fn extract_diff(blocks: &[ContentBlock]) -> Option<String> {
    let mut diffs: Vec<&str> = Vec::new();
    for block in blocks {
        if let ContentBlock::Diff { unified_diff } = block {
            if !unified_diff.trim().is_empty() {
                diffs.push(unified_diff.as_str());
            }
        }
    }
    if diffs.is_empty() {
        None
    } else {
        Some(diffs.join("\n"))
    }
}

/// 协议 PlanStepStatus → 本地 PlanStepStatus(供 /tasks overlay)。
pub(super) fn map_plan_step_status(
    s: reflect_protocol::PlanStepStatus,
) -> crate::events::PlanStepStatus {
    use reflect_protocol::PlanStepStatus as P;
    match s {
        P::Pending => crate::events::PlanStepStatus::Pending,
        P::InProgress => crate::events::PlanStepStatus::InProgress,
        P::Done => crate::events::PlanStepStatus::Done,
        P::Skipped => crate::events::PlanStepStatus::Skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reflect_protocol::{ContentBlock, ToolOutput};

    fn diff_block(s: &str) -> ContentBlock {
        ContentBlock::Diff {
            unified_diff: s.to_string(),
        }
    }

    #[test]
    fn format_content_blocks_truncates_multibyte_diff_on_char_boundary() {
        // 构造一个长度 > 500 字节、且 500 字节处正好落在多字节 UTF-8 码点
        // (U+4E2D「中」= 0xE4 0xB8 0xAD) 内部的 diff。旧实现按字节切片
        // `&s[..500]` 在此处会 panic;floor_char_boundary 必须回退到字符边界。
        // 前 498 字节为 ASCII('a' * 498),随后「中」占字节 498/499/500,
        // 即 byte 500 是「中」的尾字节(0xAD),切片点落在多字节码点内部。
        let prefix = "a".repeat(498);
        let diff = format!("{prefix}中中");
        assert_eq!(diff.len(), 504, "前置条件:总长 504 字节");
        assert_eq!(diff.as_bytes()[500], 0xAD, "前置条件:byte 500 是「中」的尾字节(码点内部)");

        let out = format_content_blocks(&[diff_block(&diff)]);
        assert!(out.ends_with('…'), "截断后应以省略号结尾");
        // 截断点必须回退到 498(最后一个完整 ASCII 字符),保留「a…」而非半个字符。
        assert!(
            out.ends_with("a…"),
            "截断应在字符边界,实际结尾:{:?}",
            out
        );
        // 且不应 panic(本测试若能跑到断言即说明未 panic)。
    }

    #[test]
    fn format_content_blocks_truncates_pure_ascii_diff() {
        // 纯 ASCII diff 截断点恰好在 500,行为不变(回退到自身)。
        let diff = "a".repeat(600);
        let out = format_content_blocks(&[diff_block(&diff)]);
        assert!(out.ends_with('…'));
        // 纯 ASCII 下截断后长度应为 500 + 1('…')。
        let body = out.strip_suffix('…').unwrap();
        assert_eq!(body.chars().count(), 500);
    }

    #[test]
    fn format_content_blocks_keeps_short_diff_intact() {
        let diff = "--- a/foo\n+++ b/foo\n@@ -1 +1 @@\n-old\n+new";
        let out = format_content_blocks(&[diff_block(diff)]);
        assert_eq!(out, diff);
        assert!(!out.ends_with('…'));
    }

    #[test]
    fn extract_diff_none_when_no_diff_blocks() {
        let blocks = vec![ContentBlock::Text {
            text: "hello".into(),
        }];
        assert_eq!(extract_diff(&blocks), None);
    }

    #[test]
    fn extract_diff_returns_full_unified_diff() {
        let blocks = vec![diff_block("--- a/foo\n+++ b/foo\n@@ -1 +1 @@\n-old\n+new")];
        let out = extract_diff(&blocks).unwrap();
        assert!(out.contains("--- a/foo"));
        assert!(out.contains("+new"));
        assert!(!out.ends_with('…'), "不应截断");
    }

    #[test]
    fn extract_diff_skips_empty_diffs() {
        let blocks = vec![diff_block("   \n  "), diff_block("real diff")];
        let out = extract_diff(&blocks).unwrap();
        assert_eq!(out, "real diff");
    }

    #[test]
    fn extract_diff_joins_multiple_diffs() {
        let blocks = vec![diff_block("diff1"), diff_block("diff2")];
        let out = extract_diff(&blocks).unwrap();
        assert!(out.contains("diff1"));
        assert!(out.contains("diff2"));
    }

    #[test]
    fn toolcallend_diff_preserved_through_convert() {
        // 端到端:ToolCallEnd 带 Diff 块 → UiEventKind::ToolCompleted.diff 有值。
        let te = reflect_protocol::ToolCallEndEvent {
            call_id: "c1".into(),
            output: ToolOutput {
                content: vec![diff_block("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b")],
                is_error: false,
                metadata: serde_json::Value::Null,
                elapsed_ms: 5,
            },
            is_error: false,
            elapsed_ms: 5,
            child_id: None,
        };
        let ev = reflect_protocol::Event::new(
            "id",
            reflect_protocol::event_msg::EventMsg::ToolCallEnd(te),
        );
        let ui = convert_event(ev).unwrap();
        match ui.kind {
            UiEventKind::ToolCompleted { diff, .. } => {
                assert!(diff.is_some(), "diff 应被保留");
                assert!(diff.unwrap().contains("+b"));
            }
            _ => panic!("expected ToolCompleted"),
        }
    }

    // ── v1.x Plan mode 事件转换回归 ─────────────────────────────────────

    #[test]
    fn convert_plan_ready_carries_plan_id_and_markdown() {
        use reflect_protocol::event_msg::EventMsg;
        use reflect_protocol::{event_msg::PlanReadyEvent, item::PlanId};
        let ev = reflect_protocol::Event::new(
            "id-pr",
            EventMsg::PlanReady(PlanReadyEvent {
                plan_id: PlanId::default(),
                markdown: "# plan body".into(),
                path: None,
            }),
        );
        let ui = convert_event(ev).unwrap();
        match ui.kind {
            UiEventKind::PlanReady {
                plan_id,
                markdown,
                path: _,
            } => {
                assert!(!plan_id.is_empty(), "plan_id 应非空");
                assert_eq!(markdown, "# plan body");
            }
            other => panic!("expected PlanReady, got {other:?}"),
        }
    }

    #[test]
    fn convert_permission_mode_changed_not_dropped() {
        // 回归:此前 PermissionModeChanged 在 conversion 里 return None(丢弃)。
        use reflect_protocol::PermissionMode;
        use reflect_protocol::event_msg::EventMsg;
        use reflect_protocol::event_msg::PermissionModeChangedEvent;
        let ev = reflect_protocol::Event::new(
            "id-pmc",
            EventMsg::PermissionModeChanged(PermissionModeChangedEvent {
                from: PermissionMode::Auto,
                to: PermissionMode::Plan,
            }),
        );
        let ui = convert_event(ev).expect("PermissionModeChanged 不应再被丢弃");
        match ui.kind {
            UiEventKind::PermissionModeChanged { from, to } => {
                assert_eq!(from, PermissionMode::Auto);
                assert_eq!(to, PermissionMode::Plan);
            }
            other => panic!("expected PermissionModeChanged, got {other:?}"),
        }
    }

    #[test]
    fn convert_plan_approved_and_rejected_not_dropped() {
        use reflect_protocol::event_msg::EventMsg;
        use reflect_protocol::event_msg::{PlanApprovedEvent, PlanRejectedEvent};
        use reflect_protocol::item::PlanId;

        let ev_a = reflect_protocol::Event::new(
            "id-pa",
            EventMsg::PlanApproved(PlanApprovedEvent {
                plan_id: PlanId::default(),
            }),
        );
        let ui_a = convert_event(ev_a).expect("PlanApproved 不应被丢弃");
        assert!(matches!(ui_a.kind, UiEventKind::PlanApproved { .. }));

        let ev_r = reflect_protocol::Event::new(
            "id-pr",
            EventMsg::PlanRejected(PlanRejectedEvent {
                plan_id: PlanId::default(),
                reason: Some("nope".into()),
            }),
        );
        let ui_r = convert_event(ev_r).expect("PlanRejected 不应被丢弃");
        match ui_r.kind {
            UiEventKind::PlanRejected { reason, .. } => assert_eq!(reason.as_deref(), Some("nope")),
            other => panic!("expected PlanRejected, got {other:?}"),
        }
    }
}
