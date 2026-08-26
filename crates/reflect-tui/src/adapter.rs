use crate::events::{PendingApproval, UiState};
use crate::history_render;
use crate::tui_core::custom_terminal::Terminal as ReflectTerminal;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::insert_history::insert_history_lines;
use ratatui::backend::Backend;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::io::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum UiEventKind {
    /// 回合开始。携带 `turn_id`,供 fork overlay 建立 user prompt → turn 映射。
    TurnStarted {
        turn_id: reflect_protocol::TurnId,
    },
    TurnCompleted,
    AgentDelta(String),
    AgentMessage(String),
    Thinking(String),
    ToolStarted {
        call_id: String,
        name: String,
    },
    ToolCompleted {
        call_id: String,
        output: String,
        error: bool,
        elapsed_ms: u64,
        /// 工具产出里的 unified_diff 原文(若有,供 /diff 全屏视图)。
        diff: Option<String>,
    },
    Error(String),
    /// v1.x Plan mode：plan markdown 已生成，等待用户审批。
    /// 调用方据此打开 plan approval 弹窗（携带 `plan_id`）。
    PlanReady {
        plan_id: String,
        markdown: String,
        /// plan markdown 落盘路径（`<workspace>/.reflect/plan/<plan_id>.md`）；
        /// core 写盘失败时为 `None`。
        path: Option<std::path::PathBuf>,
    },
    /// v1.x Plan mode 草稿预览:agent 写盘到 `.reflect/plan/<draft_id>.md`
    /// 后即时推到对话流。**不**触发 approval modal —— 用户做 1/2/3 决策
    /// 仍走 `PlanReady`。多次写盘触发多次事件,可覆盖式刷新。
    PlanDraftUpdated {
        /// 草稿源文件名(不含扩展名),用于辨识同一草稿的迭代。
        draft_id: String,
        markdown: String,
        path: Option<std::path::PathBuf>,
    },
    /// v1.x Plan mode：进入 plan 模式的确认请求(`/plan <task>` 或 LLM
    /// `EnterPlanModeTool`)。调用方据此打开进入确认条;用户 `1/3/Esc` 后
    /// 回送 `Op::PlanApproval { id, choice }` 解锁 core 的 enter waiter。
    PlanRequest {
        plan_id: String,
        task: String,
    },
    /// v1.x：用户 approve plan 后 runtime 回流。
    PlanApproved {
        plan_id: String,
    },
    /// v1.x：用户 reject plan 后 runtime 回流（携带可选反馈 reason）。
    PlanRejected {
        plan_id: String,
        reason: Option<String>,
    },
    /// v1.x：`PermissionMode` 切换通知（`/mode`、Shift+Tab、plan 批准后）。
    PermissionModeChanged {
        from: reflect_protocol::PermissionMode,
        to: reflect_protocol::PermissionMode,
    },
    Notice(String),
    /// 工具/Hook 审批请求。`kind` 保留原始 `ApprovalKind`，
    /// 回执时据此选 `Op::ToolApproval` / `Op::HookApproval`。
    ApprovalNeeded {
        id: String,
        summary: String,
        kind: reflect_protocol::ApprovalKind,
    },
    /// 上下文窗口配置(SessionConfigured 携带的 context_window_size)。
    ContextWindowConfigured {
        window_size: Option<u32>,
        /// 模型名(来自 SessionConfiguredEvent,供状态栏显示)。
        model: Option<String>,
    },
    /// 最近一次 token 用量(TokenCount 事件,供 HUD context bar)。
    TokenCount {
        input_tokens: u32,
        cached_tokens: u32,
        /// 最近一次调用成本(USD),若有。
        cost_usd: Option<f64>,
    },
    /// 计划步骤进度(PlanStep 事件,供 /tasks overlay)。
    PlanStep {
        index: usize,
        total: usize,
        status: crate::events::PlanStepStatus,
        title: Option<String>,
    },
    /// 子 agent 协作开始(CollabStarted,供 /tasks overlay agent 面板)。
    CollabStarted {
        id: String,
        participants: Vec<String>,
    },
    /// 子 agent 协作结束(CollabFinished,标记对应 agent Done)。
    CollabFinished {
        id: String,
        outcome: String,
        rounds: u32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiEvent {
    pub kind: UiEventKind,
}

#[derive(Debug, Clone)]
pub enum UiHistoryItem {
    User(String),
    Agent(String),
    /// 已**逐行流式进 scrollback** 的 agent 文本片段。
    ///
    /// 流式接线后，agent 文字不再作为一条整段 `Agent` 落滚屏（那样会把所有文字排到
    /// 所有 tool 之后、且与流式行重复）；改为逐行流进滚屏的同时，把每段原文以本变体
    /// 记入 `history`，**仅供 transcript / `/copy` 等 overlay 取用**。`history_tail`
    /// 的滚屏渲染会跳过它（行已流式插入），避免双份。
    StreamedAgent(String),
    /// 已**整段 flush 进 scrollback** 的 reasoning（LLM 思考）片段。
    ///
    /// reasoning 流式累积到 `state.live_thinking`，在边界（agent 正文 / 工具调用 /
    /// 回合结束）整段渲染成 thinking 行经 `apply_event` 返回值写进 scrollback；
    /// 同时把原文以本变体记入 `history`，**仅供 transcript overlay 取用**。
    /// `history_tail` 的滚屏渲染会跳过它（行已通过返回值插入），避免双份。
    StreamedThinking(String),
    Thinking(String),
    /// 工具调用:首行 `✓/✗ name (Nms)` + 截断输出(见 ToolCallCell)。
    ToolCall {
        name: String,
        output: String,
        error: bool,
        elapsed_ms: u64,
        /// 工具产出里的 unified_diff 原文(若有,供 /diff 全屏视图)。
        diff: Option<String>,
    },
    /// 回合分隔:`─ ─ ─`(有实际工作时附 `Worked for Nm`)。
    Separator {
        elapsed_secs: Option<u64>,
        had_work: bool,
    },
    Plan(String),
    Error(String),
    Notice(String),
    /// 启动 Logo / 欢迎横幅(双轴渐变 REFLECT 字形 + 版本/提示语)。
    /// 仅在 `run_async` 启动时 push 一条,跟随 scrollback 自然滚出。
    Banner(Vec<Line<'static>>),
}

/// 按时间顺序收集 agent 回复:整段 `Agent` 与流式 `StreamedAgent` 片段都算。
///
/// 流式接线后,agent 正文以 `StreamedAgent` 片段入 history(整段 `Agent` 只出现在
/// 非流式场景),且同一条回复常被边界 flush 拆成多个**连续的 StreamedAgent** 片段
/// —— 连续的流式片段合并为一条回复,中间隔了 tool/user/notice 等其它条目则视为
/// 新回复;整段 `Agent` 项各自独立(一条即一条回复)。
/// `/copy`、Ctrl+O、copy-history overlay 都从这里取数据;若只匹配 `Agent`,
/// 流式场景(正常路径)会永远报「no agent reply yet」。
pub fn agent_replies_in_order(history: &[UiHistoryItem]) -> Vec<String> {
    let mut replies: Vec<String> = Vec::new();
    // 上一条是否为 StreamedAgent(只有「流式片段 + 流式片段」才合并)。
    let mut prev_was_streamed = false;
    for item in history {
        let (text, is_streamed): (Option<&String>, bool) = match item {
            UiHistoryItem::Agent(t) => (Some(t), false),
            UiHistoryItem::StreamedAgent(t) => (Some(t), true),
            _ => (None, false),
        };
        let Some(text) = text else {
            prev_was_streamed = false;
            continue;
        };
        if prev_was_streamed && is_streamed {
            replies
                .last_mut()
                .expect("prev_was_streamed 为 true 时必有上一条")
                .push_str(text);
        } else {
            replies.push(text.clone());
        }
        prev_was_streamed = is_streamed;
    }
    replies
}

/// 应用 UI 事件。
///
/// 流式引擎接线：`AgentDelta` 一方面 push 进 `StreamController`（保持与 Reflect
/// 同源的 markdown 解析 / 表格 holdback），另一方面**立即把已稳定行 drain 成
/// 行返回**——主循环据此逐行写进 scrollback，文字边到边流式进入滚屏；live 区
/// 只剩可变 tail（见 `draw`）。这样文字与工具调用在同一条 scrollback 流里**按
/// 真实发生顺序交错**，不再割裂成「上半工具 / 下半文字」两块。
///
/// 工具调用按 call_id 去重写入 history（begin 记名、end 落定，只写一条），并在
/// begin 前做一次边界 flush，确保 tool 前的文字已落 scrollback；TurnCompleted
/// 在本回合有工具调用时追加一条 Separator。返回值 `extra` 即本事件让出的待写行。
pub fn apply_event(state: &mut UiState, event: UiEvent) -> Vec<Line<'static>> {
    // 边界 flush 累积的「已稳定」文字行：在 tool 调用前 / 回合结束时 drain 出来，
    // 直接返回给主循环的 pending_history_lines，让文字逐行流进 scrollback，
    // 与工具调用按真实发生顺序交错（修掉「文字 / 工具割裂成两块」的体验问题）。
    let mut extra: Vec<Line<'static>> = Vec::new();
    match event.kind {
        UiEventKind::TurnStarted { turn_id } => {
            state.busy = true;
            state.clear_stream();
            state.turn_started_at = Some(std::time::Instant::now());
            state.had_tool_this_turn = false;
            state.seen_tool_calls.clear();
            state.segment_start = 0;
            // 新回合开始，清除上回合残留的审批 banner，避免跨回合 stale state。
            state.pending_approval = None;
            // v1.x fork 接线:记录本回合 turn_id。user prompt 在 emit_op 前
            // 已 push 进 history,TurnStarted 回流时二者配对,追加进
            // user_prompt_turns(与 fork_rewind overlay 的 user_prompt_indices
            // 同序,供 fork_with_history 的 up_to_turn_id 截断)。
            state.current_turn_id = Some(turn_id);
            state.user_prompt_turns.push(turn_id);
        }
        UiEventKind::TurnCompleted => {
            state.busy = false;
            // ── reasoning flush：回合结束时若仍有累积的思考，整段落进 scrollback。
            // 通常 reasoning 在 AgentDelta/ToolStarted 边界已 flush，此处仅兜底。
            extra.extend(flush_thinking(state));
            // ── 流式引擎终结：把整段未落 scrollback 的文字（稳定行 + 可变 tail）
            // 全部渲染成行返回。不再把整段塞回 history 作为一条 Agent，避免
            // 「所有文字排在所有 tool 之后」的非顺序结果 + 与流式行双份。
            if let Some(stream) = state.stream.as_mut() {
                let (lines, _source) = stream.finalize_to_lines(state.last_stream_width);
                extra.extend(lines);
                // finalize 已把剩余行交付滚屏；记录最后一段原文（自上个边界起）为
                // StreamedAgent，供 overlay。finalize 后整段 source 即 state.live。
                push_segment(state, state.segment_start, state.live.len());
            } else if !state.live.is_empty() {
                // 无 stream（罕见，如旧路径）：整段 live 作为最后一段 StreamedAgent。
                push_segment(state, state.segment_start, state.live.len());
            }
            state.clear_stream();
            // 回合分隔符：仅在本回合做过实际工具调用时画，避免纯对话回合空分隔。
            if state.had_tool_this_turn {
                let elapsed_secs = state
                    .turn_started_at
                    .map(|t| t.elapsed().as_secs())
                    .filter(|s| *s > 0);
                state.history.push(UiHistoryItem::Separator {
                    elapsed_secs,
                    had_work: true,
                });
            }
            state.turn_started_at = None;
            state.had_tool_this_turn = false;
            state.seen_tool_calls.clear();
            // 回合结束，清除残留的审批 banner，避免显示已过时的审批提示。
            state.pending_approval = None;
        }
        UiEventKind::AgentDelta(delta) => {
            // ── reasoning → agent 正文边界：先 flush 累积的思考成一条消息，
            // 再处理 agent delta，保证 thinking 行先于 agent 文字进 scrollback。
            extra.extend(flush_thinking(state));
            state.live.push_str(&delta);
            // 用最近一次记录的内容宽度建流（主循环宽度变化时会 sync_stream_width）。
            state.ensure_stream(state.last_stream_width.max(2) + 2);
            if let Some(stream) = state.stream.as_mut() {
                let _ = stream.push(&delta);
                // ── 逐行流式：把本次 delta 让出的「已稳定行」立刻 drain 进 scrollback。
                // 这样文字边到边显示在滚屏，而非整段堆在 live 区；live 区只剩可变 tail。
                extra.extend(stream.drain_committed_to_lines(state.last_stream_width));
                state.committed_live_len = stream.committed_source_len();
            }
        }
        UiEventKind::AgentMessage(text) => {
            // 完整最终消息：先把流里已稳定的行刷进 scrollback，再落一条完整消息。
            // 非流式场景下 stream 多半为空，flush_committed 返回空数组。
            // reasoning 边界：若有累积思考，先 flush 成一条消息。
            extra.extend(flush_thinking(state));
            extra.extend(flush_committed(state));
            state.clear_stream();
            state.history.push(UiHistoryItem::Agent(text));
        }
        UiEventKind::Thinking(text) => state.live_thinking.push_str(&text),
        UiEventKind::ToolStarted { call_id, name } => {
            // ── 边界 flush：工具调用开始前，把已流式稳定的文字先落 scrollback。
            // 这样 text1 → toolA → text2 在滚屏里天然按序交错，不再割裂成两块。
            // reasoning 边界：若有累积思考，先 flush 成一条消息，再 flush agent 文字。
            extra.extend(flush_thinking(state));
            extra.extend(flush_committed(state));
            state.had_tool_this_turn = true;
            // 记 call_id→name:end 事件只带 call_id，靠这里补名。
            state.tool_names.insert(call_id.clone(), name);
            state.seen_tool_calls.insert(call_id);
        }
        UiEventKind::ToolCompleted {
            call_id,
            output,
            error,
            elapsed_ms,
            diff,
        } => {
            state.had_tool_this_turn = true;
            // 优先用 begin 记下的 tool_name；取不到就退化为 call_id。
            let name = state
                .tool_names
                .remove(&call_id)
                .unwrap_or_else(|| call_id.clone());
            state.seen_tool_calls.insert(call_id.clone());
            state.history.push(UiHistoryItem::ToolCall {
                name,
                output,
                error,
                elapsed_ms,
                diff,
            });
        }
        UiEventKind::Error(text) => state.history.push(UiHistoryItem::Error(text)),
        UiEventKind::PlanReady {
            plan_id,
            markdown,
            path,
        } => {
            // PlanReady 不能重复 push:plan 正文已由 PlanDraftUpdated 进入对话流。
            // 用 upsert 就地更新最近一条 Plan(若最后一条恰是 Plan),否则追加。
            upsert_plan_history(state, markdown.clone());
            // 任务摘要：优先沿用之前 PlanRequest 留下的 task；取不到就退化为 plan 首行。
            let task = markdown
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("(plan)")
                .trim()
                .to_string();
            state.plan_approval = Some(crate::events::PlanApprovalState::new(
                plan_id,
                task,
                markdown,
                path,
            ));
        }
        UiEventKind::PlanRequest { plan_id, task } => {
            // 进入 plan 模式的确认条:记录 task 供 PlanReady 摘要复用,
            // 并打开 plan_enter_request 让用户 1/3/Esc 确认。同时给一条
            // notice 让 scrollback 也有记录(overlay 关闭后仍可回看)。
            state.history.push(UiHistoryItem::Notice(format!(
                "Plan request: {task}"
            )));
            state.plan_enter_request = Some(crate::events::PlanEnterRequestState {
                plan_id,
                task,
            });
        }
        UiEventKind::PlanApproved { plan_id: _ } => {
            state.plan_approval = None;
            state.history.push(UiHistoryItem::Notice(
                "✓ Plan approved — switching to execution mode.".into(),
            ));
        }
        UiEventKind::PlanRejected { plan_id: _, reason } => {
            state.plan_approval = None;
            // 进入确认条也可能因 Revise 被回退 —— 同步清掉,避免残留。
            state.plan_enter_request = None;
            let msg = match reason {
                Some(r) if !r.trim().is_empty() => {
                    format!("↻ Plan needs revision: {r}")
                }
                _ => "↻ Plan revised — staying in plan mode.".to_string(),
            };
            state.history.push(UiHistoryItem::Notice(msg));
        }
        // v1.x Plan mode 草稿预览:agent 写盘 plan markdown 后即时推到对话流,
        // **不**触发 approval modal。多次事件即多次覆盖式刷新。
        // 复用 UiHistoryItem::Plan 让 ProposedPlanCell 渲染(与 PlanReady 一致),
        // 但通过 Notice 前缀标识"草稿"以区分最终审批版。
        UiEventKind::PlanDraftUpdated { markdown, path: _, draft_id: _ } => {
            // 实时草稿预览:每次写盘就地更新最近一条 Plan,避免堆积重复条目。
            upsert_plan_history(state, markdown);
        }
        UiEventKind::PermissionModeChanged { from: _, to } => {
            state.permission_mode = to;
            // 进入 plan 模式后,plan_enter_request 的使命已完成,清掉确认条。
            state.plan_enter_request = None;
            // 模式变化已由 statusline 的 render_identity_line 实时显示，
            // 不再向聊天记录推送 "Mode: X → Y" 通知，避免消息堆积。
        }
        UiEventKind::Notice(text) => state.history.push(UiHistoryItem::Notice(text)),
        UiEventKind::ApprovalNeeded { id, summary, kind } => {
            state.pending_approval = Some(PendingApproval { id, summary, kind });
        }
        UiEventKind::ContextWindowConfigured { window_size, model } => {
            state.context_usage.window_size = window_size;
            // 从 SessionConfigured 提取的模型名优先于环境变量。
            if let Some(m) = model {
                state.status_model = Some(m);
            }
        }
        UiEventKind::TokenCount {
            input_tokens,
            cached_tokens,
            cost_usd,
        } => {
            state.context_usage.last_input_tokens = Some(input_tokens);
            state.context_usage.last_cached_tokens = Some(cached_tokens);
            // 累加 cost:把最近一次调用成本叠加到总成本。
            if let Some(cost) = cost_usd {
                let total = state.context_usage.total_cost_usd.unwrap_or(0.0) + cost;
                state.context_usage.total_cost_usd = Some(total);
            }
        }
        UiEventKind::PlanStep {
            index,
            total,
            status,
            title,
        } => {
            // 按 index 就地更新或追加;total 仅用于裁剪过长(通常不裁)。
            let title = title.unwrap_or_else(|| format!("step {}", index + 1));
            let _ = total;
            if let Some(slot) = state.plan_steps.iter_mut().find(|s| s.index == index) {
                slot.status = status;
                slot.title = title;
            } else {
                state.plan_steps.push(crate::events::PlanStep {
                    index,
                    status,
                    title,
                });
                state.plan_steps.sort_by_key(|s| s.index);
            }
        }
        UiEventKind::CollabStarted { id, participants } => {
            // 每个参与者记一个 Running agent(id 用 collab id + 名)。
            for p in &participants {
                let aid = format!("{}::{}", id, p);
                if let Some(a) = state.agents.iter_mut().find(|a| a.id == aid) {
                    a.status = crate::events::AgentStatus::Running;
                } else {
                    state.agents.push(crate::events::AgentEntry {
                        id: aid,
                        status: crate::events::AgentStatus::Running,
                        label: p.clone(),
                    });
                }
            }
        }
        UiEventKind::CollabFinished { id, outcome, .. } => {
            let _ = outcome;
            // 把该 collab 的所有 agent 标 Done。
            for a in state.agents.iter_mut() {
                if a.id.starts_with(&format!("{id}::")) {
                    a.status = crate::events::AgentStatus::Done;
                }
            }
        }
    }
    extra
}

/// 把 plan markdown upsert 进对话流：若历史中已存在 `Plan` 条目（来自
/// 草稿写盘或早先的 PlanReady），则就地覆盖**最近一条**，避免同一 plan
/// 在 scrollback 中重复显示；否则（LLM 跳过 PlanWrite 直接 ExitPlanMode）
/// 追加新条目兜底。
///
/// 空 markdown 直接忽略，不产生空条目。
fn upsert_plan_history(state: &mut UiState, markdown: String) {
    if markdown.trim().is_empty() {
        return;
    }
    if let Some(slot) = state
        .history
        .iter_mut()
        .rev()
        .find(|h| matches!(h, UiHistoryItem::Plan(_)))
    {
        *slot = UiHistoryItem::Plan(markdown);
    } else {
        state.history.push(UiHistoryItem::Plan(markdown));
    }
}

/// 在边界（agent 正文 / 工具调用 / 回合结束）flush 累积的 reasoning。
///
/// 把 `state.live_thinking` 整段渲染成 thinking 行返回（供主循环写进 scrollback），
/// 同时把原文记成一条 `StreamedThinking` 入 `history` 供 transcript overlay 取用。
/// `history_tail` 渲染滚屏时跳过 `StreamedThinking`（行已通过返回值插入），避免双份。
///
/// 与 `flush_committed` 对称：前者处理 reasoning 的整段 flush，后者处理 agent 文字
/// 的逐行 drain。两者在同一个边界事件里按 `flush_thinking → flush_committed` 顺序
/// 调用，保证 thinking 行先于 agent 文字进 scrollback。
///
/// `show_thinking == false` 时（`/think` 关闭思考显示），不渲染行也不入 history，
/// 仅清空 buffer —— 修复 `/think` 命令长期只切字段不生效的问题。
fn flush_thinking(state: &mut UiState) -> Vec<Line<'static>> {
    if state.live_thinking.is_empty() {
        return Vec::new();
    }
    let text = std::mem::take(&mut state.live_thinking);
    if !state.show_thinking {
        return Vec::new();
    }
    state
        .history
        .push(UiHistoryItem::StreamedThinking(text.clone()));
    let cell = crate::history_render::cells::thinking_cell(text);
    cell.display_lines(state.last_stream_width)
}

/// 在 tool 调用边界 flush 流式：把已稳定行 drain 成 `Vec<Line>`（返回给主循环写进
/// scrollback），并**把上一个边界以来累积的原文记成一条 `StreamedAgent`** 入
/// `history`，供 transcript / `/copy` 等 overlay 取用，再把片段起点推进到当前提交边界。
///
/// 滚屏里文字逐行插入、与工具按真实顺序交错；overlay 拿到的是按 tool 切片的完整原文。
/// `history_tail` 渲染滚屏时跳过 `StreamedAgent`（行已流式插入），避免双份。
/// 无 stream 或本段无内容时返回空数组、不 push。
fn flush_committed(state: &mut UiState) -> Vec<Line<'static>> {
    let start = state.segment_start;
    if let Some(stream) = state.stream.as_mut() {
        let lines = stream.drain_committed_to_lines(state.last_stream_width);
        let new_committed = stream.committed_source_len();
        state.committed_live_len = new_committed;
        // 记录本段（上一个边界 → 当前提交边界）原文为 StreamedAgent。
        push_segment(state, start, new_committed);
        lines
    } else {
        // 无 stream：仍尝试把直接赋值的 live 整段记一条（兼容旧路径）。
        push_segment(state, start, state.live.len());
        Vec::new()
    }
}

