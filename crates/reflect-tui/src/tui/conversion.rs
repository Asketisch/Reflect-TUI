//! 事件转换与内容格式化辅助。从 tui/mod.rs 抽出。

use super::*;

/// M4:工具输出流式渲染开关(默认开;`REFLECT_TUI_TOOL_STREAM=0` 关闭)。
/// 关闭时 ToolCallOutputDelta 保持旧行为(忽略),完整输出仍由 ToolCallEnd
/// 展示,不会缺失内容。
fn tool_stream_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("REFLECT_TUI_TOOL_STREAM")
            .map(|v| !(v == "0" || v.eq_ignore_ascii_case("false")))
            .unwrap_or(true)
    })
}

/// 压缩策略的人读标签。协议枚举没有 `Display`,集中在这里避免散落 match。
fn compact_strategy_label(s: &reflect_protocol::ContextCompactedStrategy) -> &'static str {
    use reflect_protocol::ContextCompactedStrategy as S;
    match s {
        S::Noop => "no-op",
        S::Microcompact => "microcompact",
        S::SmartPrune => "smart prune",
        S::LlMSummarize => "LLM 摘要",
    }
}

/// cooldown 秒数 → `1h 23m 45s` 形式(0 秒时给明确结论,不输出空串)。
fn format_window_secs(secs: u64) -> String {
    if secs == 0 {
        return "已重置".to_string();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    let mut out = String::new();
    if h > 0 {
        out.push_str(&format!("{h}h "));
    }
    if h > 0 || m > 0 {
        out.push_str(&format!("{m}m "));
    }
    out.push_str(&format!("{s}s"));
    out
}

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
        // v1.x:结构化问题不再是 Notice 一行了事 —— agent 阻塞等待回执,
        // 必须打开交互面板让用户作答(Op::AskUserQuestionResponse)。
        AskUserQuestion(aq) => UiEventKind::QuestionAsked {
            request_id: aq.request_id,
            questions: aq.questions,
        },
        // v1.x:自由文本询问同理 —— composer 下一条提交捕获为回答。
        AskUserInput(ai) => UiEventKind::AskUserRequested {
            request_id: ai.request_id,
            prompt: ai.prompt,
            secret: ai.secret.unwrap_or(false),
        },
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
        // v1.4 子代理可观测(推送通道):父代理转发子代理中间进度,
        // 以 Notice 行实时进对话流(每个子代理的分组视图留待后续迭代)。
        // Message 直接展示文本;ToolBegin/End 的 text 已是工具名,
        // 补省略号/完成标记区分方向。
        SubagentProgress(sp) => {
            let text = match sp.kind {
                reflect_protocol::event_msg::SubagentProgressKind::Message => sp.text,
                reflect_protocol::event_msg::SubagentProgressKind::ToolBegin => {
                    format!("{} …", sp.text)
                }
                reflect_protocol::event_msg::SubagentProgressKind::ToolEnd => {
                    format!("{} ✓", sp.text)
                }
            };
            UiEventKind::Notice(format!("[子代理 {}] {}", sp.role, text))
        }
        // v1.4 子代理可观测(查询通道):状态快照 → /tasks 面板 agents
        // 列表实时刷新(apply_event 按 child_id upsert)。映射为本地投影。
        SubagentStatus(ev) => UiEventKind::SubagentStatusUpdate {
            children: ev
                .children
                .into_iter()
                .map(|c| crate::adapter::SubagentView {
                    id: c.child_id,
                    role: c.role,
                    running: matches!(c.state, reflect_protocol::SubagentRunStateMirror::Running),
                    current_tool: c.current_tool,
                    iteration: c.iteration,
                    total_tokens: c.total_tokens,
                })
                .collect(),
        },
        // M4:v1.4 工具输出流式增量 → live 区 tail 渲染。开关
        // REFLECT_TUI_TOOL_STREAM=0 关闭(默认开);关闭时保持旧行为,
        // ToolCallEnd 的完整输出仍是唯一展示路径,不会缺失内容。
        ToolCallOutputDelta(d) => {
            if tool_stream_enabled() {
                UiEventKind::ToolOutputDelta {
                    call_id: d.call_id,
                    delta: d.delta,
                }
            } else {
                return None;
            }
        }
        // MCP / LSP 生命周期事件(bootstrap 接线连接管理器后真实到达):
        // 除 Notice 外,McpServerStarted/Failed 还 upsert `state.mcp_servers`
        // 连接态快照,供 /mcp overlay 展示真实状态(此前 overlay 硬编码假列表)。
        McpServerStarted(e) => UiEventKind::McpServerStarted {
            server: e.server,
            tool_names: e.tool_names,
        },
        // 插件挂载成功(bootstrap_plugins per-plugin emit):与 MCP 上线
        // 同款通知行,让用户看到插件能力面(skills / commands 计数)。
        PluginLoaded(e) => UiEventKind::Notice(format!(
            "Plugin '{}' loaded (v{}, {}, {} skills, {} commands)",
            e.plugin, e.version, e.scope, e.skill_count, e.command_count
        )),
        McpServerFailed(e) => UiEventKind::McpServerFailed {
            server: e.server,
            error: e.error,
            will_retry: e.will_retry,
        },
        LspServerStarted(e) => UiEventKind::Notice(format!(
            "LSP server '{}' started ({})",
            e.server,
            e.language_ids.join(", ")
        )),
        LspServerFailed(e) => UiEventKind::Notice(format!(
            "LSP server '{}' failed: {}{}",
            e.server,
            e.error,
            if e.will_retry { " (will retry)" } else { "" }
        )),
        McpToolInvoked(_) => return None,
        // v1.3 SDK 远程工具执行请求仅存在于 serve 模式(SDK 客户端经
        // Op::RegisterTools 注册远程工具后,core 请客户端本地执行并等回执)。
        // TUI 内置运行 agent、不注册远程工具,此事件按构造不可能出现,忽略。
        // v1.5:`/rewind` 的回执 —— 回报实际截断的记录数,让用户确认
        // 对话回退已生效(会话 JSONL 的 .bak 备份由 writer 侧保证)。
        TurnRewound(e) => UiEventKind::Notice(match e.truncated_after {
            0 => "Rewound: nothing to drop (already at the last turn boundary).".to_string(),
            n => format!("Rewound: dropped {n} conversation record(s). A .bak backup was kept."),
        }),
        ToolExecutionRequest(_)
        | ShutdownComplete
        | PermissionBubble(_)
        | ConfigReloaded(_)
        | CollabMessage(_)
        | Routing(_) => return None,
        // 自动压缩:必须让用户看见。此前整条丢弃,结果就是「上下文突然变短」
        // /「模型突然不记得前面说过什么」在界面上毫无解释,用户只能猜。
        // Claude Code 同样会显示 Compacting conversation…。
        ContextCompacted(cc) => UiEventKind::Notice(format!(
            "🗜 已压缩上下文:{} · 移除 {} 条消息 · {} → {} tokens",
            compact_strategy_label(&cc.strategy),
            cc.removed_messages,
            cc.before_tokens,
            cc.after_tokens
        )),
        // 配额耗尽:窗口上限 + 重置时间是用户唯一能据此决策的信息
        // (等 / 换 provider / 换 credential),整条丢弃等于让回合莫名停住。
        QuotaExhausted(qe) => UiEventKind::Notice(format!(
            "⛔ 配额耗尽:{} credential '{}' · 已用 {}/{} tokens · 窗口 {} 后重置",
            qe.provider,
            qe.label,
            qe.used_tokens,
            qe.max_tokens,
            format_window_secs(qe.window_ends_secs)
        )),
        // v1.3 SDK:serve 在 per-turn 通道排空后发出的收尾标记,仅存在于
        // serve 模式的 SDK 客户端;TUI 内嵌运行不经 serve,按构造不可能收到。
        SubmissionClosed => return None,
        // v1.6.1 `/prompt list` 的应答:可用提示词层 + 当前模型生效层。
        // active_layers 只记录用户目录层的命中(内嵌层不记录),空 = 全部
        // 走内嵌默认。作为 Notice 进对话流(选择 overlay 留待后续迭代)。
        PromptList(pl) => UiEventKind::Notice(format!(
            "📋 提示词层 · 模型 {} · 生效: {} · 可用: {}",
            pl.current_spec,
            format_prompt_layers(&pl.active_layers),
            pl.available.join(", "),
        )),
        // v1.6.1 `/prompt reload` 的应答:提示词库已重扫,下一 turn 即用
        // 新文案。action 为触发词(`reload`,预留聚焦切换的层名)。
        PromptSwitched(ps) => UiEventKind::Notice(format!(
            "🔄 提示词已重扫({})· 生效层: {}",
            ps.action,
            format_prompt_layers(&ps.active_layers),
        )),
    };
    Some(UiEvent { kind })
}