/// 把 `state.live[start..end]` 作为一条 `StreamedAgent` 入 history（非空才 push），
/// 并把 `segment_start` 推进到 `end`。
fn push_segment(state: &mut UiState, start: usize, end: usize) {
    if end <= start {
        state.segment_start = end;
        return;
    }
    if let Some(seg) = state.live.get(start..end)
        && !seg.trim().is_empty()
    {
        state
            .history
            .push(UiHistoryItem::StreamedAgent(seg.to_string()));
    }
    state.segment_start = end;
}

pub fn render_history_item(item: &UiHistoryItem, width: u16) -> Vec<Line<'static>> {
    history_render::render_history_item(item, width)
}

/// 将额外行写入 scrollback（预留 stream stable 扩展点）。
pub fn flush_stream_lines<B: Backend + Write>(
    terminal: &mut ReflectTerminal<B>,
    lines: Vec<Line<'static>>,
) -> std::io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    insert_history_lines(terminal, lines)
}

/// 据审批 `kind` 生成内容行预览文本（用于 `approval_block` 的第二行）。
///
/// - 工具 → `Tool: {name}({args 简写})`
/// - 钩子 → `Hook: {name}: {decision_preview}`
/// - 计划 → `Plan: {summary}`
/// `args` 可能是任意 JSON,这里用 `truncate` 截断到 80 字符避免撑爆边框。
fn approval_content_preview(pending: &PendingApproval) -> String {
    match &pending.kind {
        reflect_protocol::ApprovalKind::Tool { tool_name, args } => {
            let args_preview = truncate_json_preview(args, 80);
            if args_preview.is_empty() {
                format!("Tool: {tool_name}")
            } else {
                format!("Tool: {tool_name}({args_preview})")
            }
        }
        reflect_protocol::ApprovalKind::Hook {
            hook_name,
            decision_preview,
        } => format!("Hook: {hook_name}: {decision_preview}"),
        reflect_protocol::ApprovalKind::Plan { summary, .. } => {
            format!("Plan: {summary}")
        }
    }
}

/// 把任意 JSON 值压成一行短预览:对象/数组取紧凑序列化,标量取 to_string,
/// 超过 `max` 个**字符**则截断加 `…`。按字符(非字节)切,避免在 UTF-8
/// 边界 panic(对齐本分支 UTF-8 安全主题)。
fn truncate_json_preview(v: &serde_json::Value, max: usize) -> String {
    let s = match v {
        serde_json::Value::String(s) => s.clone(),
        _ => serde_json::to_string(v).unwrap_or_default(),
    };
    if s.chars().count() <= max {
        s
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// 审批模态块（带边框,3 行:标题行 + 内容行 + 按键提示）。
///
/// 对齐老 TUI `widgets/approval_modal.rs` 的视觉风格:`⚠ APPROVAL NEEDED`
/// 黄底粗体标题,白字内容,灰字按键提示(`[Y]es/[N]o/[A]lways`)。
/// 用 `Block` 包裹,与周围内容区明显分离。
pub fn approval_block(pending: &PendingApproval) -> ratatui::widgets::Paragraph<'static> {
    use ratatui::widgets::{Block, Borders};

    let title = Line::from(Span::styled(
        " ⚠ APPROVAL NEEDED ",
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
    ));

    let content = Line::from(Span::styled(
        format!("  {}", approval_content_preview(pending)),
        Style::default().fg(Color::White),
    ));

    let hint = Line::from(vec![
        Span::styled(
            " [Y]es ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("/ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " [N]o ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("/ ", Style::default().fg(Color::DarkGray)),
        Span::styled(" [A]lways allow ", Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    Paragraph::new(vec![title, content, hint]).block(block)
}

/// Plan 审批提示条块（3 行,无边框,渲染在 composer 正上方）。
///
/// plan 展示后,在底部内联提示用户确认:plan 正文已在 scrollback 里完整
/// 展示(`ProposedPlanCell`),这里只用一行标题 + 一行按键说明,不占用
/// 屏宽,避免模态弹窗遮挡正文。
///
/// 行结构:
/// - 行1(黄底粗体):` ⚠ PLAN APPROVAL: <task> `
/// - 行2(灰):` Plan written to <path> `(path 为 None 时退化为“in memory”)
/// - 行3(彩色按键):`[1] Auto  [2] Manual  [3]/[Esc] Revise`
pub fn plan_approval_prompt(
    plan: &crate::events::PlanApprovalState,
) -> ratatui::widgets::Paragraph<'static> {
    let title = Line::from(Span::styled(
        format!(" ⚠ PLAN APPROVAL: {} ", plan.task),
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
    ));

    let source = match plan.path.as_ref() {
        Some(p) => format!(" Plan written to {}", p.display()),
        None => " Plan stored in memory".to_string(),
    };
    let source_line = Line::from(Span::styled(source, Style::default().fg(Color::DarkGray)));

    let hint = Line::from(vec![
        Span::styled(
            " [1] Auto ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("/ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " [2] Manual ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("/ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " [3]/[Esc] Revise ",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    Paragraph::new(vec![title, source_line, hint])
}

/// 审批提示行（向后兼容:旧调用点可能还在用,保留单行版）。
#[allow(dead_code)]
pub fn approval_banner(pending: &PendingApproval) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "⚠ Approval: ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(pending.summary.clone(), Style::default().fg(Color::White)),
        Span::styled(
            "  [y/Y]  [n/N]  [a/A]",
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_banner_contains_summary_and_hints() {
        let pending = PendingApproval {
            id: "test-456".to_string(),
            summary: "Read: file.txt".to_string(),
            kind: reflect_protocol::ApprovalKind::Tool {
                tool_name: "read".to_string(),
                args: serde_json::json!({ "path": "file.txt" }),
            },
        };
        let line = approval_banner(&pending);
        let text: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(
            text.contains("Read: file.txt"),
            "banner should contain summary: {text}"
        );
        assert!(
            text.contains("[Y]es") || text.contains("[y/Y]"),
            "banner should contain yes hint: {text}"
        );
        assert!(
            text.contains("[N]o") || text.contains("[n/N]"),
            "banner should contain no hint: {text}"
        );
    }

    // ── reasoning 流式累积测试 ──────────────────────────────────────────

    /// 辅助：构造 UiEvent。
    fn ev(kind: UiEventKind) -> UiEvent {
        UiEvent { kind }
    }

    /// 辅助：提取 history 中所有 StreamedThinking 的原文。
    fn streamed_thinking_segments(state: &UiState) -> Vec<String> {
        state
            .history
            .iter()
            .filter_map(|h| match h {
                UiHistoryItem::StreamedThinking(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn thinking_delta_accumulates_into_single_cell() {
        let mut state = UiState::default();
        // 三个 reasoning delta 累积，不直接 push history
        apply_event(&mut state, ev(UiEventKind::Thinking("The ".into())));
        apply_event(&mut state, ev(UiEventKind::Thinking("user ".into())));
        apply_event(&mut state, ev(UiEventKind::Thinking("wants".into())));

        // 累积期间 history 不应有任何 Thinking/StreamedThinking
        assert!(
            state
                .history
                .iter()
                .all(|h| !matches!(h, UiHistoryItem::Thinking(_) | UiHistoryItem::StreamedThinking(_))),
            "累积期间不应 push thinking cell"
        );
        assert_eq!(state.live_thinking, "The user wants");

        // TurnCompleted 触发 flush
        apply_event(&mut state, ev(UiEventKind::TurnCompleted));

        // 应恰好一条 StreamedThinking，内容是拼接全文
        let segments = streamed_thinking_segments(&state);
        assert_eq!(segments, vec!["The user wants".to_string()]);
        // buffer 已清空
        assert!(state.live_thinking.is_empty());
    }

    #[test]
    fn thinking_flushes_at_agent_delta_boundary() {
        let mut state = UiState::default();
        apply_event(&mut state, ev(UiEventKind::Thinking("a".into())));
        apply_event(&mut state, ev(UiEventKind::Thinking("b".into())));

        // AgentDelta 触发 thinking flush（返回 thinking 行），再处理 agent delta
        let lines = apply_event(&mut state, ev(UiEventKind::AgentDelta("x".into())));

        // 返回的行应包含 thinking 渲染（… ab）
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref().to_string())
            .collect();
        assert!(
            text.contains("… ab"),
            "thinking 行应在 AgentDelta 返回值中（顺序保证）: {text}"
        );

        // history 应有一条 StreamedThinking("ab")
        assert_eq!(
            streamed_thinking_segments(&state),
            vec!["ab".to_string()]
        );
        // agent delta 已累积到 live
        assert_eq!(state.live, "x");
        // thinking buffer 已清空
        assert!(state.live_thinking.is_empty());
    }

    #[test]
    fn thinking_flushes_at_tool_started_boundary() {
        let mut state = UiState::default();
        // 第一段 reasoning
        apply_event(&mut state, ev(UiEventKind::Thinking("first".into())));
        apply_event(
            &mut state,
            ev(UiEventKind::ToolStarted {
                call_id: "c1".into(),
                name: "Bash".into(),
            }),
        );
        // 第二段 reasoning
        apply_event(&mut state, ev(UiEventKind::Thinking("second".into())));
        apply_event(&mut state, ev(UiEventKind::TurnCompleted));

        // 应两条独立 StreamedThinking（每段独立成条）
        assert_eq!(
            streamed_thinking_segments(&state),
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn show_thinking_false_drops_thinking() {
        let mut state = UiState::default();
        state.show_thinking = false;

        apply_event(&mut state, ev(UiEventKind::Thinking("hidden".into())));
        let lines = apply_event(&mut state, ev(UiEventKind::TurnCompleted));

        // 不 push history
        assert!(
            state
                .history
                .iter()
                .all(|h| !matches!(h, UiHistoryItem::StreamedThinking(_))),
            "show_thinking=false 时不应 push StreamedThinking"
        );
        // 返回空行（flush_thinking 返回空 Vec）
        assert!(lines.is_empty(), "show_thinking=false 时不应返回 thinking 行");
        // buffer 仍清空（即使不渲染也清空，避免下一段混入）
        assert!(state.live_thinking.is_empty());
    }

    // ── v1.x Plan mode 草稿预览 ──────────────────────────────────────────

    #[test]
    fn plan_draft_updated_pushes_plan_to_history() {
        let mut state = UiState::default();

        apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::PlanDraftUpdated {
                    draft_id: "refactor".into(),
                    markdown: "# Plan\n1. step one\n2. step two".into(),
                    path: None,
                },
            },
        );

        // PlanDraftUpdated 应把 plan markdown 推到 history 的 Plan item 里。
        let plan_items: Vec<_> = state
            .history
            .iter()
            .filter_map(|h| match h {
                UiHistoryItem::Plan(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(plan_items.len(), 1, "应有 1 条 Plan history");
        assert!(
            plan_items[0].contains("step one"),
            "plan 内容应包含 step one: {:?}",
            plan_items[0]
        );

        // PlanDraftUpdated **不**应打开 approval modal —— 与 PlanReady 区分。
        assert!(
            state.plan_approval.is_none(),
            "PlanDraftUpdated 不应打开 approval modal"
        );
    }

    #[test]
    fn plan_draft_updated_empty_markdown_pushes_nothing() {
        let mut state = UiState::default();

        apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::PlanDraftUpdated {
                    draft_id: "x".into(),
                    markdown: "  \n  ".into(),
                    path: None,
                },
            },
        );

        // 空白 markdown 不应 push 到 history。
        let plan_items: usize = state
            .history
            .iter()
            .filter(|h| matches!(h, UiHistoryItem::Plan(_)))
            .count();
        assert_eq!(plan_items, 0, "空白 markdown 不应生成 Plan item");
    }

    #[test]
    fn plan_draft_updated_upserts_single_plan_item() {
        let mut state = UiState::default();

        // 第一次草稿
        apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::PlanDraftUpdated {
                    draft_id: "refactor".into(),
                    markdown: "# Plan\n1. step one".into(),
                    path: None,
                },
            },
        );

        // 第二次迭代(覆盖式更新,同 draft_id)
        apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::PlanDraftUpdated {
                    draft_id: "refactor".into(),
                    markdown: "# Plan\n1. step one\n2. step two".into(),
                    path: None,
                },
            },
        );

        // 多次写盘应**就地覆盖**最近一条 Plan,避免同一 plan 在 scrollback 重复。
        let plan_items: usize = state
            .history
            .iter()
            .filter(|h| matches!(h, UiHistoryItem::Plan(_)))
            .count();
        assert_eq!(plan_items, 1, "多次 PlanDraftUpdated 应保持单条 Plan item");
        // 内容应是最新一次写盘的版本。
        let last = state
            .history
            .iter()
            .find(|h| matches!(h, UiHistoryItem::Plan(_)));
        assert_eq!(
            last.map(|h| match h {
                UiHistoryItem::Plan(m) => m.clone(),
                _ => String::new(),
            }),
            Some("# Plan\n1. step one\n2. step two".to_string()),
            "Plan 内容应是最新草稿版"
        );
    }
}