/// 提示词层命中列表的人读形式:空列表 = 全部走内嵌默认层。
fn format_prompt_layers(layers: &[String]) -> String {
    if layers.is_empty() {
        "(内嵌默认)".to_string()
    } else {
        layers.join(", ")
    }
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

    #[test]
    fn format_window_secs_is_human_readable() {
        assert_eq!(format_window_secs(0), "已重置");
        assert_eq!(format_window_secs(45), "45s");
        assert_eq!(format_window_secs(605), "10m 5s");
        assert_eq!(format_window_secs(3725), "1h 2m 5s");
    }

    #[test]
    fn compact_strategy_label_covers_all_variants() {
        use reflect_protocol::ContextCompactedStrategy as S;
        assert_eq!(compact_strategy_label(&S::Noop), "no-op");
        assert_eq!(compact_strategy_label(&S::Microcompact), "microcompact");
        assert_eq!(compact_strategy_label(&S::SmartPrune), "smart prune");
        assert_eq!(compact_strategy_label(&S::LlMSummarize), "LLM 摘要");
    }

    #[test]
    fn context_compacted_becomes_visible_notice() {
        // 回归:ContextCompacted 曾被整条丢弃 —— 用户看到的是「上下文突然变短」
        // /「模型突然不记得前面的话」,界面上没有任何解释。
        let ev = reflect_protocol::Event::new(
            "t",
            reflect_protocol::event_msg::EventMsg::ContextCompacted(
                reflect_protocol::ContextCompactedEvent {
                    strategy: reflect_protocol::ContextCompactedStrategy::SmartPrune,
                    removed_messages: 7,
                    before_tokens: 90_000,
                    after_tokens: 20_000,
                },
            ),
        );
        let out = convert_event(ev).expect("应转换为 UiEvent");
        let text = match &out.kind {
            UiEventKind::Notice(t) => t.clone(),
            other => panic!("期望 Notice,得到 {other:?}"),
        };
        assert!(text.contains("smart prune"), "{text}");
        assert!(text.contains("7"), "{text}");
        assert!(text.contains("90000"), "{text}");
        assert!(text.contains("20000"), "{text}");
    }

    #[test]
    fn quota_exhausted_becomes_visible_notice() {
        // 同上:配额耗尽被丢弃时,回合只会莫名停住,用户拿不到任何可决策信息。
        let ev = reflect_protocol::Event::new(
            "t",
            reflect_protocol::event_msg::EventMsg::QuotaExhausted(
                reflect_protocol::QuotaExhaustedEvent {
                    provider: "anthropic".into(),
                    label: "plan-a".into(),
                    used_tokens: 1_000_000,
                    max_tokens: 1_000_000,
                    window_ends_secs: 3_600,
                },
            ),
        );
        let out = convert_event(ev).expect("应转换为 UiEvent");
        let text = match &out.kind {
            UiEventKind::Notice(t) => t.clone(),
            other => panic!("期望 Notice,得到 {other:?}"),
        };
        assert!(text.contains("anthropic"), "{text}");
        assert!(text.contains("plan-a"), "{text}");
        assert!(text.contains("1h 0m 0s"), "{text}");
    }

    #[test]
    fn prompt_list_becomes_visible_notice() {
        // v1.6.1:`/prompt list` 的应答必须可见 —— 可用层 + 生效层是用户
        // 编辑 `.reflect/prompts/*.md` 时唯一的事实来源。
        let ev = reflect_protocol::Event::new(
            "t",
            reflect_protocol::event_msg::EventMsg::PromptList(reflect_protocol::PromptListEvent {
                available: vec!["default".into(), "claude".into()],
                current_spec: "anthropic/claude-sonnet-5-5".into(),
                active_layers: vec!["claude".into()],
            }),
        );
        let out = convert_event(ev).expect("应转换为 UiEvent");
        let text = match &out.kind {
            UiEventKind::Notice(t) => t.clone(),
            other => panic!("期望 Notice,得到 {other:?}"),
        };
        assert!(text.contains("claude-sonnet-5-5"), "{text}");
        assert!(text.contains("claude"), "{text}");
        assert!(text.contains("default"), "{text}");
    }

    #[test]
    fn prompt_list_empty_active_layers_means_embedded_default() {
        let ev = reflect_protocol::Event::new(
            "t",
            reflect_protocol::event_msg::EventMsg::PromptList(reflect_protocol::PromptListEvent {
                available: vec!["default".into()],
                current_spec: "openai/gpt-5.6".into(),
                active_layers: vec![],
            }),
        );
        let out = convert_event(ev).unwrap();
        let text = match &out.kind {
            UiEventKind::Notice(t) => t.clone(),
            other => panic!("期望 Notice,得到 {other:?}"),
        };
        assert!(text.contains("(内嵌默认)"), "{text}");
    }

    #[test]
    fn prompt_switched_becomes_visible_notice() {
        // v1.6.1:`/prompt reload` 的应答 —— 用户需要确认重扫已生效。
        let ev = reflect_protocol::Event::new(
            "t",
            reflect_protocol::event_msg::EventMsg::PromptSwitched(
                reflect_protocol::PromptSwitchedEvent {
                    action: "reload".into(),
                    active_layers: vec!["claude".into()],
                },
            ),
        );
        let out = convert_event(ev).expect("应转换为 UiEvent");
        let text = match &out.kind {
            UiEventKind::Notice(t) => t.clone(),
            other => panic!("期望 Notice,得到 {other:?}"),
        };
        assert!(text.contains("reload"), "{text}");
        assert!(text.contains("claude"), "{text}");
    }

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
        assert_eq!(
            diff.as_bytes()[500],
            0xAD,
            "前置条件:byte 500 是「中」的尾字节(码点内部)"
        );

        let out = format_content_blocks(&[diff_block(&diff)]);
        assert!(out.ends_with('…'), "截断后应以省略号结尾");
        // 截断点必须回退到 498(最后一个完整 ASCII 字符),保留「a…」而非半个字符。
        assert!(out.ends_with("a…"), "截断应在字符边界,实际结尾:{:?}", out);
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

    // ── v1.x 问答事件转换(agent 阻塞等待,必须转交互事件而非 Notice) ──

    fn sample_question(multi: bool) -> reflect_protocol::Question {
        reflect_protocol::Question::new(
            "Approach",
            "Use which approach?",
            vec![
                reflect_protocol::QuestionOption {
                    label: "Fast".into(),
                    description: "quick but rough".into(),
                    preview: None,
                },
                reflect_protocol::QuestionOption {
                    label: "Thorough".into(),
                    description: "slow but solid".into(),
                    preview: None,
                },
            ],
            multi,
        )
        .unwrap()
    }

    #[test]
    fn convert_ask_user_question_routes_to_panel_event() {
        use reflect_protocol::event_msg::{AskUserQuestionEvent, EventMsg};
        let ev = reflect_protocol::Event::new(
            "id-q",
            EventMsg::AskUserQuestion(AskUserQuestionEvent::new(
                "req-1",
                vec![sample_question(false)],
            )),
        );
        let ui = convert_event(ev).expect("AskUserQuestion 不应被丢弃");
        match ui.kind {
            UiEventKind::QuestionAsked {
                request_id,
                questions,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(questions.len(), 1);
                assert_eq!(questions[0].options.len(), 2);
            }
            other => panic!("expected QuestionAsked, got {other:?}"),
        }
    }

    #[test]
    fn convert_ask_user_input_routes_to_composer_capture_event() {
        use reflect_protocol::event_msg::AskUserInputEvent;
        use reflect_protocol::event_msg::EventMsg;
        let ev = reflect_protocol::Event::new(
            "id-ai",
            EventMsg::AskUserInput(AskUserInputEvent {
                request_id: "req-2".into(),
                prompt: "paste your token".into(),
                secret: Some(true),
                placeholder: None,
            }),
        );
        let ui = convert_event(ev).expect("AskUserInput 不应被丢弃");
        match ui.kind {
            UiEventKind::AskUserRequested {
                request_id,
                prompt,
                secret,
            } => {
                assert_eq!(request_id, "req-2");
                assert_eq!(prompt, "paste your token");
                assert!(secret, "secret 缺省按 false,显式 Some(true) 应保留");
            }
            other => panic!("expected AskUserRequested, got {other:?}"),
        }
    }
}
