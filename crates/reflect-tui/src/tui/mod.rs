//! 稳定 Reflect 风格聊天事件循环，连接到 Reflect AgentThread。
//!
//! 在**主屏幕**上镜像 Reflect 的「内联视口」渲染模型：
//!
//! - 已提交历史 → 通过 `insert_history_lines` 进入原生 scrollback
//! - 底部视口 → 流式 + 编辑器 + 状态栏（每帧差分重绘）
//! - 视口扩缩遵循 Reflect `Tui::draw`：`scroll_region_up` + `clear_for_viewport_change`

use crate::TuiArgs;
use crate::adapter::{self, UiEvent, UiEventKind, UiHistoryItem};
use crate::events::{LoopState, UiState};
use crate::history_render::{self, SlashOutcome};
use crate::terminal::{TerminalGuard, is_exit_key, poll_event};
use crate::transcript_pager::TranscriptPager;
use crate::tui_core::custom_terminal::Frame;
use crate::tui_core::history_cell::HistoryCell;
use crate::tui_core::insert_history::insert_history_lines;
use crate::tui_core::markdown_render::render_markdown_text;
use crate::tui_core::public_widgets::composer_input::ComposerAction;
use crate::tui_core::slash_command::SlashCommand;
use crate::tui_core::tui::FrameRequester;
#[cfg(test)]
use crate::viewport::live_region_height;
use crate::viewport::prepare_bottom_viewport;
use crossterm::SynchronizedUpdate;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::size as term_size;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use reflect_core::AgentThread;
use reflect_protocol::{ContentBlock, Submission};
use std::env;
use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc};

pub async fn run_async(args: TuiArgs, thread: Arc<AgentThread>) -> anyhow::Result<()> {
    // fullscreen 合并 --fullscreen 与 REFLECT_TUI_FULLSCREEN 环境变量。
    let fullscreen = args.fullscreen
        || std::env::var("REFLECT_TUI_FULLSCREEN")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
    // fullscreen 为 true 时直接进备用屏(alt-screen),整个屏幕是 ratatui buffer。
    let mut guard = TerminalGuard::enter(fullscreen)?;
    let mut state = UiState::default();

    // FrameRequester：与 Reflect 一致，供 composer 动画/ paste-burst 调度重绘。
    let (draw_tx, mut draw_rx) = broadcast::channel::<()>(16);
    let _frame_requester = FrameRequester::new(draw_tx);

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<UiEvent>();

    // 订阅线程级生命周期事件(SessionConfigured 带 context_window_size,
    // TokenCount 走 TurnHandle 但 SessionConfigured 只走这里)。
    // 转发到统一 event_tx,让 conversion + apply_event 处理。
    {
        let mut session_rx = thread.subscribe_session();
        let tx = event_tx.clone();
        let tasks = Arc::clone(&state.background_tasks);
        let mut s = tasks.lock().unwrap();
        s.spawn(async move {
            while let Some(protocol_event) = session_rx.recv().await {
                if let Some(ui_event) = conversion::convert_event(protocol_event) {
                    if tx.send(ui_event).is_err() {
                        break;
                    }
                }
            }
        });
    }

    // 从 AgentConfig 直接读取模型名(启动期 SessionConfigured 还未发出,
    // 先写入 state.status_model 让第一帧状态栏就显示正确的模型)。
    // SessionConfigured 事件到达后会覆盖此值(保持一致)。
    let status_model = thread.config().current_model();
    let status_model = if status_model.is_empty() {
        env::var("REFLECT_MODEL_DISPLAY")
            .or_else(|_| env::var("OPENAI_MODEL"))
            .or_else(|_| env::var("ANTHROPIC_MODEL"))
            .unwrap_or_else(|_| "reflect".to_string())
    } else {
        status_model
    };
    state.status_model = Some(status_model.clone());
    // v1.x:启动期读当前 PermissionMode(--plan-mode 启动时为 Plan,否则默认 Auto),
    // 驱动 HUD mode 段与 composer 占位符。后续由 PermissionModeChanged 事件回流更新。
    state.permission_mode = thread.config().permission_mode();
    // v1.x fork 接线:从 AgentConfig 读当前 session 的 ThreadId,供 fork overlay
    // 作 fork_with_history 的 parent_id。bootstrap 期已 with_session_id 注入。
    state.session_id = thread.config().session_id;
    sync_plan_mode_visuals(&mut state);
    let status_cwd = env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| ".".to_string());
    // 启动期 best-effort 探测 git 分支(对齐老 TUI `current_git_branch`)。
    let status_git_branch = history_render::statusline::detect_git_branch();

    // 设置终端标题(OSC 0 `ESC]0;titleBEL`):在 iTerm/tmux/Kitty 上显示为标签页标题。
    // 格式: `Reflect · <model> · git:(<branch>)`，非 git 仓库时省略 branch 段。
    let initial_title = if let Some(ref branch) = status_git_branch {
        format!("Reflect · {} · git:({})", status_model, branch)
    } else {
        format!("Reflect · {}", status_model)
    };
    crate::terminal::set_terminal_title(&initial_title);

    // 启动 banner:REFLECT 块字形 + braille 装饰 + 双轴渐变 24-bit,作为 history
    // 首条 push 进去,跟随原生 scrollback 自然滚出,被 Ctrl-T transcript 收录。
    // banner 的 lines 预渲染好,banner_cell 不在 `history_render.rs` 调用链中
    // 二次加工(避免被 marker 改写破坏渐变锚点 / 误裁切)。
    {
        let banner_cell = history_render::cells::banner_cell(env!("CARGO_PKG_VERSION"));
        // 直接读出预渲染的 Line 序列:PlainHistoryCell 的 `lines` 字段是
        // `pub(super)`(在 tui_core::history_cell 模块内),通过其 HistoryCell trait
        // 的 `display_lines` 是 pub 调用点,等价地拿到内容。
        let banner_lines = banner_cell.display_lines(80);
        state.history.push(UiHistoryItem::Banner(banner_lines));
    }

    let mut history_tail: usize = 0;
    let mut is_first = true;
    // pending history 队列：对齐 Reflect flush_pending_history_lines 顺序。
    let mut pending_history_lines: Vec<Line<'static>> = Vec::new();
    // spinner 动画帧计数(每轮循环自增,驱动状态行 braille spinner)。
    let mut tick_frame: u32 = 0;
    // 终端通知后端(OSC9/BEL,启动期按终端探测自动选;失败静默)。
    let mut notifier = crate::tui_core::notifications::detect_backend(
        crate::config_compat::types::NotificationMethod::Auto,
    );

    loop {
        // ── 1. 排空代理事件 ─────────────────────────────────────────────
        // 预取终端尺寸：供 try_recv 循环内即时补齐 history_tail，让文字与
        // 工具调用按真实发生顺序交错（adapter 在 ToolStarted 边界 flush 流式
        // 文字；主循环必须在每个事件后立刻渲染对应 history 项，而非延迟到
        // 全部事件排空后才补齐——否则同一帧内文字永远排在工具之前）。
        let (width, height) = term_size().unwrap_or((80, 24));
        let wrap_width = width.max(2).saturating_sub(2);
        let mut notify_triggers = history_render::notify::NotifyTriggers::new();
        while let Ok(ui_event) = event_rx.try_recv() {
            notify_triggers.observe(&ui_event.kind);
            let extra = adapter::apply_event(&mut state, ui_event);
            pending_history_lines.extend(extra);
            // 即时补齐：本事件刚 push 进 state.history 的项（如 ToolCall）
            // 立刻渲染成行追加到 pending，与前后 AgentDelta 的流式行按真实
            // 时间顺序交错。
            while history_tail < state.history.len() {
                pending_history_lines.extend(history_render::history_item_to_lines(
                    &state.history[history_tail],
                    wrap_width,
                    &state.cwd,
                ));
                history_tail += 1;
            }
        }
        // Plan 审批弹窗 / mode 段可能因事件改变 → 同步 composer 占位符。
        sync_plan_mode_visuals(&mut state);
        // 回合完成 / 需审批 / 出错 → 终端通知(写裸 stdout,与帧缓冲分离)。
        if let Some(msg) = notify_triggers.message() {
            history_render::notify::notify(&mut notifier, &msg);
        }

        if state.composer.is_in_paste_burst() {
            let _ = state.composer.flush_paste_burst_if_due();
        }

        // ── /loop 调度:到点则把 command 当作一次 user input 提交给 agent。 ──
        // 本地状态机:`/loop <secs> <cmd>` 设 next_fire,每帧比较 now;
        // 触发后把 next_fire 推后一个 interval,实现稳态循环。
        // slash 命令(如 `/status`)在此不走 composer,直接当文本提交 ——
        // 这样既能循环触发 slash 本地动作,也能循环发普通 prompt。
        if let Some(ls) = state.loop_state.as_mut() {
            if Instant::now() >= ls.next_fire {
                let command = ls.command.clone();
                let interval = ls.interval_secs;
                ls.next_fire = Instant::now() + Duration::from_secs(interval);
                state
                    .history
                    .push(adapter::UiHistoryItem::User(command.clone()));
                // fire-and-forget:同 emit_op / Review 路径,TUI 退出时 event_tx
                // drop 自然终止。
                let thread = Arc::clone(&thread);
                let tx = event_tx.clone();
                tokio::spawn(async move {
                    let sub = Submission::user_input(command);
                    let mut handle = thread.submit(sub).await;
                    while let Some(protocol_event) = handle.next().await {
                        if let Some(ui_event) = conversion::convert_event(protocol_event) {
                            if tx.send(ui_event).is_err() {
                                break;
                            }
                        }
                    }
                });
            }
        }

        // ── 2. 同步流式宽度 + reflow 兜底 ──────────────────────────────────
        // width/wrap_width 已在阶段 1 预取；终端宽度变化时同步流式控制器，
        // 保持 stable 行行长一致。
        state.sync_stream_width(width);
        // Resize 后 reflow：推倒旧宽度 history，history_tail 重置为 0。
        if state.needs_terminal_reflow {
            history_tail = 0;
            pending_history_lines.clear();
        }
        // 无条件补齐：阶段 1 的逐事件补齐只覆盖「经 event_rx 到达、由
        // apply_event push」的 history 项；本循环捕获所有【非事件流】路径 push
        // 的项——slash 命令（/status /effort /goal 等）在阶段 6 handle_event 里
        // 直接 push 的 Notice、/loop 触发（上方）push 的 User 消息。reflow 时
        // history_tail 已重置为 0，本循环用新宽度重建全部 scrollback；否则仅
        // 追加本帧新出现的尾部项。history_tail 是单调游标（reflow 外只增），
        // 与阶段 1 共享同一游标，故不会重复渲染。
        while history_tail < state.history.len() {
            pending_history_lines.extend(history_render::history_item_to_lines(
                &state.history[history_tail],
                wrap_width,
                &state.cwd,
            ));
            history_tail += 1;
        }

        // ── 3. HUD(老 TUI 双行:identity + metrics;窄屏折叠) ────────────
        tick_frame = tick_frame.wrapping_add(1);
        // 优先用 SessionConfigured 提供的模型名,环境变量 fallback。
        let model_for_hud = state.status_model.as_deref().unwrap_or(&status_model);
        state.status_line = Some(history_render::statusline::render_identity_line(
            state.busy,
            tick_frame,
            model_for_hud,
            &status_cwd,
            status_git_branch.as_deref(),
            state.pending_approval.is_some(),
            &state.context_usage,
            state.vim_enabled,
            state.permission_mode,
            state.plan_enter_request.is_some(),
        ));
        state.metrics_line = Some(history_render::statusline::render_metrics_line(
            &state.history,
        ));

        // ── 4. 叠加层(会话记录 / diff / 任务 / 会话,备屏)──────────────
        if state.transcript.is_some()
            || state.diff_overlay.is_some()
            || state.tasks_overlay.is_some()
            || state.session_overlay.is_some()
        {
            let _ = guard.enter_alt_screen();
            guard.terminal().draw(|frame| {
                if let Some(ref pager) = state.transcript {
                    pager.draw(frame);
                } else if let Some(ref pager) = state.diff_overlay {
                    pager.draw(frame);
                } else if let Some(ref _pager) = state.tasks_overlay {
                    // tasks overlay 需要 state + width;画在闭包里。
                    if let Some(ref pager) = state.tasks_overlay {
                        pager.draw(frame, &state, width);
                    }
                } else if let Some(ref _pager) = state.session_overlay {
                    // session overlay 复用 TasksPager,session_entries 由其渲染。
                    if let Some(ref pager) = state.session_overlay {
                        pager.draw(frame, &state, width);
                    }
                }
            })?;
        } else {
            let _ = guard.leave_alt_screen();

            // ── 5. Reflect 顺序：先 shrink/贴底，再 flush history，再 draw ──
            // 长 live 时若先 insert，上方可写区极窄，正文会被拆进 scrollback；
            // shrink 的 clear/滚行再搅一次，出现「上半句 / 空行 / 下半句」观感。
            //
            // live 区内容：流式进行中优先显示「可变 tail」（已稳定行已逐行流进
            // scrollback），从而 live 区与滚屏不重复、不再堆整段文字；无 stream
            // 时回退到整段 live（兼容旧路径）。
            let (live_text_lines, live_h) = compute_live_region(&mut state, width);

            // 全屏模式(REFLECT_TUI_FULLSCREEN / --fullscreen):整个屏幕是 ratatui
            // buffer,跳过 viewport 扩缩与 scrollback 插入,彻底避免 inline 模型
            // shrink 时向终端原生 scrollback 注入空行。历史一次渲染进全屏区。
            if fullscreen {
                // pending 只服务于 inline 的 scrollback 插入,全屏丢弃。
                pending_history_lines.clear();
                state.needs_terminal_reflow = false;
                guard.terminal().draw(|frame| {
                    draw_fullscreen(frame, &mut state, height, &live_text_lines, live_h)
                })?;
            } else {
                let composer_h = state.composer.desired_height(width);
                // 两种审批条高度不同:plan_approval_prompt 3 行(无边框),
                // approval_block 5 行(3 行内容 + 上下边框)。二者互斥,取其一。
                let approval_h: u16 = if state.pending_approval.is_some() {
                    5
                } else if state.plan_approval.is_some() {
                    3
                } else {
                    0
                };
                // HUD 行数(窄屏防御):>=12→2,8-11→1,<8→0。
                let status_h: u16 = history_render::statusline::hud_height(height);
                let desired = composer_h
                    .saturating_add(live_h)
                    .saturating_add(approval_h)
                    .saturating_add(status_h)
                    .min(height)
                    .max(1);

                // sync_update 防闪烁。顺序：reflow? → prepare → insert_history → draw。
                let draw_result = stdout().sync_update(|_| -> std::io::Result<()> {
                    // Resize 后 reflow:把已按旧宽度折好的 scrollback 推倒,后续
                    // `prepare → insert_history → draw` 在空屏上用新宽度重建,确保
                    // viewport 与 scrollback 视觉宽度对齐(否则永久错位/重叠/重复)。
                    // 注:本闭包之外已完成 `history_tail = 0` + 清空
                    // `pending_history_lines`,让 pending 用新宽度从头生成。
                    if state.needs_terminal_reflow {
                        guard.terminal().clear_scrollback_and_visible_screen_ansi()?;
                        // 屏已空,`set_viewport_area` 内的 width-change reset 已
                        // 让 buffer 与清屏后的实际终端状态一致。
                        state.needs_terminal_reflow = false;
                    }

                    prepare_bottom_viewport(guard.terminal(), width, height, desired)?;

                    if !pending_history_lines.is_empty() {
                        let lines = std::mem::take(&mut pending_history_lines);
                        insert_history_lines(guard.terminal(), lines)?;
                    }

                    guard.terminal().draw(|frame| {
                        draw(frame, &mut state, height, &live_text_lines, live_h)
                    })?;
                    Ok(())
                })?;
                draw_result?;
            }
        }

        if is_first {
            is_first = false;
            continue;
        }

        // ── 6. 等待输入 / agent / frame 请求 ──────────────────────────────
        let (poll_tx, mut poll_rx) = mpsc::channel::<Option<Event>>(1);
        tokio::task::spawn_blocking(move || {
            let result = poll_event(Duration::from_millis(200));
            let _ = poll_tx.blocking_send(result.ok().flatten());
        });

        tokio::select! {
            maybe_event = poll_rx.recv() => {
                if let Some(Some(event)) = maybe_event {
                    if handle_event(
                        event,
                        &mut state,
                        &mut guard,
                        &thread,
                        &event_tx,
                        &mut history_tail,
                        &_frame_requester,
                        fullscreen,
                    )? {
                        break;
                    }
                }
            }
            Some(ui_event) = event_rx.recv() => {
                let mut triggers = history_render::notify::NotifyTriggers::new();
                triggers.observe(&ui_event.kind);
                let extra = adapter::apply_event(&mut state, ui_event);
                pending_history_lines.extend(extra);
                // 即时补齐 history_tail（与 try_recv 批量路径一致），确保
                // 单事件路径下文字与工具调用也按真实顺序交错。
                while history_tail < state.history.len() {
                    pending_history_lines.extend(history_render::history_item_to_lines(
                        &state.history[history_tail],
                        wrap_width,
                        &state.cwd,
                    ));
                    history_tail += 1;
                }
                sync_plan_mode_visuals(&mut state);
                if let Some(msg) = triggers.message() {
                    history_render::notify::notify(&mut notifier, &msg);
                }
            }
            _ = draw_rx.recv() => {
                // FrameRequester 触发的重绘；下一轮循环会 draw。
            }
        }
    }
    // TUI 退出时 abort 所有后台任务，防止事件丢失。
    state.background_tasks.lock().unwrap().abort_all();
    Ok(())
}

/// 如果循环应退出则返回 true。
fn handle_event(
    event: Event,
    state: &mut UiState,
    guard: &mut TerminalGuard,
    thread: &Arc<AgentThread>,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
    history_tail: &mut usize,
    frame_requester: &crate::tui_core::tui::FrameRequester,
    fullscreen: bool,
) -> anyhow::Result<bool> {
    // v1.x Tier 5: checkpoint rewind 确认 modal 独占键位(最高优先级)。
    if state.checkpoint_rewind_sha.is_some() {
        match &event {
            Event::Key(key) => {
                let is_cancel = matches!(
                    key.code,
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc
                ) || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL));
                if is_cancel {
                    state.checkpoint_rewind_sha = None;
                    return Ok(false);
                }
                let confirm = matches!(
                    key.code,
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter
                );
                if confirm {
                    let sha = state.checkpoint_rewind_sha.take().unwrap();
                    let short: String = sha.chars().take(8).collect();
                    // git reset --hard 在稳定 TUI 中通过 emit_op 触发。
                    // 目前作为占位通知,留待后续接 git 工具路径。
                    state.history.push(adapter::UiHistoryItem::Notice(format!(
                        "⚠ rewind to {short}: git reset --hard not yet wired (sha saved)."
                    )));
                }
                return Ok(false);
            }
            _ => {}
        }
    }

    // Overlay(transcript / diff / tasks / session)独占键位。
    let overlay_open = state.transcript.is_some()
        || state.diff_overlay.is_some()
        || state.tasks_overlay.is_some()
        || state.session_overlay.is_some();
    if overlay_open {
        match &event {
            Event::Key(key) if is_exit_key(*key) => return Ok(true),
            Event::Key(key)
                if (key.code == KeyCode::Char('t')
                    && key.modifiers.contains(KeyModifiers::CONTROL))
                    || key.code == KeyCode::Esc =>
            {
                state.transcript = None;
                state.diff_overlay = None;
                state.tasks_overlay = None;
                state.session_overlay = None;
                guard.leave_alt_screen()?;
                return Ok(false);
            }
            Event::Key(key) => {
                let page = 10usize;
                // transcript/diff 用 TranscriptPager;tasks 用 TasksPager。
                if let Some(ref mut pager) = state
                    .transcript
                    .as_mut()
                    .or_else(|| state.diff_overlay.as_mut())
                {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => pager.scroll_up(1),
                        KeyCode::Down | KeyCode::Char('j') => pager.scroll_down(1, page),
                        KeyCode::PageUp => pager.page_up(page),
                        KeyCode::PageDown | KeyCode::Char(' ') => pager.page_down(page),
                        KeyCode::Home => pager.jump_top(),
                        KeyCode::End => pager.jump_bottom(page),
                        _ => {}
                    }
                } else if let Some(ref mut pager) = state
                    .tasks_overlay
                    .as_mut()
                    .or_else(|| state.session_overlay.as_mut())
                {
                    match key.code {
                        KeyCode::Up | KeyCode::Char('k') => pager.page_up(1),
                        KeyCode::Down | KeyCode::Char('j') => pager.page_down(1, usize::MAX),
                        KeyCode::PageUp => pager.page_up(page),
                        KeyCode::PageDown | KeyCode::Char(' ') => pager.page_down(page, usize::MAX),
                        KeyCode::Home => pager.jump_top(),
                        KeyCode::End => pager.jump_bottom(usize::MAX),
                        _ => {}
                    }
                }
                return Ok(false);
            }
            _ => return Ok(false),
        }
    }

    match event {
        Event::Key(key) => {
            // v1.x:Tab 在输入为空时也循环 permission mode(等价 Shift+Tab)。
            // 裸 Tab 不在 `resolve_global_key`(避免抢占 composer 的补全/缩进),
            // 这里在 fallthrough 前单独判定。
            if key.code == KeyCode::Tab
                && key.modifiers == KeyModifiers::NONE
                && state.composer.is_empty()
                && state.plan_approval.is_none()
                && state.pending_approval.is_none()
            {
                emit_op(thread, event_tx, reflect_protocol::Op::CyclePermissionMode);
                return Ok(false);
            }
            // 全局快捷键经 keymap 解析(单一事实来源);未命中 fallthrough 到 composer。
            //
            // v1.x 回归修复:任意 picker/checkpoint overlay 打开时**跳过**
            // `resolve_global_key`,否则 Ctrl+C 会被全局键映射为 `Exit`
            // 直接退出整个 TUI —— 各 picker 的 `handle_key`(在下方)里
            // `Ctrl+C → Close` / `Esc → Close` 分支永远不会被执行。
            // (此前的 overlay_open 守卫只覆盖 transcript/diff/tasks/session
            // 四个分页器,漏掉了 8 个 picker。/traces /plugin /mcp 复用
            // transcript pager,已被上面的 overlay_open 覆盖,无需重复。)
            let any_picker_open = state.checkpoint_overlay.is_some()
                || state.checkpoint_rewind_sha.is_some()
                || state.fork_rewind.is_some()
                || state.copy_history.is_some()
                || state.skills_hub.is_some()
                || state.image_picker.is_some()
                || state.keymap_picker.is_some()
                || state.statusline_picker.is_some()
                || state.theme_picker.is_some();
            // 全屏模式(REFLECT_TUI_FULLSCREEN / --fullscreen):PageUp/PageDown 滚动
            // 历史区(null 掉终端原生 scrollback 滚动后,这是唯一翻历史的方式)。
            // 避开 picker/审批模态打开的场景,把键让给它们内部使用。
            if fullscreen
                && !any_picker_open
                && state.pending_approval.is_none()
                && state.plan_approval.is_none()
            {
                match key.code {
                    KeyCode::PageUp => {
                        state.history_scroll_offset =
                            state.history_scroll_offset.saturating_add(10);
                        return Ok(false);
                    }
                    KeyCode::PageDown => {
                        state.history_scroll_offset =
                            state.history_scroll_offset.saturating_sub(10);
                        return Ok(false);
                    }
                    _ => {}
                }
            }
            // picker 打开时 Ctrl+C 默认只关 picker(见下方各 picker handle_key),
            // 用户必须再按一次才能退出 —— 容易让人误以为「退不掉」。这里实现
            // 双击退出:picker 打开时若 1.5s 内第二次按 Ctrl+C,直接退出 TUI;
            // 否则记下时刻,放行让 picker 关闭它。(第一次按下即便关掉 picker,
            // 第二次按下若 picker 仍开着则退出;若已关,经 keymap 同样退出。)
            if any_picker_open && is_exit_key(key) {
                const QUIT_DOUBLE_PRESS_WINDOW: std::time::Duration =
                    std::time::Duration::from_millis(1500);
                let now = std::time::Instant::now();
                let second_press = state
                    .last_quit_press
                    .map(|t| now.duration_since(t) <= QUIT_DOUBLE_PRESS_WINDOW)
                    .unwrap_or(false);
                if second_press {
                    state.last_quit_press = None;
                    return Ok(true);
                }
                state.last_quit_press = Some(now);
            } else if !any_picker_open {
                // 没有 picker 打开时清除上一次记录,避免「关 picker 后慢吞吞按
                // Ctrl+C」被误判为双击退出(无 picker 时走 keymap 一次退出即可)。
                state.last_quit_press = None;
            }
            if !any_picker_open
                && let Some(action) = crate::keymap::resolve_global_key(event.clone())
            {
                match action {
                    crate::keymap::KeyAction::Exit => return Ok(true),
                    crate::keymap::KeyAction::OpenTranscript => {
                        open_transcript(state);
                        return Ok(false);
                    }
                    crate::keymap::KeyAction::CycleMode => {
                        emit_op(thread, event_tx, reflect_protocol::Op::CyclePermissionMode);
                        return Ok(false);
                    }
                    // v1.x Tier 3:
                    crate::keymap::KeyAction::CopyLastReply => {
                        // 模态打开时让模态先吃键(approval 弹窗有自己的 y/n 逻辑)。
                        if state.pending_approval.is_none() && state.plan_approval.is_none() {
                            copy_last_reply(state);
                        }
                        return Ok(false);
                    }
                    crate::keymap::KeyAction::OpenExternalEditor => {
                        // 编辑器需要 suspend alt-screen + raw mode,模态打开时跳过避免悬挂。
                        if state.pending_approval.is_none() && state.plan_approval.is_none() {
                            open_external_editor(state, guard, frame_requester);
                        }
                        return Ok(false);
                    }
                    crate::keymap::KeyAction::ForceRedraw => {
                        force_redraw(frame_requester);
                        return Ok(false);
                    }
                    crate::keymap::KeyAction::OpenDiff
                    | crate::keymap::KeyAction::ToggleDiffView => {
                        if state.pending_approval.is_none() && state.plan_approval.is_none() {
                            open_diff_overlay(state);
                        }
                        return Ok(false);
                    }
                }
            }
            Ok(handle_key(
                key,
                state,
                guard,
                thread,
                event_tx,
                history_tail,
            )?)
        }
        Event::Paste(text) => {
            let text = text.replace('\r', "\n");
            state.composer.handle_paste(text);
            Ok(false)
        }
        Event::Resize(_, _) => {
            // 终端宽度/高度变化。Viewport buffer 已在 `set_viewport_area`
            // 单独修复里 reset,但已入 scrollback 的历史行按旧宽度折死,
            // 会与新宽度 viewport 重叠/错位/重复,且永远不自动恢复。
            // 标记 reflow:主循环下一帧先清屏 + scrollback,再用新宽度重新
            // 插入 `state.history`,让 scrollback 与 viewport 对齐。
            state.needs_terminal_reflow = true;
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn open_transcript(state: &mut UiState) {
    let width = term_size().map(|(w, _)| w).unwrap_or(80);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for item in &state.history {
        lines.extend(history_render::history_item_to_lines(
            item, width, &state.cwd,
        ));
    }
    if lines.is_empty() {
        lines.push(Line::from("(empty transcript)"));
    }
    state.transcript = Some(TranscriptPager::new(lines));
}

/// v1.x:把一个 `Op` 包装成 `Submission` 发给 agent thread,并在后台 drain 回流事件
/// 转成 `UiEvent` 推回 `event_tx`。复刻 `/compact` 的提交范式,消除重复。
///
/// `Op::EnterPlanMode` / `ExitPlanMode` / `PlanApproval` / `SetPermissionMode` /
/// `CyclePermissionMode` 都走这里;它们的 `EventMsg` 回流(PlanReady /
/// PermissionModeChanged …)由主循环的 drain + apply_event 处理。
///
/// 使用 `tokio::spawn` fire-and-forget 是安全的:当 TUI 退出时，`event_tx` channel
/// 被 drop，所有 spawned tasks 通过 `tx.send().is_err()` 自然退出。TUI 不等待
/// 这些任务完成，因为退出时不需要再处理任何事件。
fn emit_op(
    thread: &Arc<AgentThread>,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
    op: reflect_protocol::Op,
) {
    let thread = Arc::clone(thread);
    let tx = event_tx.clone();
    tokio::spawn(async move {
        let sub = Submission::with_id(uuid::Uuid::new_v4().to_string(), op);
        let mut handle = thread.submit(sub).await;
        while let Some(protocol_event) = handle.next().await {
            if let Some(ui_event) = conversion::convert_event(protocol_event) {
                if tx.send(ui_event).is_err() {
                    break;
                }
            }
        }
    });
}

/// 同步 plan 模式的可见反馈:composer 占位符。
///
/// Plan 模式时占位符改为「describe the task or /exit-plan …」,其余模式还原默认。
/// 边框 / HUD mode 段分别由 composer 渲染层与 `render_identity_line` 处理,这里只管占位符。
/// 放在事件 drain 之后与启动期调用,保证 mode 切换即时反映。
fn sync_plan_mode_visuals(state: &mut UiState) {
    use reflect_protocol::PermissionMode as Pm;
    let placeholder = match state.permission_mode {
        Pm::Plan => "describe the task or /exit-plan to leave plan mode",
        _ => "Send a message… (shift+enter for newline)",
    };
    state.composer.set_placeholder(placeholder.to_string());
}

// ── v1.x Tier 3: 全局热键动作实现 ────────────────────────────────────

/// v1.x Tier 3 (Ctrl+O):从 history 抽出最后一条 Agent 回复,推到剪贴板。
///
/// 失败(无 agent 回复 / 剪贴板不支持)时通过 Notice 告知用户,不静默吞错。
/// 剪贴板写入走 vendored `clipboard_copy::copy_to_clipboard`(OSC 52 + arboard
/// 优先链),与归档 TUI 行为一致。
fn copy_last_reply(state: &mut UiState) {
    let Some(text) = state.history.iter().rev().find_map(|item| match item {
        adapter::UiHistoryItem::Agent(t) => Some(t.clone()),
        _ => None,
    }) else {
        state
            .history
            .push(UiHistoryItem::Notice("Copy: no agent reply yet.".into()));
        return;
    };

    match crate::tui_core::clipboard_copy::copy_to_clipboard(&text) {
        Ok(lease) => {
            // Linux/X11 与部分 Wayland 后端需要持有 ClipboardLease 才能保留剪贴板
            // 内容直到用户粘贴;存到 state,覆盖并 drop 旧 lease。
            state.clipboard_lease = lease;
            let n_chars = text.chars().count();
            state.history.push(UiHistoryItem::Notice(format!(
                "Copied last reply ({} chars) to clipboard.",
                n_chars
            )));
        }
        Err(e) => {
            state
                .history
                .push(UiHistoryItem::Notice(format!("Copy failed: {e}")));
        }
    }
}

/// v1.x Tier 3 (Ctrl+G):打开外部编辑器(由 `$VISUAL` / `$EDITOR` 指定)
/// 加载当前 composer draft,保存退出后把内容回填到 composer。
///
/// 实现要点:
/// - **必须先 suspend alt-screen + raw mode**,否则 editor 启动后 TUI 输出会污染终端。
/// - 通过 `FrameRequester` 强制下一帧重绘,避免 ratatui 内部缓存与新文本不一致。
/// - 编辑器自身若失败(`$EDITOR` 未设、spawn 失败等),回填 draft 不变,推 Notice。
fn open_external_editor(
    state: &mut UiState,
    guard: &mut TerminalGuard,
    frame_requester: &FrameRequester,
) {
    // 抓取当前 composer 文本作为编辑器 seed:这样用户编辑的是「已有输入」,
    // 而不是从空 buffer 起步导致原内容丢失。
    let draft = state.composer.current_text();

    // 1. suspend terminal:leave alt-screen + 退出 raw mode。
    let _ = guard.leave_alt_screen();
    let _ = crossterm::terminal::disable_raw_mode();

    // 2. run editor。失败时尝试恢复 terminal 再返回。
    let result = crate::tui_core::external_editor::run_editor(&draft);

    // 3. restore terminal:raw mode + alt-screen,下一帧重绘。
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = frame_requester.schedule_frame();

    match result {
        Ok(edited) if edited == draft => {
            // 未修改,只刷帧。
        }
        Ok(edited) => {
            // 回填:把编辑后的内容塞回 composer buffer。
            state.composer.clear();
            if !edited.is_empty() {
                // 字符级逐个 insert 是 chat_composer 公开的稳定接口
                // (无 paste_burst 抖动)。Tier 3.1 简化:粘贴整个字符串。
                state.composer.handle_paste(edited);
            }
            state.history.push(UiHistoryItem::Notice(
                "External editor closed; content loaded into composer.".into(),
            ));
        }
        Err(e) => {
            state.history.push(UiHistoryItem::Notice(format!(
                "External editor failed: {e}"
            )));
        }
    }
}

/// v1.x Tier 3 (Ctrl+L):强制 redraw。
///
/// ratatui `Terminal` 不是 `Send`,不能在 `tokio::spawn` 内部 `clear()` +
/// `draw()`,所以走「调度下一帧」模式:返回 `Ok(false)` 后主循环下一轮
/// 自然 draw 一次。这是归档 TUI 的等价实现。
fn force_redraw(frame_requester: &FrameRequester) {
    frame_requester.schedule_frame();
}

// ── v1.x Tier 4: 新增 slash 命令辅助函数 ───────────────────────────────

/// v1.x `/copy [N]`：复制 history 中第 N 条 agent 回复（默认最后一条）到剪贴板。
/// N=0 或缺省 + 多条回复 → 打开 copy history overlay 让用户选择。
/// N=缺省 + 单条回复 → 直接复制最后一条。
/// N=给定值 → 直接复制第 N 条。失败时推 Notice,不静默。
fn copy_reply_n(n: Option<usize>, state: &mut UiState) {
    let agent_replies: Vec<String> = state
        .history
        .iter()
        .filter_map(|item| match item {
            adapter::UiHistoryItem::Agent(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    if agent_replies.is_empty() {
        state
            .history
            .push(UiHistoryItem::Notice("/copy: no agent reply yet.".into()));
        return;
    }

    // 无参数 + 多条回复 → 打开 overlay 让用户选
    if n.is_none() && agent_replies.len() > 1 {
        let overlay = crate::picker::copy_history::populate_from_history(&state.history).unwrap();
        state.copy_history = Some(overlay);
        state.history.push(adapter::UiHistoryItem::Notice(format!(
            "/copy: choose from {} replies (↑↓ · Enter copy · Esc cancel)",
            agent_replies.len()
        )));
        return;
    }

    // 单条 或 指定了 N → 直接复制
    let idx = n.unwrap_or(agent_replies.len()).saturating_sub(1);
    let Some(text) = agent_replies.get(idx) else {
        state.history.push(adapter::UiHistoryItem::Notice(format!(
            "/copy: only {} agent replies available; /copy {} out of range.",
            agent_replies.len(),
            idx + 1
        )));
        return;
    };
    match crate::tui_core::clipboard_copy::copy_to_clipboard(text) {
        Ok(lease) => {
            state.clipboard_lease = lease;
            state.history.push(adapter::UiHistoryItem::Notice(format!(
                "/copy: copied reply #{} ({} chars) to clipboard.",
                idx + 1,
                text.chars().count()
            )));
        }
        Err(e) => {
            state
                .history
                .push(UiHistoryItem::Notice(format!("/copy failed: {e}")));
        }
    }
}

/// v1.x `/version`：显示 TUI 版本 + git sha + rustc triple（best-effort）。
fn show_version(state: &mut UiState) {
    let version = env!("CARGO_PKG_VERSION");
    // git sha via -e 解析 `git rev-parse --short HEAD`（失败 → "unknown"）。
    let git_sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "--quiet", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let target = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "rustc unavailable".to_string());
    let msg = format!(
        "/version: Reflect TUI v{version}\n  git: {git_sha}\n  target: {target}-{os}\n  {rustc}"
    );
    state.history.push(UiHistoryItem::Notice(msg));
}

/// v1.x `/doctor`：环境自检（best-effort，failure 不视为致命）。
/// 检查项:git 可用、git 仓库、工作目录可写、reflect-protocol Op::Compact 可用。
fn run_doctor(state: &mut UiState) {
    let mut lines: Vec<String> = vec!["/doctor: Reflect TUI environment check".into()];

    // 1. git 可用
    let git_ok = std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    lines.push(format!(
        "  • git: {}",
        if git_ok { "OK" } else { "NOT FOUND" }
    ));

    // 2. git 仓库
    let in_repo = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false);
    lines.push(format!(
        "  • git repo: {}",
        if in_repo { "yes" } else { "no" }
    ));

    // 3. cwd 可写
    let cwd = std::env::current_dir().ok();
    let cwd_writable = cwd
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| !m.permissions().readonly())
        .unwrap_or(false);
    lines.push(format!(
        "  • cwd writable: {}",
        if cwd_writable { "yes" } else { "no" }
    ));

    // 4. 协议版本（reflect-protocol：Op::Compact 始终可用）
    lines.push("  • reflect-protocol: Op::Compact available".into());

    // 5. 档位
    lines.push("  • plan mode: supported (--plan-mode, /plan, /exit-plan)".into());
    lines.push("  • permission mode: supported (/mode, Shift+Tab)".into());

    state.history.push(UiHistoryItem::Notice(lines.join("\n")));
}

/// v1.x `/history [N]`：显示最近 N 次提交记录（来自 input 历史）。
/// 注意:live loop 当前未持久化 user input 历史(TODO v2);这里回退到
/// 显示「当前会话的 User 行 + 长度统计」。
fn show_input_history(state: &mut UiState, n: usize) {
    let user_count = state
        .history
        .iter()
        .filter(|h| matches!(h, adapter::UiHistoryItem::User(_)))
        .count();
    let mut msg = format!(
        "/history: {} user messages in current session (showing last {}).",
        user_count, n
    );
    let user_lines: Vec<String> = {
        let mut all_user: Vec<String> = state
            .history
            .iter()
            .filter_map(|h| match h {
                adapter::UiHistoryItem::User(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        all_user.reverse();
        all_user.truncate(n);
        all_user.reverse();
        all_user
            .into_iter()
            .map(|t| {
                let snippet = if t.chars().count() > 60 {
                    let mut s: String = t.chars().take(57).collect();
                    s.push('…');
                    s
                } else {
                    t
                };
                format!("  • {snippet}")
            })
            .collect()
    };
    if !user_lines.is_empty() {
        msg.push('\n');
        msg.push_str(&user_lines.join("\n"));
    }
    state.history.push(UiHistoryItem::Notice(msg));
}

/// v1.x `/files`：`git ls-files` 前 50（best-effort，非 git 仓库 → 提示）。
fn show_git_files(state: &mut UiState) {
    let Ok(out) = std::process::Command::new("git")
        .args(["ls-files"])
        .output()
    else {
        state
            .history
            .push(UiHistoryItem::Notice("/files: git not available.".into()));
        return;
    };
    if !out.status.success() {
        state.history.push(UiHistoryItem::Notice(
            "/files: not inside a git work tree.".into(),
        ));
        return;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let all: Vec<&str> = s.lines().collect();
    let n = all.len();
    let head = all.iter().take(50);
    let mut msg = format!("/files: {n} tracked files (showing first 50):");
    for f in head {
        msg.push('\n');
        msg.push_str("  • ");
        msg.push_str(f);
    }
    if n > 50 {
        msg.push_str(&format!("\n  … ({} more)", n - 50));
    }
    state.history.push(UiHistoryItem::Notice(msg));
}

/// v1.x `/branch [list|<name>|create <name>]`：git 分支只读 / 创建（不切换）。
/// 复杂操作（checkout、merge）不在 v1.4 范围。
fn show_branch(state: &mut UiState, arg: Option<&str>) {
    match arg {
        None | Some("list") | Some("ls") => {
            let Ok(out) = std::process::Command::new("git")
                .args(["branch", "--list", "--no-color"])
                .output()
            else {
                state
                    .history
                    .push(UiHistoryItem::Notice("/branch: git not available.".into()));
                return;
            };
            if !out.status.success() {
                state.history.push(UiHistoryItem::Notice(
                    "/branch: not inside a git work tree.".into(),
                ));
                return;
            }
            let s = String::from_utf8_lossy(&out.stdout);
            let branches: Vec<&str> = s
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.trim_start_matches("* "))
                .collect();
            let current = std::process::Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "--quiet", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty() && s != "HEAD")
                .unwrap_or_else(|| "(detached)".to_string());
            let mut msg = format!("/branch: {} branches, current = {current}", branches.len());
            for b in branches.iter().take(20) {
                msg.push('\n');
                msg.push_str("  • ");
                msg.push_str(b);
            }
            if branches.len() > 20 {
                msg.push_str(&format!("\n  … ({} more)", branches.len() - 20));
            }
            state.history.push(UiHistoryItem::Notice(msg));
        }
        Some(name) if name.starts_with("create ") => {
            let new_branch = name.trim_start_matches("create ").trim();
            if new_branch.is_empty() {
                state.history.push(UiHistoryItem::Notice(
                    "/branch create: missing branch name.".into(),
                ));
                return;
            }
            // 不切换,只创建(等价 git branch <name>)。
            match std::process::Command::new("git")
                .args(["branch", new_branch])
                .output()
            {
                Ok(o) if o.status.success() => {
                    state.history.push(UiHistoryItem::Notice(format!(
                        "/branch: created branch '{new_branch}' (not checked out)."
                    )));
                }
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    state.history.push(UiHistoryItem::Notice(format!(
                        "/branch create failed: {}",
                        err.trim()
                    )));
                }
                Err(_) => {
                    state.history.push(UiHistoryItem::Notice(
                        "/branch create: git not available.".into(),
                    ));
                }
            }
        }
        Some(name) => {
            // 给定名字 → 仅展示该分支的 HEAD commit（best-effort）。
            let Ok(out) = std::process::Command::new("git")
                .args(["log", "-1", "--oneline", name])
                .output()
            else {
                state
                    .history
                    .push(UiHistoryItem::Notice("/branch: git not available.".into()));
                return;
            };
            if !out.status.success() {
                state.history.push(UiHistoryItem::Notice(format!(
                    "/branch: no branch '{name}'."
                )));
                return;
            }
            let s = String::from_utf8_lossy(&out.stdout);
            let first = s.lines().next().unwrap_or("(empty)");
            state
                .history
                .push(UiHistoryItem::Notice(format!("/branch: {name} → {first}")));
        }
    }
}

/// 列出 config.toml 中已注册的 MCP servers。
///
/// `/mcp` 命令的本地实现：直接扫描 `~/.reflect/config.toml` 的 `[mcp_servers.XXX]`
/// 段，提取 server 名。不依赖外部 `reflect_config` 的富类型，保证 reflect-tui
/// 这一层可独立编译。连接状态与工具清单由后端 agent 运行时管理，故仅展示
/// 配置态并给出 `/debug-config` 指引。
fn format_configured_mcp_servers() -> String {
    use crate::utils_home_dir::find_reflect_home;
    let Some(home) = find_reflect_home() else {
        return "/mcp: 无法定位 Reflect 配置目录（REFLECT_HOME / HOME 均未设置）".into();
    };
    let path = home.join("config.toml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return format!(
            "/mcp: 未找到配置文件 {}\n  示例配置: [mcp_servers.filesystem]\n  command = \"npx\"\n  args = [\"-y\", \"@modelcontextprotocol/server-filesystem\", \"$HOME\"]",
            path.display()
        );
    };
    // 扫描 [mcp_servers.XXX] 表头，提取 server 名（简单可靠，不触发完整 TOML 解析）。
    let mut servers: Vec<&str> = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("[mcp_servers.") {
            if let Some(name) = rest.strip_suffix(']') {
                servers.push(name);
            }
        }
    }
    if servers.is_empty() {
        return format!(
            "/mcp: 配置文件中未注册任何 MCP server\n  路径: {}\n  示例: 在 config.toml 添加 [mcp_servers.filesystem] 段\n  详见 /debug-config",
            path.display()
        );
    }
    let mut out = format!("/mcp: 已配置 {} 个 MCP server\n", servers.len());
    for s in &servers {
        out.push_str(&format!("  • {s}\n"));
    }
    out.push_str(&format!("\n  配置文件: {}", path.display()));
    out.push_str("\n  提示: 连接状态与工具清单由后端 agent 运行时管理；用 /debug-config 查看完整配置层。");
    out
}

/// v1.x Tier 2：分发内嵌的 `SlashCommand`（无参数）到对应动作。
/// 镜像已归档的 `slash_dispatch.rs` 模式。
///
/// 有后端支持的命令发出 `Op`；本地 UI 命令（vim、diff、clear 等）
/// 直接操作 `UiState`；其他命令记录「尚未支持」通知。
/// 返回 `true` 表示应当退出 TUI（`/exit` / `/quit`）。此前该函数返回 `()`,
/// `SlashCommand::Exit`/`Quit` 分支只能推一条 "Exiting..." 通知却无法让
/// 调用方真正退出 —— 从命令弹窗选中 `/exit` 时 TUI 不会退出(文字提交
/// 路径走 `SlashOutcome::Exit` 能正确退出,但弹窗路径经此函数,漏了信号)。
fn dispatch_slash_command(
    cmd: SlashCommand,
    state: &mut UiState,
    thread: &Arc<AgentThread>,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
    history_tail: &mut usize,
    guard: &mut TerminalGuard,
) -> bool {
    match cmd {
        SlashCommand::Compact => {
            emit_op(thread, event_tx, reflect_protocol::Op::Compact);
            state.history.push(UiHistoryItem::Notice(
                "/compact: Context compaction requested. Next turn will compact.".into(),
            ));
        }
        SlashCommand::Vim => {
            let now_enabled = state.composer.toggle_vim_enabled();
            state.vim_enabled = now_enabled;
            let msg = if now_enabled {
                "Vim mode enabled — composer now in NORMAL. Press i to enter INSERT."
            } else {
                "Vim mode disabled — composer back to standard editing."
            };
            state.history.push(UiHistoryItem::Notice(msg.into()));
        }
        SlashCommand::Diff => {
            open_diff_overlay(state);
        }
        SlashCommand::Clear => {
            // 两步确认：显示提示并等待 y/n。
            state.pending_clear_confirm = true;
            state.history.push(UiHistoryItem::Notice(
                "⚠ Clear scrollback? This will erase all conversation history. [Y]es / [N]o".into(),
            ));
        }
        SlashCommand::Exit | SlashCommand::Quit => {
            // 文字提交路径走 `SlashOutcome::Exit => Ok(true)`;命令弹窗路径
            // 经此函数,故返回 `true` 让调用方(handle_key)以 Ok(true) 退出 TUI。
            state
                .history
                .push(UiHistoryItem::Notice("/exit: Exiting...".into()));
            return true;
        }
        SlashCommand::Status => {
            let model = state.status_model.clone().unwrap_or_else(|| {
                std::env::var("REFLECT_MODEL_DISPLAY")
                    .or_else(|_| std::env::var("OPENAI_MODEL"))
                    .or_else(|_| std::env::var("ANTHROPIC_MODEL"))
                    .unwrap_or_else(|_| "reflect".to_string())
            });
            let cwd = std::env::current_dir()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| ".".to_string());
            let branch = history_render::statusline::detect_git_branch()
                .unwrap_or_else(|| "(none)".to_string());
            let vim = if state.vim_enabled { "ON" } else { "OFF" };
            let cost = state
                .context_usage
                .total_cost_usd
                .map(|c| format!("${:.4}", c))
                .unwrap_or_else(|| "N/A".to_string());
            let tokens = state
                .context_usage
                .last_input_tokens
                .map(|t| format!("{} tokens", t))
                .unwrap_or_else(|| "N/A".to_string());
            let context_window = state
                .context_usage
                .window_size
                .map(|w| format!("{} tokens", w))
                .unwrap_or_else(|| "N/A".to_string());

            let status = format!(
                "/status:\n  model: {}\n  cwd: {}\n  git: {}\n  mode: {}\n  vim: {}\n  cost: {}\n  tokens: {}\n  context window: {}\n  busy: {}",
                model,
                cwd,
                branch,
                state.permission_mode.as_str(),
                vim,
                cost,
                tokens,
                context_window,
                state.busy
            );
            state.history.push(UiHistoryItem::Notice(status));
        }
        SlashCommand::Goal => {
            state.history.push(UiHistoryItem::Notice(
                "/goal: usage: /goal <objective>  (e.g. /goal refactor auth module)".into(),
            ));
        }
        SlashCommand::Loop => {
            state.history.push(UiHistoryItem::Notice(
                "/loop: usage: /loop <secs> <cmd>  (e.g. /loop 30 /status)  /loop stop to cancel"
                    .into(),
            ));
        }
        SlashCommand::Usage => {
            let input = state.context_usage.last_input_tokens.unwrap_or(0);
            let cached = state.context_usage.last_cached_tokens.unwrap_or(0);
            let window = state
                .context_usage
                .window_size
                .map(|w| format!(" / window: {} tokens", w))
                .unwrap_or_default();
            state.history.push(UiHistoryItem::Notice(format!(
                "/usage: input: {} tokens, cached: {} tokens{}",
                input, cached, window
            )));
        }
        // ── v1.x 补全:此前落到兜底「not yet supported from popup」的命令 ──
        // 这些命令 composer 无法解析为 SlashOutcome（仅注册了名字/描述），
        // 会归为 ComposerAction::Command 走本函数；原实现只给了误导性提示
        // （「try typing it directly」），但直接文本输入同样走本路径，
        // 因此这里是它们的唯一落地处，必须给出真实反馈而非误导。
        SlashCommand::MultiAgents => {
            // /subagents — 列出当前会话已注册的子 agent / 协作参与者。
            // 数据来自 UiState::agents（由后端 SubAgentStarted 等事件回流）。
            if state.agents.is_empty() {
                state.history.push(UiHistoryItem::Notice(
                    "/subagents: 当前无活跃子 agent。任务派发的子 agent 会出现在这里。".into(),
                ));
            } else {
                let mut lines = String::from("/subagents: 活跃子 agent 列表\n");
                for a in &state.agents {
                    lines.push_str(&format!("  • {} — {}\n", a.id, a.label));
                }
                state.history.push(UiHistoryItem::Notice(lines));
            }
        }
        SlashCommand::Mcp => {
            // /mcp — 列出 config.toml 中已注册的 MCP servers。
            // 不依赖外部 config 类型，直接扫描配置文件，保证本层可独立编译。
            state
                .history
                .push(UiHistoryItem::Notice(format_configured_mcp_servers()));
        }
        SlashCommand::Side | SlashCommand::Btw => {
            // /side、/btw 需要带消息内容发起临时侧边对话。
            // 无参调用时给出用法指引（带参路径见 dispatch_slash_command_with_args）。
            state.history.push(UiHistoryItem::Notice(
                "/side: 用法 /side <message> — 在临时 fork 中发起侧边对话。请带上消息内容。".into(),
            ));
        }
        SlashCommand::Skills => {
            state.history.push(UiHistoryItem::Notice(
                "/skills: 技能管理未接入当前构建。可用 /debug-config 查看配置层。".into(),
            ));
        }
        SlashCommand::Hooks => {
            state.history.push(UiHistoryItem::Notice(
                "/hooks: 生命周期 hooks 浏览未接入当前构建。配置见 ~/.reflect/config.toml [hooks]。".into(),
            ));
        }
        SlashCommand::Permissions => {
            // /permissions — 循环切换权限模式（Plan → Prompt → AcceptEdits → Auto）。
            emit_op(thread, event_tx, reflect_protocol::Op::CyclePermissionMode);
        }
        _ => {
            // 兜底：明确告知「未实现」，不再用「try typing it directly」误导
            // （直接输入与本路径同源，提示重试无意义）。用 command() 显示命令名，
            // 而非 description()（后者返回完整描述句，会让消息变成
            // 「/switch to Plan mode: ...」这种不可读形式）。
            state.history.push(UiHistoryItem::Notice(format!(
                "/{}: 此命令在当前构建中尚未接入分发",
                cmd.command()
            )));
        }
    }
    let _ = (guard, history_tail); // silence unused warns for future use
    false
}

/// v1.x Tier 2：分发带参数的供应商化 `SlashCommand`。
///
/// 处理需要尾随参数的命令（例如 `/plan <task>`、`/model <name>`）。
/// 未知组合记录一条提示并直接放行。
fn dispatch_slash_command_with_args(
    cmd: SlashCommand,
    args: String,
    state: &mut UiState,
    thread: &Arc<AgentThread>,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
    history_tail: &mut usize,
) {
    let _ = history_tail; // silence unused warn for future use
    match cmd {
        SlashCommand::Plan => {
            let task = args.trim().to_string();
            if task.is_empty() {
                state.history.push(UiHistoryItem::Notice(
                    "/plan: usage: /plan <task description>  (e.g. /plan refactor auth module)"
                        .into(),
                ));
                return;
            }
            emit_op(
                thread,
                event_tx,
                reflect_protocol::Op::EnterPlanMode { task },
            );
        }
        SlashCommand::Goal => {
            let goal = args.trim().to_string();
            if goal.is_empty() || goal.eq_ignore_ascii_case("clear") {
                emit_op(thread, event_tx, reflect_protocol::Op::ExitGoalMode);
                state
                    .history
                    .push(UiHistoryItem::Notice("/goal: Goal mode cleared.".into()));
                return;
            }
            emit_op(
                thread,
                event_tx,
                reflect_protocol::Op::EnterGoalMode {
                    goal,
                    verify_command: None,
                    token_budget: None,
                },
            );
            state
                .history
                .push(UiHistoryItem::Notice("/goal: Goal mode entered.".into()));
        }
        SlashCommand::Loop => {
            // `/loop <secs> <cmd>` 本地调度器(无 agent round-trip)。
            // 与文本提交路径(SlashOutcome::Loop)语义一致:解析参数 → 设置/替换/
            // 取消 state.loop_state,主循环每帧推进 next_fire。
            let trimmed = args.trim();
            if trimmed.is_empty() {
                state.history.push(UiHistoryItem::Notice(
                    "/loop: usage: /loop <secs> <cmd>  (e.g. /loop 30 /status)  \
                     /loop stop to cancel"
                        .into(),
                ));
                return;
            }
            let mut parts = trimmed.splitn(2, char::is_whitespace);
            let first = parts.next().unwrap_or("").trim();
            let rest_after = parts.next().unwrap_or("").trim();
            if first.eq_ignore_ascii_case("stop") || first == "0" {
                if let Some(prev) = state.loop_state.take() {
                    state.history.push(UiHistoryItem::Notice(format!(
                        "/loop: stopped (was: {} every {}s)",
                        prev.command, prev.interval_secs
                    )));
                } else {
                    state.history.push(UiHistoryItem::Notice(
                        "/loop: no active loop to stop".into(),
                    ));
                }
                return;
            }
            let secs: u64 = match first.parse() {
                Ok(n) if n >= 1 => n,
                _ => {
                    state.history.push(UiHistoryItem::Notice(format!(
                        "/loop: invalid interval '{first}'. Use /loop <secs> <cmd> \
                         (secs must be >= 1)"
                    )));
                    return;
                }
            };
            if rest_after.is_empty() {
                state.history.push(UiHistoryItem::Notice(
                    "/loop: missing command. Use /loop <secs> <cmd>".into(),
                ));
                return;
            }
            let next_fire = Instant::now() + Duration::from_secs(secs);
            if let Some(prev) = state
                .loop_state
                .replace(LoopState { interval_secs: secs, command: rest_after.to_string(), next_fire })
            {
                state.history.push(UiHistoryItem::Notice(format!(
                    "/loop: replaced previous ({} every {}s) with new ({} every {}s)",
                    prev.command, prev.interval_secs, rest_after, secs
                )));
            } else {
                state.history.push(UiHistoryItem::Notice(format!(
                    "/loop: started — running `{}` every {}s. /loop stop to cancel.",
                    rest_after, secs
                )));
            }
        }
        SlashCommand::Model => {
            let model = args.trim().to_string();
            if model.is_empty() {
                state.history.push(UiHistoryItem::Notice(
                    "/model: Missing model name. Usage: /model <model-name>".into(),
                ));
                return;
            }
            state.history.push(UiHistoryItem::Notice(format!(
                "/model: Model changed to '{}' (display only — full model switching pending).",
                model
            )));
        }
        SlashCommand::Side | SlashCommand::Btw => {
            let msg = args.trim().to_string();
            if msg.is_empty() {
                state.history.push(UiHistoryItem::Notice(
                    "/side: 用法 /side <message>".into(),
                ));
            } else {
                // 后端 StartSide 事件尚未在 reflect_protocol::Op 暴露，
                // 当前仅回显，避免静默丢弃用户输入（此前会落到误导兜底）。
                state.history.push(UiHistoryItem::Notice(format!(
                    "/side: 侧边对话「{msg}」已收到，但后端 StartSide 通道未接入当前构建。\
                     可改用 /fork 在新会话继续。"
                )));
            }
        }
        _ => {
            // 兜底（带参版）：明确告知未实现，用 command() 显示命令名。
            state.history.push(UiHistoryItem::Notice(format!(
                "/{}: 此命令（带参数）在当前构建中尚未接入分发",
                cmd.command()
            )));
        }
    }
}

/// 打开 diff 全屏 overlay:扫 history 收集所有 ToolCall 的 unified_diff,
/// 经 Reflect `create_diff_summary`(语法高亮 + +/- 着色)渲染成行,喂给 pager。
fn open_diff_overlay(state: &mut UiState) {
    let width = term_size().map(|(w, _)| w).unwrap_or(80) as usize;
    let mut changes: std::collections::HashMap<
        std::path::PathBuf,
        crate::tui_core::diff_model::FileChange,
    > = std::collections::HashMap::new();
    let mut idx = 0usize;
    for item in &state.history {
        if let adapter::UiHistoryItem::ToolCall {
            name: _,
            diff: Some(diff),
            ..
        } = item
        {
            let key = derive_diff_path(diff, idx);
            changes.insert(
                key,
                crate::tui_core::diff_model::FileChange::Update {
                    unified_diff: diff.clone(),
                    move_path: None,
                },
            );
            idx += 1;
        }
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    if changes.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no diffs in this session)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.extend(crate::tui_core::diff_render::create_diff_summary(
            &changes, &state.cwd, width,
        ));
    }
    let mut pager = TranscriptPager::new(lines);
    pager.title = "Diff (Esc / Ctrl+T to close · /diff)".into();
    state.diff_overlay = Some(pager);
}

/// 从 unified_diff 文本里 best-effort 推导文件路径(做 changes map 的 key)。
/// 取不到就按序号生成稳定占位路径,避免多 diff 互相覆盖。
fn derive_diff_path(diff: &str, fallback_idx: usize) -> std::path::PathBuf {
    // unified diff 头形如 `diff --git a/foo b/foo` 或 `+++ b/foo`。
    for line in diff.lines().take(8) {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            let p = rest.trim();
            if !p.is_empty() && p != "/dev/null" {
                return std::path::PathBuf::from(p);
            }
        }
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            // `diff --git a/foo b/foo` → 取 `foo`(首个空格前)。
            let p = rest.split_whitespace().next().unwrap_or(rest).trim();
            if !p.is_empty() {
                return std::path::PathBuf::from(p);
            }
        }
    }
    std::path::PathBuf::from(format!("diff-{fallback_idx}.txt"))
}

/// 打开 tasks/agent overlay(alt-screen 双栏:左 message_list + 右 task/agent)。
fn open_tasks_overlay(state: &mut UiState) {
    state.tasks_overlay = Some(crate::tasks_pager::TasksPager::default());
}

/// v1.x Tier 4.5:打开 session overlay(alt-screen 双栏,复用 TasksPager 渲染)。
/// 左 message_list + 右 session list(/resume /fork /rename 入口)。
fn open_session_overlay(state: &mut UiState) {
    state.session_overlay = Some(crate::tasks_pager::TasksPager::default());
}

/// v1.x Tier 4.5:打开 image picker overlay(`/image` / `/image ls`)。
/// 扫描 cwd 下支持的图片文件,允许选中后 `@<path>` 回填 composer。
fn open_image_picker(state: &mut UiState) {
    let mut picker = crate::events::ImagePickerState::default();
    crate::picker::image_picker::scan_cwd(&mut picker);
    state.image_picker = Some(picker);
}

/// v1.x Tier 4.5:打开 keymap picker overlay(`/keymap`)。
fn open_keymap_picker(state: &mut UiState) {
    let mut picker = crate::events::KeymapPickerState::default();
    crate::picker::keymap_picker::fill_defaults(&mut picker);
    state.keymap_picker = Some(picker);
}

/// v1.x Tier 4.5:打开 statusline picker overlay(`/statusline`)。
fn open_statusline_picker(state: &mut UiState) {
    state.statusline_picker = Some(crate::events::StatuslinePickerState::default());
}

/// v1.x Tier 4.5:打开 theme picker overlay(`/theme picker`)。
fn open_theme_picker(state: &mut UiState) {
    state.theme_picker = Some(crate::events::ThemePickerState::default());
}

/// v1.x Tier 5:打开 checkpoint overlay(`/checkpoint`)。
fn open_checkpoint_overlay(state: &mut UiState) {
    // 占位:空列表,用户可通过 c 创建首个 checkpoint。
    state.checkpoint_overlay =
        Some(crate::picker::checkpoint_overlay::CheckpointOverlayState::default());
}

/// v1.x Tier 5:打开 fork select overlay(`/fork` 无参数时)。
fn open_fork_select(state: &mut UiState) {
    use crate::picker::fork_rewind::{ForkRewindMode, populate_from_history};
    let Some(overlay) = populate_from_history(&state.history, ForkRewindMode::Fork) else {
        state.history.push(adapter::UiHistoryItem::Notice(
            "/fork: no user prompts to fork from yet.".into(),
        ));
        return;
    };
    state.fork_rewind = Some(overlay);
}

/// v1.x Tier 5:打开 rewind select overlay(`/rewind`)。
fn open_rewind_select(state: &mut UiState) {
    use crate::picker::fork_rewind::{ForkRewindMode, populate_from_history};
    let Some(overlay) = populate_from_history(&state.history, ForkRewindMode::Rewind) else {
        state.history.push(adapter::UiHistoryItem::Notice(
            "/rewind: no user prompts to rewind to yet.".into(),
        ));
        return;
    };
    state.fork_rewind = Some(overlay);
}

/// v1.x Tier 4.5:填充 `state.session_entries`(best-effort 占位数据,供 picker 演示)。
///
/// 真实实现需要从 `~/.reflect/sessions/*.jsonl` 或 runtime API 拉取;本阶段给 1 条
/// 「current session」占位,避免空列表。失败不报错。
fn seed_session_entries(state: &mut UiState) {
    if !state.session_entries.is_empty() {
        return; // 已 seed 过,避免重复
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    state.session_entries.push(crate::events::SessionEntry {
        id: "current".into(),
        name: "Current session".into(),
        created_at: format!("{now}"),
        last_active: "now".into(),
        is_current: true,
    });
}

fn handle_key(
    key: KeyEvent,
    state: &mut UiState,
    guard: &mut TerminalGuard,
    thread: &Arc<AgentThread>,
    event_tx: &mpsc::UnboundedSender<UiEvent>,
    history_tail: &mut usize,
) -> anyhow::Result<bool> {
    // v1.x Plan mode 审批弹窗:1/2/3/Esc + 滚动键 优先拦截(plan_approval 打开时)。
    if let Some(plan) = state.plan_approval.clone() {
        let choice = match key.code {
            KeyCode::Char('1') => Some(reflect_protocol::PlanApprovalChoice::AutoMode),
            KeyCode::Char('2') => Some(reflect_protocol::PlanApprovalChoice::ManualApprove),
            KeyCode::Char('3') | KeyCode::Esc => Some(reflect_protocol::PlanApprovalChoice::Revise),
            _ => None,
        };
        if let Some(choice) = choice {
            emit_op(
                thread,
                event_tx,
                reflect_protocol::Op::PlanApproval {
                    id: plan.plan_id,
                    choice,
                },
            );
            // 乐观关闭弹窗;若 runtime reject(Revise),PlanRejected 事件会重新置位。
            // 对 AutoMode/ManualApprove,PlanApproved 事件会清空 plan_approval。
            // 为避免 Revise 时弹窗闪一下又开,这里仅在非 Revise 时乐观关闭。
            if !matches!(choice, reflect_protocol::PlanApprovalChoice::Revise) {
                state.plan_approval = None;
            }
            return Ok(false);
        }
        // 注意:plan 正文已在 scrollback 中完整展示(ProposedPlanCell),
        // 不再需要弹窗内滚动 —— 用户可用终端原生滚动浏览 plan。
    }
    // v1.x Plan mode 进入确认条(plan_enter_request 打开时):1=进入(AutoMode),
    // 3/Esc=取消(Revise)。与 plan_approval 不同的是,这是「是否进入 plan 模式」
    // 的前置确认,core 的 spawn_plan_enter_waiter 阻塞在对应 plan_id 上。
    if let Some(req) = state.plan_enter_request.clone() {
        let choice = match key.code {
            KeyCode::Char('1') => Some(reflect_protocol::PlanApprovalChoice::AutoMode),
            KeyCode::Char('3') | KeyCode::Esc => Some(reflect_protocol::PlanApprovalChoice::Revise),
            _ => None,
        };
        if let Some(choice) = choice {
            emit_op(
                thread,
                event_tx,
                reflect_protocol::Op::PlanApproval {
                    id: req.plan_id,
                    choice,
                },
            );
            // 乐观关闭;Revise 时 core 会留在当前模式,无需重开。
            state.plan_enter_request = None;
            if matches!(choice, reflect_protocol::PlanApprovalChoice::AutoMode) {
                state.history.push(UiHistoryItem::Notice(
                    "✓ Entering plan mode…".into(),
                ));
            }
            return Ok(false);
        }
    }

    // 审批快捷键：有 pending 时 y/n/a 优先。
    // y → Approve / n → Deny / a → ApproveForSession，
    // 据 pending.kind 回送 Op::ToolApproval / Op::HookApproval 唤醒 ApprovalGate。
    if let Some(ref pending) = state.pending_approval.clone() {
        let decision = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                Some(reflect_protocol::ReviewDecision::Approve)
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                Some(reflect_protocol::ReviewDecision::Deny {
                    reason: "denied by user".to_string(),
                })
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                Some(reflect_protocol::ReviewDecision::ApproveForSession)
            }
            _ => None,
        };
        if let Some(decision) = decision {
            // Plan 审批走 Op::PlanApproval（由 plan_approval 弹窗处理），
            // 不应触达此分支；防御性只记 Notice。
            let label = match decision {
                reflect_protocol::ReviewDecision::Approve => "Approved",
                reflect_protocol::ReviewDecision::Deny { .. } => "Denied",
                reflect_protocol::ReviewDecision::ApproveForSession => "Always allow",
            };
            state.history.push(adapter::UiHistoryItem::Notice(format!(
                "{label}: {}",
                pending.summary
            )));
            match &pending.kind {
                reflect_protocol::ApprovalKind::Tool { .. } => {
                    emit_op(
                        thread,
                        event_tx,
                        reflect_protocol::Op::ToolApproval {
                            id: pending.id.clone(),
                            decision,
                        },
                    );
                }
                reflect_protocol::ApprovalKind::Hook { .. } => {
                    emit_op(
                        thread,
                        event_tx,
                        reflect_protocol::Op::HookApproval {
                            id: pending.id.clone(),
                            decision,
                        },
                    );
                }
                reflect_protocol::ApprovalKind::Plan { .. } => {
                    // plan 审批由 plan_approval 弹窗处理；此处仅清空本地状态。
                }
            }
            // TODO:持久化 always-allow 规则到 config.toml [permissions] 段。
            state.pending_approval = None;
            return Ok(false);
        }
    }
    // /clear 二次确认:有 pending_clear_confirm 时 y/n 优先。
    if state.pending_clear_confirm {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                // 确认清除。
                state.pending_clear_confirm = false;
                state.history.clear();
                *history_tail = 0;
                guard
                    .terminal()
                    .clear_scrollback_and_visible_screen_ansi()?;
                state
                    .history
                    .push(adapter::UiHistoryItem::Notice("Scrollback cleared.".into()));
                return Ok(false);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                // 取消清除。
                state.pending_clear_confirm = false;
                state
                    .history
                    .push(adapter::UiHistoryItem::Notice("Clear cancelled.".into()));
                return Ok(false);
            }
            _ => {}
        }
    }

    if matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
        state.composer.force_flush_paste_burst();
    }

    // Esc 在「没有任何 picker/checkpoint/fork/copy overlay 打开」时才落入
    // composer(让 composer 用它 abort 候选/清空),否则会**抢先**消费掉 Esc,
    // 导致下方 8 个 picker 的 `handle_key`(Esc → Close)永远拿不到键 ——
    // 用户无法用 Esc 关闭任何 picker overlay。
    // plan/pending/clear-confirm 已在上面提前 return,不受影响。
    let no_picker_open = state.checkpoint_overlay.is_none()
        && state.fork_rewind.is_none()
        && state.copy_history.is_none()
        && state.skills_hub.is_none()
        && state.image_picker.is_none()
        && state.keymap_picker.is_none()
        && state.statusline_picker.is_none()
        && state.theme_picker.is_none();
    if no_picker_open
        && key.code == KeyCode::Esc
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        let _ = state.composer.input(key);
        return Ok(false);
    }

    // v1.x Tier 5: checkpoint overlay 独占键位。
    if state.checkpoint_overlay.is_some() {
        let action = if let Some(o) = state.checkpoint_overlay.as_mut() {
            crate::picker::checkpoint_overlay::handle_key(o, key)
        } else {
            crate::picker::checkpoint_overlay::CheckpointAction::Close
        };
        match action {
            crate::picker::checkpoint_overlay::CheckpointAction::Close => {
                state.checkpoint_overlay = None;
                return Ok(false);
            }
            crate::picker::checkpoint_overlay::CheckpointAction::Create => {
                // git auto-commit 通过 emit_op 触发,目前作为占位通知。
                state.history.push(adapter::UiHistoryItem::Notice(
                    "📸 checkpoint: git auto-commit not yet wired (use CLI `git commit`).".into(),
                ));
                return Ok(false);
            }
            crate::picker::checkpoint_overlay::CheckpointAction::Rewind(sha) => {
                state.checkpoint_overlay = None;
                state.checkpoint_rewind_sha = Some(sha);
                return Ok(false);
            }
            crate::picker::checkpoint_overlay::CheckpointAction::Copy(sha) => {
                // 复制 sha 到剪贴板(终端支持 OSC52 时走 vendored clipboard_copy)。
                let short: String = sha.chars().take(8).collect();
                match crate::tui_core::clipboard_copy::copy_to_clipboard(&sha) {
                    Ok(lease) => {
                        state.clipboard_lease = lease;
                        state.history.push(adapter::UiHistoryItem::Notice(format!(
                            "📋 sha copied: {short} (full sha in clipboard)"
                        )));
                    }
                    Err(e) => state.history.push(adapter::UiHistoryItem::Notice(format!(
                        "📋 sha copy failed ({e}): {short}"
                    ))),
                }
                return Ok(false);
            }
            crate::picker::checkpoint_overlay::CheckpointAction::Refresh => {
                state.history.push(adapter::UiHistoryItem::Notice(
                    "🔄 checkpoints refreshed (placeholder).".into(),
                ));
                return Ok(false);
            }
            crate::picker::checkpoint_overlay::CheckpointAction::Nop => {}
        }
    }

    // v1.x Tier 5: fork/rewind select overlay 独占键位。
    if state.fork_rewind.is_some() {
        let action = if let Some(o) = state.fork_rewind.as_mut() {
            crate::picker::fork_rewind::handle_key(o, key)
        } else {
            crate::picker::fork_rewind::ForkRewindAction::Close
        };
        match action {
            crate::picker::fork_rewind::ForkRewindAction::Close => {
                state.fork_rewind = None;
                return Ok(false);
            }
            crate::picker::fork_rewind::ForkRewindAction::Confirm => {
                // 根据模式(fork/rewind)执行不同动作。
                let overlay = state.fork_rewind.take().unwrap();
                let text = crate::picker::fork_rewind::get_selected_text(&overlay, &state.history);
                match overlay.mode {
                    crate::picker::fork_rewind::ForkRewindMode::Fork => {
                        // v1.x fork 接线:调 reflect_rollout::index::fork_with_history
                        // 创建子会话(原样复制父 records 截至选中 turn),然后提示用
                        // --resume 切换。与 reflect-cli session fork 语义一致,
                        // 不做进程内 thread 切换(避免重构 run_async)。
                        let parent_id = match state.session_id {
                            Some(id) => id,
                            None => {
                                state.history.push(adapter::UiHistoryItem::Error(
                                    "/fork: current session id unavailable (cannot fork).".into(),
                                ));
                                return Ok(false);
                            }
                        };
                        let up_to = crate::picker::fork_rewind::get_selected_turn_id(
                            &overlay,
                            &state.user_prompt_turns,
                        );
                        let base = reflect_rollout::path::default_base();
                        match reflect_rollout::index::fork_with_history(
                            &base,
                            parent_id,
                            "tui-fork",
                            up_to.as_ref(),
                        ) {
                            Ok(child_id) => {
                                let preview = text
                                    .map(|t| {
                                        let p: String = t.chars().take(50).collect();
                                        format!(" at prompt: {p}…")
                                    })
                                    .unwrap_or_default();
                                state.history.push(adapter::UiHistoryItem::Notice(format!(
                                    "/fork: forked{preview}\n  → {child_id}\n  Resume with: reflect-tui --resume {child_id}"
                                )));
                            }
                            Err(e) => {
                                state.history.push(adapter::UiHistoryItem::Error(format!(
                                    "/fork: failed to fork session {parent_id}: {e}"
                                )));
                            }
                        }
                    }
                    crate::picker::fork_rewind::ForkRewindMode::Rewind => {
                        // rewind:回填 prompt 到 composer,发送 Op::Rewind。
                        if let Some(t) = text {
                            state.composer.clear();
                            state.composer.handle_paste(t);
                            emit_op(
                                thread,
                                event_tx,
                                reflect_protocol::Op::Rewind { to_turn_id: None },
                            );
                            state.history.push(adapter::UiHistoryItem::Notice(
                                "/rewind: loaded prompt — edit and Enter to re-send.".into(),
                            ));
                        } else {
                            state.history.push(adapter::UiHistoryItem::Notice(
                                "/rewind: selected prompt not found.".into(),
                            ));
                        }
                    }
                }
                return Ok(false);
            }
            crate::picker::fork_rewind::ForkRewindAction::Nop => {}
        }
    }

    // v1.x Tier 5: copy history select overlay 独占键位。
    if state.copy_history.is_some() {
        let action = if let Some(o) = state.copy_history.as_mut() {
            crate::picker::copy_history::handle_key(o, key)
        } else {
            crate::picker::copy_history::CopyHistoryAction::Close
        };
        match action {
            crate::picker::copy_history::CopyHistoryAction::Close => {
                state.copy_history = None;
                return Ok(false);
            }
            crate::picker::copy_history::CopyHistoryAction::Copy => {
                let overlay = state.copy_history.take().unwrap();
                let text = overlay
                    .entries
                    .get(overlay.selected)
                    .cloned()
                    .unwrap_or_default();
                let selected_n = overlay.selected + 1;
                let total = overlay.entries.len();
                match crate::tui_core::clipboard_copy::copy_to_clipboard(&text) {
                    Ok(lease) => {
                        state.clipboard_lease = lease;
                        state.history.push(adapter::UiHistoryItem::Notice(format!(
                            "/copy: copied reply {selected_n}/{total} ({} chars) to clipboard.",
                            text.chars().count()
                        )));
                    }
                    Err(e) => {
                        state
                            .history
                            .push(adapter::UiHistoryItem::Notice(format!("/copy failed: {e}")));
                    }
                }
                return Ok(false);
            }
            crate::picker::copy_history::CopyHistoryAction::Nop => {}
        }
    }

    // v1.x Tier 5: skills hub overlay 独占键位。
    if state.skills_hub.is_some() {
        let action = if let Some(o) = state.skills_hub.as_mut() {
            crate::picker::skills_hub::handle_key(o, key)
        } else {
            crate::picker::skills_hub::SkillsHubAction::Close
        };
        match action {
            crate::picker::skills_hub::SkillsHubAction::Close => {
                state.skills_hub = None;
                return Ok(false);
            }
            crate::picker::skills_hub::SkillsHubAction::Nop => {}
        }
    }

    // v1.x Tier 4.5: image picker overlay 独占键位。
    if state.image_picker.is_some() {
        let action = if let Some(o) = state.image_picker.as_mut() {
            crate::picker::image_picker::handle_key(o, key)
        } else {
            crate::picker::image_picker::ImagePickerAction::Close
        };
        match action {
            crate::picker::image_picker::ImagePickerAction::Close => {
                state.image_picker = None;
                return Ok(false);
            }
            crate::picker::image_picker::ImagePickerAction::Attach(path) => {
                state.image_picker = None;
                // 回填 @<path> 到 composer(对齐 /image 文档语义)。
                let mention = format!("@{}", path.display());
                state.composer.handle_paste(mention);
                return Ok(false);
            }
            crate::picker::image_picker::ImagePickerAction::Nop => {}
        }
    }

    // v1.x Tier 4.5: keymap picker overlay 独占键位(只展示,rebind 未实现)。
    if state.keymap_picker.is_some() {
        let action = if let Some(o) = state.keymap_picker.as_mut() {
            crate::picker::keymap_picker::handle_key(o, key)
        } else {
            crate::picker::keymap_picker::KeymapPickerAction::Close
        };
        match action {
            crate::picker::keymap_picker::KeymapPickerAction::Close => {
                state.keymap_picker = None;
                return Ok(false);
            }
            crate::picker::keymap_picker::KeymapPickerAction::Nop => {}
        }
    }

    // v1.x Tier 4.5: statusline picker overlay 独占键位(Tab 切段 / Enter toggle)。
    if state.statusline_picker.is_some() {
        let action = if let Some(o) = state.statusline_picker.as_mut() {
            crate::picker::statusline_picker::handle_key(o, key)
        } else {
            crate::picker::statusline_picker::StatuslinePickerAction::Close
        };
        match action {
            crate::picker::statusline_picker::StatuslinePickerAction::Close => {
                state.statusline_picker = None;
                return Ok(false);
            }
            crate::picker::statusline_picker::StatuslinePickerAction::Nop => {}
        }
    }

    // v1.x Tier 4.5: theme picker overlay 独占键位(Enter 应用当前选择)。
    if state.theme_picker.is_some() {
        let action = if let Some(o) = state.theme_picker.as_mut() {
            crate::picker::theme_picker::handle_key(o, key)
        } else {
            crate::picker::theme_picker::ThemePickerAction::Close
        };
        match action {
            crate::picker::theme_picker::ThemePickerAction::Close => {
                state.theme_picker = None;
                return Ok(false);
            }
            crate::picker::theme_picker::ThemePickerAction::Apply => {
                // picker.current 已由 handle_key 回写;调色板真正切换留待后续。
                let theme_id = state
                    .theme_picker
                    .as_ref()
                    .and_then(|p| p.themes.get(p.selected))
                    .map(|t| t.id.clone());
                state.theme_picker = None;
                if let Some(id) = theme_id {
                    state.history.push(adapter::UiHistoryItem::Notice(format!(
                        "/theme: switched to '{id}' (display only — palette apply not yet wired)."
                    )));
                }
                return Ok(false);
            }
            crate::picker::theme_picker::ThemePickerAction::Nop => {}
        }
    }

    match state.composer.input(key) {
        ComposerAction::Submitted(text) => {
            if text.is_empty() {
                return Ok(false);
            }
            match history_render::dispatch_slash(&text) {
                SlashOutcome::Exit => return Ok(true),
                SlashOutcome::Help => {
                    let lines: Vec<Line<'static>> = history_render::help_lines();
                    state.transcript = Some(TranscriptPager::new(lines));
                    return Ok(false);
                }
                SlashOutcome::ConfirmClear => {
                    // 二次确认:显示确认提示,等待 y/n 输入。
                    state.pending_clear_confirm = true;
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "⚠ Clear scrollback? This will erase all conversation history. [Y]es / [N]o".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::Clear => {
                    // 真正执行清除(由 y 键触发)。
                    state.history.clear();
                    *history_tail = 0;
                    guard
                        .terminal()
                        .clear_scrollback_and_visible_screen_ansi()?;
                    state
                        .history
                        .push(adapter::UiHistoryItem::Notice("Scrollback cleared.".into()));
                    return Ok(false);
                }
                SlashOutcome::OpenDiff => {
                    open_diff_overlay(state);
                    return Ok(false);
                }
                SlashOutcome::OpenTasks => {
                    open_tasks_overlay(state);
                    return Ok(false);
                }
                SlashOutcome::ToggleVim { .. } => {
                    // 真正翻转:由 ComposerInput.toggle_vim_enabled() 完成,
                    // 并把新状态写回 state.vim_enabled(状态行回显用)。
                    let now_enabled = state.composer.toggle_vim_enabled();
                    state.vim_enabled = now_enabled;
                    let msg = if now_enabled {
                        "Vim mode enabled — composer now in NORMAL. Press i to enter INSERT."
                    } else {
                        "Vim mode disabled — composer back to standard editing."
                    };
                    state
                        .history
                        .push(adapter::UiHistoryItem::Notice(msg.into()));
                    return Ok(false);
                }
                SlashOutcome::Compact => {
                    // v1.2 P1-12: `Op::Compact` 通过 reflect_protocol::Submission
                    // 发送到 agent thread,触发 force_compact_next 标志。
                    use reflect_protocol::{Op, Submission};
                    let thread = Arc::clone(thread);
                    let tx = event_tx.clone();
                    tokio::spawn(async move {
                        let sub =
                            Submission::with_id(uuid::Uuid::new_v4().to_string(), Op::Compact);
                        let mut handle = thread.submit(sub).await;
                        while let Some(protocol_event) = handle.next().await {
                            if let Some(ui_event) = conversion::convert_event(protocol_event) {
                                if tx.send(ui_event).is_err() {
                                    break;
                                }
                            }
                        }
                    });
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/compact: Context compaction requested. Next turn will compact.".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::ShowCost => {
                    // /cost 显示本次会话累计调用成本。
                    if let Some(cost) = state.context_usage.total_cost_usd {
                        state.history.push(adapter::UiHistoryItem::Notice(format!(
                            "/cost: Session cost — ${:.4} USD",
                            cost
                        )));
                    } else {
                        state.history.push(adapter::UiHistoryItem::Notice(
                            "/cost: No cost data available for this session.".into(),
                        ));
                    }
                    return Ok(false);
                }
                SlashOutcome::ShowUsage => {
                    // /usage 显示 token 用量信息。
                    let input = state.context_usage.last_input_tokens.unwrap_or(0);
                    let cached = state.context_usage.last_cached_tokens.unwrap_or(0);
                    let window = state
                        .context_usage
                        .window_size
                        .map(|w| format!(" / window: {} tokens", w))
                        .unwrap_or_default();
                    state.history.push(adapter::UiHistoryItem::Notice(format!(
                        "/usage: input: {} tokens, cached: {} tokens{}",
                        input, cached, window
                    )));
                    return Ok(false);
                }
                SlashOutcome::Handled { notice } => {
                    if let Some(n) = notice {
                        state.history.push(adapter::UiHistoryItem::Notice(n));
                    }
                    return Ok(false);
                }
                SlashOutcome::SetModel { model } => {
                    // /model <name> 切换模型。当前仅更新状态栏显示的模型名。
                    // TODO: 通过 config.toml 或 agent API 真正切换模型。
                    state.history.push(adapter::UiHistoryItem::Notice(format!(
                        "/model: Model changed to '{}' (display only — full model switching pending).",
                        model
                    )));
                    return Ok(false);
                }
                SlashOutcome::EnterPlan { task } => {
                    // v1.x:/plan <task> → Op::EnterPlanMode。
                    emit_op(
                        thread,
                        event_tx,
                        reflect_protocol::Op::EnterPlanMode { task },
                    );
                    return Ok(false);
                }
                SlashOutcome::ExitPlan => {
                    // v1.x:/exit-plan → Op::ExitPlanMode。
                    emit_op(thread, event_tx, reflect_protocol::Op::ExitPlanMode);
                    return Ok(false);
                }
                SlashOutcome::SetMode { mode } => {
                    emit_op(
                        thread,
                        event_tx,
                        reflect_protocol::Op::SetPermissionMode { mode },
                    );
                    return Ok(false);
                }
                SlashOutcome::CycleMode => {
                    emit_op(thread, event_tx, reflect_protocol::Op::CyclePermissionMode);
                    return Ok(false);
                }
                SlashOutcome::ShowStatus => {
                    // /status 显示完整状态信息。
                    let model = state.status_model.clone().unwrap_or_else(|| {
                        env::var("REFLECT_MODEL_DISPLAY")
                            .or_else(|_| env::var("OPENAI_MODEL"))
                            .or_else(|_| env::var("ANTHROPIC_MODEL"))
                            .unwrap_or_else(|_| "reflect".to_string())
                    });
                    let cwd = env::current_dir()
                        .ok()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| ".".to_string());
                    let branch = history_render::statusline::detect_git_branch()
                        .unwrap_or_else(|| "(none)".to_string());
                    let vim = if state.vim_enabled { "ON" } else { "OFF" };
                    let cost = state
                        .context_usage
                        .total_cost_usd
                        .map(|c| format!("${:.4}", c))
                        .unwrap_or_else(|| "N/A".to_string());
                    let tokens = state
                        .context_usage
                        .last_input_tokens
                        .map(|t| format!("{} tokens", t))
                        .unwrap_or_else(|| "N/A".to_string());
                    let context_window = state
                        .context_usage
                        .window_size
                        .map(|w| format!("{} tokens", w))
                        .unwrap_or_else(|| "N/A".to_string());

                    let status = format!(
                        "/status:\n  model: {}\n  cwd: {}\n  git: {}\n  mode: {}\n  vim: {}\n  cost: {}\n  tokens: {}\n  context window: {}\n  busy: {}",
                        model,
                        cwd,
                        branch,
                        state.permission_mode.as_str(),
                        vim,
                        cost,
                        tokens,
                        context_window,
                        state.busy
                    );
                    state.history.push(adapter::UiHistoryItem::Notice(status));
                    return Ok(false);
                }
                // ── v1.x Tier 4: 新增 slash 命令派发 ───────────────────
                SlashOutcome::SetEffort { effort } => {
                    let mirror = effort.to_mirror();
                    emit_op(
                        thread,
                        event_tx,
                        reflect_protocol::Op::SetEffort { effort: mirror },
                    );
                    state.history.push(adapter::UiHistoryItem::Notice(format!(
                        "/effort: reasoning effort set to {}.",
                        effort.as_str()
                    )));
                    return Ok(false);
                }
                SlashOutcome::EnterGoal { goal } => {
                    emit_op(
                        thread,
                        event_tx,
                        reflect_protocol::Op::EnterGoalMode {
                            goal,
                            verify_command: None,
                            token_budget: None,
                        },
                    );
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/goal: Goal mode entered.".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::ExitGoal => {
                    emit_op(thread, event_tx, reflect_protocol::Op::ExitGoalMode);
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/goal: Goal mode cleared.".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::Loop { interval_secs, command } => {
                    // 本地调度:不向 agent 发请求,只是把 loop 状态塞到 state,
                    // 由 tui/mod.rs 的事件循环每 tick 推进。
                    if interval_secs == 0 {
                        if let Some(prev) = state.loop_state.take() {
                            state.history.push(adapter::UiHistoryItem::Notice(format!(
                                "/loop: stopped (was: {} every {}s)",
                                prev.command, prev.interval_secs
                            )));
                        } else {
                            state.history.push(adapter::UiHistoryItem::Notice(
                                "/loop: no active loop to stop".into(),
                            ));
                        }
                    } else if let Some(prev) = state.loop_state.replace(LoopState {
                        interval_secs,
                        command: command.clone(),
                        next_fire: std::time::Instant::now()
                            + std::time::Duration::from_secs(interval_secs),
                    }) {
                        state.history.push(adapter::UiHistoryItem::Notice(format!(
                            "/loop: replaced previous ({} every {}s) with new ({} every {}s)",
                            prev.command, prev.interval_secs, command, interval_secs
                        )));
                    } else {
                        state.history.push(adapter::UiHistoryItem::Notice(format!(
                            "/loop: started — running `{}` every {}s. /loop stop to cancel.",
                            command, interval_secs
                        )));
                    }
                    return Ok(false);
                }
                SlashOutcome::Review { instructions } => {
                    let prompt = instructions.unwrap_or_else(|| {
                        "Please review my current changes and find issues.".to_string()
                    });
                    state
                        .history
                        .push(adapter::UiHistoryItem::User(prompt.clone()));
                    // fire-and-forget (同 emit_op): TUI 退出时 event_tx 被 drop，
                    // 任务通过 tx.send().is_err() 自然退出。
                    let thread = Arc::clone(thread);
                    let tx = event_tx.clone();
                    tokio::spawn(async move {
                        let sub = Submission::user_input(prompt);
                        let mut handle = thread.submit(sub).await;
                        while let Some(protocol_event) = handle.next().await {
                            if let Some(ui_event) = conversion::convert_event(protocol_event) {
                                if tx.send(ui_event).is_err() {
                                    break;
                                }
                            }
                        }
                    });
                    return Ok(false);
                }
                SlashOutcome::Init => {
                    let prompt = "Create an AGENTS.md file with project instructions for Reflect.";
                    state
                        .history
                        .push(adapter::UiHistoryItem::User(prompt.to_string()));
                    let thread = Arc::clone(thread);
                    let tx = event_tx.clone();
                    tokio::spawn(async move {
                        let sub = Submission::user_input(prompt.to_string());
                        let mut handle = thread.submit(sub).await;
                        while let Some(protocol_event) = handle.next().await {
                            if let Some(ui_event) = conversion::convert_event(protocol_event) {
                                if tx.send(ui_event).is_err() {
                                    break;
                                }
                            }
                        }
                    });
                    return Ok(false);
                }
                SlashOutcome::NewSession => {
                    // v1.x Tier 4.5:打开 session_overlay(alt-screen,复用 TasksPager 渲染)。
                    // 真实 new 流程走 Ops.SessionConfigured → 状态清理;这里只打开 picker。
                    open_session_overlay(state);
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/new: Open session overlay (use Ctrl+C to cancel).".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::Archive => {
                    // v1.x Tier 4.5:打开 session_overlay;archive 操作作用于选中的 session。
                    open_session_overlay(state);
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/archive: Open session overlay. Archive semantics pending.".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::Delete => {
                    // v1.x Tier 4.5:打开 session_overlay;delete 操作作用于选中的 session。
                    open_session_overlay(state);
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/delete: Open session overlay. Delete semantics pending.".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::Resume { id } => {
                    // v1.x Tier 4.5:打开 session_overlay;`/resume [id]` 时用 id 高亮/选中。
                    seed_session_entries(state);
                    if let Some(target_id) = id.as_deref() {
                        // 选中和给定 id 匹配的 session(若有)。
                        if let Some(_idx) =
                            state.session_entries.iter().position(|s| s.id == target_id)
                        {
                            // pager 不需要 selected;通过推 Notice 反馈。
                            state.history.push(adapter::UiHistoryItem::Notice(format!(
                                "/resume: requested session '{target_id}'."
                            )));
                        } else {
                            state.history.push(adapter::UiHistoryItem::Notice(format!(
                                "/resume: session '{target_id}' not found in current entries (overlay opened)."
                            )));
                        }
                    }
                    open_session_overlay(state);
                    if id.is_none() {
                        state.history.push(adapter::UiHistoryItem::Notice(
                            "/resume: Open session overlay.".into(),
                        ));
                    }
                    return Ok(false);
                }
                SlashOutcome::Fork { name } => {
                    // v1.x Tier 5: `/fork`(无参数) → fork select overlay(从历史选 fork 点)
                    // `/fork <name>` → session overlay(保留旧行为)
                    if let Some(n) = name {
                        seed_session_entries(state);
                        open_session_overlay(state);
                        state.history.push(adapter::UiHistoryItem::Notice(format!(
                            "/fork: Open session overlay. Fork as '{n}' (apply pending)."
                        )));
                    } else {
                        open_fork_select(state);
                    }
                    return Ok(false);
                }
                SlashOutcome::Rename { name } => {
                    // v1.x fork 接线:`/rename <name>` 接通 reflect_rollout rename_session,
                    // 直接重命名当前 session(不再 apply pending)。无 name 时打开 overlay。
                    if let Some(n) = name {
                        match state.session_id {
                            Some(tid) => {
                                let base = reflect_rollout::path::default_base();
                                match reflect_rollout::index::rename_session(&base, tid, &n) {
                                    Ok(()) => state.history.push(adapter::UiHistoryItem::Notice(
                                        format!("/rename: renamed current session → '{n}'."),
                                    )),
                                    Err(e) => state.history.push(adapter::UiHistoryItem::Error(
                                        format!("/rename: failed: {e}"),
                                    )),
                                }
                            }
                            None => state.history.push(adapter::UiHistoryItem::Error(
                                "/rename: current session id unavailable.".into(),
                            )),
                        }
                        return Ok(false);
                    }
                    seed_session_entries(state);
                    open_session_overlay(state);
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/rename: Open session overlay.".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::Mention { path } => {
                    let text = match path {
                        Some(p) => format!("@{p} "),
                        None => "@".to_string(),
                    };
                    state.composer.handle_paste(text);
                    return Ok(false);
                }
                SlashOutcome::Copy { n } => {
                    copy_reply_n(n, state);
                    return Ok(false);
                }
                SlashOutcome::ToggleRaw { on } => {
                    let now = on.unwrap_or(!state.raw_output);
                    state.raw_output = now;
                    state.history.push(adapter::UiHistoryItem::Notice(format!(
                        "Raw output mode: {}",
                        if now { "ON" } else { "OFF" }
                    )));
                    return Ok(false);
                }
                SlashOutcome::ToggleThink => {
                    let now = !state.show_thinking;
                    state.show_thinking = now;
                    state.history.push(adapter::UiHistoryItem::Notice(format!(
                        "Thinking blocks: {}",
                        if now { "shown" } else { "hidden" }
                    )));
                    return Ok(false);
                }
                SlashOutcome::Theme { arg } => {
                    // v1.x Tier 4.5:`/theme` 或 `/theme picker` → 打开 theme picker overlay。
                    // `/theme ls` → 列出主题;`/theme <name>` → 打开 picker 并高亮该主题。
                    match arg.as_deref() {
                        None | Some("picker") => {
                            open_theme_picker(state);
                            state.history.push(adapter::UiHistoryItem::Notice(
                                "/theme: Open theme picker (Esc to close).".into(),
                            ));
                        }
                        Some("ls") => {
                            state.history.push(adapter::UiHistoryItem::Notice(
                                "/theme: Available themes: dark, light, no-color (open picker for selection)."
                                    .into(),
                            ));
                            open_theme_picker(state);
                        }
                        Some(name) => {
                            open_theme_picker(state);
                            // 把 picker.selected 对齐到匹配的主题。
                            if let Some(picker) = state.theme_picker.as_mut() {
                                if let Some(idx) = picker.themes.iter().position(|t| t.id == name) {
                                    picker.selected = idx;
                                }
                            }
                            state.history.push(adapter::UiHistoryItem::Notice(format!(
                                "/theme: Highlighted '{name}' in picker."
                            )));
                        }
                    }
                    return Ok(false);
                }
                SlashOutcome::OpenImagePicker { arg } => {
                    // v1.x Tier 4.5:`/image` 打开 picker;`/image ls` 列出 cwd 下的图片。
                    open_image_picker(state);
                    if let Some(a) = arg {
                        state.history.push(adapter::UiHistoryItem::Notice(format!(
                            "/image: Picker opened (arg: {a})."
                        )));
                    } else {
                        state.history.push(adapter::UiHistoryItem::Notice(
                            "/image: Picker opened (Esc to close, ↑↓ to navigate, Enter to attach)."
                                .into(),
                        ));
                    }
                    return Ok(false);
                }
                SlashOutcome::OpenKeymapPicker { arg: _ } => {
                    // v1.x Tier 4.5:`/keymap` 打开 keymap picker overlay。
                    open_keymap_picker(state);
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/keymap: Picker opened (Esc to close).".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::OpenStatuslinePicker { arg: _ } => {
                    // v1.x Tier 4.5:`/statusline` 打开 statusline picker overlay。
                    open_statusline_picker(state);
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/statusline: Picker opened (Esc to close, Tab section, Enter toggle)."
                            .into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::OpenCheckpoint => {
                    // v1.x `/checkpoint` 打开 checkpoint overlay。
                    open_checkpoint_overlay(state);
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/checkpoint: Overlay opened (Esc/q to close, c create, r rewind).".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::Personality { arg } => {
                    let msg = match arg.as_deref() {
                        None => {
                            "/personality: Open personality picker (not yet wired).".to_string()
                        }
                        Some(name) => format!("/personality: set to '{name}' (not yet wired)."),
                    };
                    state.history.push(adapter::UiHistoryItem::Notice(msg));
                    return Ok(false);
                }
                SlashOutcome::ShowVersion => {
                    show_version(state);
                    return Ok(false);
                }
                SlashOutcome::RunDoctor => {
                    run_doctor(state);
                    return Ok(false);
                }
                SlashOutcome::ShowHistory { n } => {
                    show_input_history(state, n.unwrap_or(10));
                    return Ok(false);
                }
                SlashOutcome::ShowFiles => {
                    show_git_files(state);
                    return Ok(false);
                }
                SlashOutcome::Branch { arg } => {
                    show_branch(state, arg.as_deref());
                    return Ok(false);
                }
                SlashOutcome::Skills { arg } => {
                    match arg.as_deref() {
                        None | Some("ls") | Some("hub") => {
                            state.skills_hub = Some(crate::picker::skills_hub::populate());
                            state.history.push(adapter::UiHistoryItem::Notice(
                                "/skills: Open skills hub (j/k navigate, Esc close).".into(),
                            ));
                        }
                        Some(other) => {
                            state.history.push(adapter::UiHistoryItem::Notice(format!(
                                "/skills: subcommand '{other}' not supported (try /skills ls)."
                            )));
                        }
                    }
                    return Ok(false);
                }
                SlashOutcome::ToggleIde => {
                    state.history.push(adapter::UiHistoryItem::Notice(
                        "/ide: IDE integration toggle (not yet wired).".into(),
                    ));
                    return Ok(false);
                }
                SlashOutcome::OpenRewindSelect => {
                    // v1.x `/rewind` 打开 rewind select overlay。
                    open_rewind_select(state);
                    return Ok(false);
                }
                SlashOutcome::OpenTraces => {
                    let lines = crate::picker::traces_overlay::load_recent_traces();
                    state.transcript = Some(TranscriptPager::new(lines));
                    return Ok(false);
                }
                SlashOutcome::OpenPlugin => {
                    let lines = crate::picker::plugin_overlay::load_plugins();
                    state.transcript = Some(TranscriptPager::new(lines));
                    return Ok(false);
                }
                SlashOutcome::OpenMcp => {
                    let lines = crate::picker::mcp_overlay::load_mcp_status();
                    state.transcript = Some(TranscriptPager::new(lines));
                    return Ok(false);
                }
                SlashOutcome::SubmitAsUser(text) => {
                    state
                        .history
                        .push(adapter::UiHistoryItem::User(text.clone()));
                    let thread = Arc::clone(thread);
                    let tx = event_tx.clone();
                    tokio::spawn(async move {
                        let sub = Submission::user_input(text);
                        let mut handle = thread.submit(sub).await;
                        while let Some(protocol_event) = handle.next().await {
                            if let Some(ui_event) = conversion::convert_event(protocol_event) {
                                if tx.send(ui_event).is_err() {
                                    break;
                                }
                            }
                        }
                    });
                }
            }
            Ok(false)
        }
        // v1.x Tier 2: composer 弹出的 slash 命令（用户在 / 弹窗选中或输入后按 Enter）。
        // 不经过文本提交路径,直接映射到 emit_op / 本地动作。
        ComposerAction::Command(cmd) => {
            let should_exit =
                dispatch_slash_command(cmd, state, thread, event_tx, history_tail, guard);
            Ok(should_exit)
        }
        ComposerAction::CommandWithArgs(cmd, args) => {
            dispatch_slash_command_with_args(cmd, args, state, thread, event_tx, history_tail);
            return Ok(false);
        }
        ComposerAction::None => Ok(false),
    }
}

/// 计算 live 区要渲染的行与高度。
///
/// 流式接线后，**已换行结束的行**会逐条流进 scrollback（见 `apply_event` 的
/// `drain_committed_to_lines`），live 区只显示**尚未提交**的部分：
///
/// - `state.live` 中最后一个 `\n` 之后的「正在输入的未完成行」。
/// - 若流式控制器因表格 holdback 仍保留若干稳定行（`has_live_tail()`），也一并
///   经 `tail_display_lines` 显示，避免进行中的表格被吞掉。
///
/// 这样 live 区与 scrollback 内容不重复，且始终能看到正在打字的当前行；回合结束
/// （`TurnCompleted` 把剩余全部提交）后 live 自然清空。
///
/// 返回 `(lines, height)`：`lines` 为最终要绘制的行（已含 `○ ` 流式标记），`height`
/// 是按当前宽度折行后的行数（供视口布局）。
fn compute_live_region(state: &mut UiState, width: u16) -> (Vec<Line<'static>>, u16) {
    // reasoning 流式：live 区实时显示累积中的思考（与 agent 文字流式对称）。
    // 思考期间通常还没有 agent 文字，所以优先级最高。整段落定时由 adapter 的
    // flush_thinking 写进 scrollback，live 区随之清空。
    if !state.live_thinking.is_empty() {
        let cell = crate::history_render::cells::thinking_cell(state.live_thinking.clone());
        let lines = cell.display_lines(width);
        let h = lines_height(&lines, width);
        return (lines, h);
    }

    // 表格 holdback 的稳定行（罕见）：优先用引擎 tail。
    if let Some(stream) = state.stream.as_mut()
        && stream.has_live_tail()
    {
        let tail = stream.tail_display_lines(state.last_stream_width);
        if !tail.is_empty() {
            // tail 已含 `• ` 前缀，直接返回（不再加 `○ `）。
            let h = lines_height(&tail, width);
            return (tail, h);
        }
    }

    // 一般路径：显示 `state.live` 中尚未提交进 scrollback 的部分。
    // `committed_live_len` 由 adapter 在每次 drain 后推进（已换行的前缀已流进滚屏）。
    // 先对原文切片（committed_live_len 按 live 原文计），再 sanitize，保证下标口径一致。
    // 直接赋值 `state.live`（无流式增量）的场景下 committed_live_len==0，整段都显示。
    if state.live.is_empty() {
        return (Vec::new(), 0);
    }
    let bound = state.committed_live_len.min(state.live.len());
    let uncommitted_raw = &state.live[bound..];
    let display = history_render::sanitize_agent_display_text(uncommitted_raw);
    let display = display.trim_end_matches('\n');
    if display.is_empty() {
        return (Vec::new(), 0);
    }
    let mut live_text = render_markdown_text(display);
    let streaming_marker = Line::from(Span::styled("○ ", Style::default().fg(Color::DarkGray)));
    if let Some(first) = live_text.lines.first_mut() {
        let mut spans = Vec::with_capacity(first.spans.len() + 1);
        spans.push(streaming_marker.spans[0].clone());
        spans.append(&mut first.spans);
        first.spans = spans;
    } else {
        live_text.lines.push(streaming_marker);
    }
    let h = lines_height(&live_text.lines.to_vec(), width);
    (live_text.lines, h)
}

/// 按 `width` 折行估算一组行占用的屏幕行数（与 `live_region_height` 同口径）。
fn lines_height(lines: &[Line<'_>], width: u16) -> u16 {
    let wrap_w = usize::from(width.max(1));
    let mut rows: u16 = 0;
    for line in lines {
        let w = line.width().max(1);
        let line_rows = w.div_ceil(wrap_w);
        rows = rows.saturating_add(u16::try_from(line_rows).unwrap_or(u16::MAX).max(1));
    }
    rows.max(if lines.is_empty() { 0 } else { 1 })
}

fn draw(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    screen_height: u16,
    live_lines: &[Line<'static>],
    live_h: u16,
) {
    // live_h 由调用方按行宽折行后算好传入；draw 不再二次按逻辑行数算，
    // 避免 live 折行后占用的 viewport 空间与 layout 分配的口径不一致，
    // 导致 live 区底部留白、composer 被推离底部。
    let area = frame.area();
    let width = area.width;

    let composer_h = state.composer.desired_height(width);
    // 两种审批条高度:approval_block 带边框需 5 行(3 行内容 + 上下边框),
    // plan_approval_prompt 无边框 3 行。二者互斥(plan 审批时 pending 为 None)。
    let approval_h: u16 = if state.pending_approval.is_some() {
        5 // approval_block: 标题 + 内容 + 按键 = 3 行内容 + 边框 2 行
    } else {
        0
    };
    let plan_approval_h: u16 = if state.plan_approval.is_some() {
        3 // plan_approval_prompt: 标题 + 来源 + 按键 = 3 行(无边框)
    } else {
        0
    };
    // HUD 行数(窄屏防御):按「屏幕总高度」判定,不是视口高度
    // (视口高度 = desired,通常很小,用它判定会让 hud_height 永远返回 0)。
    let status_h: u16 = history_render::statusline::hud_height(screen_height);

    // 布局顺序:live 流式区 → 审批提示区 → composer → HUD。审批提示条紧贴
    // composer 上方,既不会被 scrollback 淹没,也不遮挡正文。
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(live_h),
            Constraint::Length(plan_approval_h),
            Constraint::Length(approval_h),
            Constraint::Length(composer_h),
            Constraint::Length(status_h),
        ])
        .split(area);

    if !live_lines.is_empty() {
        // live 区内容由调用方预渲染:流式 tail 已含 `• ` 前缀(来自
        // StreamingAgentTailCell);整段 live 回退路径在此加 `○ ` 流式标记。
        let owned: Vec<Line<'static>> = live_lines.to_vec();
        // 启用 wrap：live_h 已按折行后行数分配空间，若不 wrap 长行会被截断，
        // 填不满 layout[0]；wrap 让长行在分配高度内真正折行填充。
        frame.render_widget_ref(
            Paragraph::new(owned).wrap(ratatui::widgets::Wrap { trim: false }),
            layout[0],
        );
    }

    // plan 审批提示优先,其次工具审批(二者不会同时出现,plan 审批时
    // pending_approval 必为 None)。
    if let Some(ref plan) = state.plan_approval {
        frame.render_widget_ref(adapter::plan_approval_prompt(plan), layout[1]);
    }
    if let Some(ref pending) = state.pending_approval {
        frame.render_widget_ref(adapter::approval_block(pending), layout[2]);
    }

    // Composer：不再手工铺 Black 底；ChatComposer 用 user_message_style()。
    let composer_area = layout[3];
    state.composer.render_ref(composer_area, frame.buffer_mut());
    if let Some((x, y)) = state.composer.cursor_pos(composer_area) {
        frame.set_cursor_position((x, y));
    }

    // HUD:status_h==0 不画(窄屏);>=1 画 identity,>=2 追加 metrics(同一 2 行区)。
    if status_h >= 1 {
        let mut hud_lines: Vec<Line<'static>> = Vec::with_capacity(status_h as usize);
        if let Some(ref status) = state.status_line {
            hud_lines.push(status.clone());
        } else {
            hud_lines.push(Line::from(""));
        }
        if status_h >= 2 {
            if let Some(ref metrics) = state.metrics_line {
                hud_lines.push(metrics.clone());
            }
        }
        frame.render_widget_ref(Paragraph::new(hud_lines), layout[4]);
    }

    // 居中 overlay(plan 弹窗 + picker 覆盖层)由 draw / draw_fullscreen 共用。
    render_centered_overlays(frame, state, area);
}

/// 渲染居中 overlay(plan_enter_request 弹窗 + 全部 picker 覆盖层)。
///
/// 与面积相关(base 上居中),draw 与 draw_fullscreen 共用,避免两份 picker
/// 渲染逻辑重复。
fn render_centered_overlays(frame: &mut Frame<'_>, state: &mut UiState, area: Rect) {
    // v1.x Plan mode:plan_enter_request(进入 plan 模式前的权限请求)保留居中弹窗。
    // plan_approval 已改为内联提示条,不再使用模态弹窗。
    if let Some(ref req) = state.plan_enter_request {
        let modal = plan_enter_request_block(req);
        let modal_area = centered_rect(70, 55, area);
        frame.render_widget_ref(ratatui::widgets::Clear, modal_area);
        frame.render_widget_ref(modal, modal_area);
    }

    // v1.x Tier 4.5:picker overlays(都复用 plan_approval_block 的居中布局模式)。
    if state.image_picker.is_some() {
        crate::picker::image_picker::draw(frame, state, area);
    }
    if state.keymap_picker.is_some() {
        crate::picker::keymap_picker::draw(frame, state, area);
    }
    if state.statusline_picker.is_some() {
        crate::picker::statusline_picker::draw(frame, state, area);
    }
    if state.theme_picker.is_some() {
        crate::picker::theme_picker::draw(frame, state, area);
    }
    // v1.x Tier 5: checkpoint overlay 居中覆盖层。
    if state.checkpoint_overlay.is_some() {
        let overlay = state.checkpoint_overlay.as_ref().unwrap();
        crate::picker::checkpoint_overlay::draw(overlay, frame, area);
    }
    // checkpoint rewind 确认 modal(红色警告,优先级最高)。
    if let Some(ref sha) = state.checkpoint_rewind_sha {
        crate::picker::checkpoint_overlay::draw_confirm(sha, frame, area);
    }

    // v1.x Tier 5: fork/rewind select overlay 居中覆盖层。
    if let Some(ref overlay) = state.fork_rewind {
        crate::picker::fork_rewind::draw(overlay, &state.history, frame, area);
    }

    // v1.x Tier 5: copy history overlay 居中覆盖层。
    if let Some(ref overlay) = state.copy_history {
        crate::picker::copy_history::draw(overlay, frame, area);
    }

    // v1.x Tier 5: skills hub overlay 居中覆盖层。
    if let Some(ref overlay) = state.skills_hub {
        crate::picker::skills_hub::draw(overlay, frame, area);
    }
}

/// 全屏模式渲染：整个屏幕是 ratatui buffer，history 区铺满、live/审批/composer/
/// status 贴底。
///
/// 与 `draw`（底部 inline viewport）不同，历史不从原生 scrollback 读取，而是从
/// `state.history` 一次性渲染进全屏区，滚动由 `history_scroll_offset` 控制。全屏
/// 模式无 scrollback 注入路径，因此 `StreamedAgent`/`StreamedThinking` 也在此渲染。
fn draw_fullscreen(
    frame: &mut Frame<'_>,
    state: &mut UiState,
    screen_height: u16,
    live_lines: &[Line<'static>],
    live_h: u16,
) {
    let area = frame.area();
    let width = area.width;

    let composer_h = state.composer.desired_height(width);
    // 两种审批条高度：approval_block 带边框需 5 行，plan_approval_prompt 无边框 3 行。
    let approval_h: u16 = if state.pending_approval.is_some() { 5 } else { 0 };
    let plan_approval_h: u16 = if state.plan_approval.is_some() { 3 } else { 0 };
    let status_h: u16 = history_render::statusline::hud_height(screen_height);

    // 历史区吃掉所有剩余空间（Min(0)），其余区域贴底，与 draw 的底部锚定一致。
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(live_h),
            Constraint::Length(plan_approval_h),
            Constraint::Length(approval_h),
            Constraint::Length(composer_h),
            Constraint::Length(status_h),
        ])
        .split(area);

    // history 区：全屏模式没有 scrollback 注入路径，Streamed 变体也完整渲染，
    // 与 inline 模式（history_item_to_lines 对流式变体早返回空）不同。
    let mut all_lines: Vec<Line<'static>> = Vec::new();
    for item in &state.history {
        all_lines.extend(history_render::fullscreen_item_to_lines(
            item, width, &state.cwd,
        ));
    }
    // 滚动：history_scroll_offset 表示距底部的行数（0 = 贴底，新消息可见）。
    // Paragraph.scroll 的 y 作用于折行后行，故总行数用折行估算（与 lines_height 同口径）。
    let wrap_total = lines_height(&all_lines, width);
    let scroll_rows = wrap_total
        .saturating_sub(layout[0].height)
        .saturating_sub(state.history_scroll_offset.min(wrap_total));
    frame.render_widget_ref(
        Paragraph::new(all_lines)
            .scroll((scroll_rows, 0))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        layout[0],
    );

    // ── 下方与 draw 相同的 live / 审批 / composer / HUD 渲染 ──
    if !live_lines.is_empty() {
        let owned: Vec<Line<'static>> = live_lines.to_vec();
        frame.render_widget_ref(
            Paragraph::new(owned).wrap(ratatui::widgets::Wrap { trim: false }),
            layout[1],
        );
    }
    if let Some(ref plan) = state.plan_approval {
        frame.render_widget_ref(adapter::plan_approval_prompt(plan), layout[2]);
    }
    if let Some(ref pending) = state.pending_approval {
        frame.render_widget_ref(adapter::approval_block(pending), layout[3]);
    }
    let composer_area = layout[4];
    state.composer.render_ref(composer_area, frame.buffer_mut());
    if let Some((x, y)) = state.composer.cursor_pos(composer_area) {
        frame.set_cursor_position((x, y));
    }
    if status_h >= 1 {
        let mut hud_lines: Vec<Line<'static>> = Vec::with_capacity(status_h as usize);
        if let Some(ref status) = state.status_line {
            hud_lines.push(status.clone());
        } else {
            hud_lines.push(Line::from(""));
        }
        if status_h >= 2 {
            if let Some(ref metrics) = state.metrics_line {
                hud_lines.push(metrics.clone());
            }
        }
        frame.render_widget_ref(Paragraph::new(hud_lines), layout[5]);
    }

    // 居中 overlay 与 draw 共用。
    render_centered_overlays(frame, state, area);
}

/// v1.x Plan mode：进入 plan 模式前的确认弹窗 widget。
///
/// 标题 `Enter Plan Mode`；body 解释 Plan mode 行为（只读 + bash Safe + 写入
/// `.reflect/plan/`）+ Task 摘要；底部两段键位 `[1]` 进入 / `[3]/[Esc]` 取消。
///
/// 设计要点：
/// - 与 plan_approval_block 互斥：plan_approval 优先。
/// - 弹窗尺寸 70%×55%（比 plan_approval 略小,因为内容是固定文案）。
fn plan_enter_request_block(
    req: &crate::events::PlanEnterRequestState,
) -> ratatui::widgets::Paragraph<'static> {
    use ratatui::style::Modifier;
    use ratatui::widgets::{Block, Borders};

    let title = Line::from(Span::styled(
        " Enter Plan Mode ",
        Style::default()
            .fg(Color::Yellow)
            .bg(Color::Black)
            .add_modifier(Modifier::BOLD),
    ));

    let intro = Line::from(vec![
        Span::styled(
            "The agent wants to plan before making any code changes.",
            Style::default().fg(Color::White),
        ),
    ]);
    let task_line = Line::from(vec![
        Span::styled(" Task: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            req.task.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let mut rules = Vec::new();
    rules.push(Line::from(Span::styled(
        "In Plan mode the agent can:",
        Style::default().fg(Color::DarkGray),
    )));
    rules.push(Line::from(vec![
        Span::styled("  - ", Style::default().fg(Color::Green)),
        Span::styled(
            "run read / grep / glob / listing (read-only tools)",
            Style::default().fg(Color::White),
        ),
    ]));
    rules.push(Line::from(vec![
        Span::styled("  - ", Style::default().fg(Color::Green)),
        Span::styled(
            "run safe shell commands (ls, git status, cargo check ...)",
            Style::default().fg(Color::White),
        ),
    ]));
    rules.push(Line::from(vec![
        Span::styled("  - ", Style::default().fg(Color::Green)),
        Span::styled(
            "write to .reflect/plan/ for the plan file",
            Style::default().fg(Color::White),
        ),
    ]));
    rules.push(Line::from(""));
    rules.push(Line::from(Span::styled(
        "Editing tools and risky shell commands are blocked.",
        Style::default().fg(Color::DarkGray),
    )));
    rules.push(Line::from(Span::styled(
        "After approving the plan you choose how to implement it.",
        Style::default().fg(Color::DarkGray),
    )));

    let enter = Line::from(vec![
        Span::styled(
            " [1] Enter plan mode ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "(switch to Plan mode, agent continues planning)",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let cancel = Line::from(vec![
        Span::styled(
            " [3] / [Esc] Cancel ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            "(stay in current mode, no plan)",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let mut all = vec![title, Line::from(""), intro, Line::from(""), task_line, Line::from("")];
    all.extend(rules);
    all.push(Line::from(""));
    all.push(enter);
    all.push(cancel);

    Paragraph::new(all)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false })
}

/// v1.x Plan mode：进入 plan 模式弹窗底部 hint 行数。
const fn plan_enter_footer_rows() -> u16 {
    // 标题(1) + 空白(1) + 简介(1) + 空白(1) + 任务(1) + 空白(1) +
    // rules(8) + blank(1) + enter(1) + cancel(1) = 17。
    17
}

/// 居中矩形(ratatui 官方 helper):`percent_x`/`percent_y` 为 0-100 占比。
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let pop_w = area.width.saturating_mul(percent_x) / 100;
    let pop_h = area.height.saturating_mul(percent_y) / 100;
    // 至少给弹窗留够最小尺寸,避免窄屏下挤成 0。
    let pop_w = pop_w.max(40).min(area.width);
    let pop_h = pop_h.max(8).min(area.height);
    let x = area.x + (area.width.saturating_sub(pop_w)) / 2;
    let y = area.y + (area.height.saturating_sub(pop_h)) / 2;
    Rect::new(x, y, pop_w, pop_h)
}

// ── 事件转换与内容格式化（外移子模块） ──
mod conversion;

#[cfg(test)]
mod draw_tests {
    use super::*;
    use crate::tui_core::custom_terminal::Terminal as ReflectTerminal;
    use crate::tui_core::test_backend::VT100Backend;
    use crate::viewport::{clear_for_viewport_change, prepare_bottom_viewport};
    use reflect_protocol::EventMsg;

    #[test]
    fn draw_does_not_panic() {
        let backend = VT100Backend::new(40, 6);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 40, 6));
        let mut state = UiState::default();
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
    }

    #[test]
    fn draw_renders_status_line() {
        // 用 ≥8 行的屏:hud_height(10)=1,identity 行正常渲染。
        let backend = VT100Backend::new(40, 10);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 40, 10));
        let mut state = UiState::default();
        state.status_line = Some(Line::from("model/cwd"));
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(
            text.contains("model/cwd"),
            "status line not rendered: {text}"
        );
    }

    #[test]
    fn draw_hud_hidden_on_narrow_screen() {
        // 窄屏(<8 行)防御:HUD 不渲染,避免挤压 composer。
        let backend = VT100Backend::new(40, 6);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 40, 6));
        let mut state = UiState::default();
        state.status_line = Some(Line::from("model/cwd"));
        state.metrics_line = Some(Line::from("metrics"));
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(
            !text.contains("model/cwd"),
            "HUD should be hidden on narrow screen (<8): {text}"
        );
        assert!(
            !text.contains("metrics"),
            "metrics line should be hidden on narrow screen: {text}"
        );
    }

    #[test]
    fn draw_renders_both_hud_lines_on_tall_screen() {
        // ≥12 行:双行 HUD(identity + metrics)。
        let backend = VT100Backend::new(50, 14);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 50, 14));
        let mut state = UiState::default();
        state.status_line = Some(Line::from("identity"));
        state.metrics_line = Some(Line::from("metrics-here"));
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(text.contains("identity"), "identity line: {text}");
        assert!(text.contains("metrics-here"), "metrics line: {text}");
    }

    #[test]
    fn draw_hud_uses_screen_height_not_viewport_height() {
        // 回归:主循环里 frame.area() 是视口(= desired,很小),不是屏幕高度。
        // hud_height 必须按「屏幕高度」判定,否则视口小 → hud_height=0 → HUD 永不显示。
        // 这里模拟:屏幕 80x24(够高),但视口只有 5 行(composer+status)。
        let backend = VT100Backend::new(80, 24);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        // 视口锚在屏底,只占 5 行 —— frame.area().height 会是 5(<8)。
        terminal.set_viewport_area(Rect::new(0, 24 - 5, 80, 5));
        let mut state = UiState::default();
        state.status_line = Some(Line::from("identity-line"));
        let screen_height: u16 = 24;
        terminal
            .draw(|frame| draw(frame, &mut state, screen_height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(
            text.contains("identity-line"),
            "HUD must render even when viewport is small (screen=24): {text}"
        );
    }

    #[test]
    fn draw_renders_live_region_multiline() {
        let backend = VT100Backend::new(60, 12);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 60, 12));
        let mut state = UiState::default();
        state.live = "line1\nline2\nline3".to_string();
        let (live_lines, live_h) = compute_live_region(&mut state, 60);
        assert!(live_lines.len() >= 3, "live lines should be >= 3");
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &live_lines, live_h))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(text.contains("line1"), "live region not rendered: {text}");
    }

    #[test]
    fn draw_marks_live_region_with_streaming_glyph() {
        // 老 TUI 样式:live 区前置 `○ `(流式),与 history 的 `● `(完整)区分。
        let backend = VT100Backend::new(60, 12);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 60, 12));
        let mut state = UiState::default();
        state.live = "streaming reply".to_string();
        let (live_lines, live_h) = compute_live_region(&mut state, 60);
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &live_lines, live_h))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(
            text.contains("○ "),
            "live region should start with streaming glyph ○: {text}"
        );
        assert!(text.contains("streaming reply"), "live content: {text}");
    }

    #[test]
    fn draw_fullscreen_renders_history_and_composer() {
        // 全屏模式:历史 + composer 一次性渲染进全屏 buffer,不依赖原生 scrollback,且
        // 内容少于屏幕时不产生 scrollback 空行(它们分散在屏幕剩余区域,而非插入终端历史)。
        let backend = VT100Backend::new(60, 20);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 60, 20));
        let mut state = UiState::default();
        state
            .history
            .push(adapter::UiHistoryItem::User("hello".into()));
        state.history.push(adapter::UiHistoryItem::Agent(
            "hi there 你好".into(),
        ));
        terminal
            .draw(|frame| draw_fullscreen(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(text.contains("hello"), "user history: {text}");
        assert!(text.contains("hi there"), "agent history: {text}");
    }

    #[test]
    fn legacy_markers_rewrite_history_prefixes() {
        // 历史 user/agent 经 history_item_to_lines 应得到老字形 ▶ / ●。
        let cwd = std::path::Path::new(".");
        let user = history_render::history_item_to_lines(
            &adapter::UiHistoryItem::User("hello".into()),
            60,
            cwd,
        );
        let user_text: String = user
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(user_text.contains("▶ "), "user marker ▶: {user_text}");

        let agent = history_render::history_item_to_lines(
            &adapter::UiHistoryItem::Agent("world".into()),
            60,
            cwd,
        );
        let agent_text: String = agent
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(agent_text.contains("● "), "agent marker ●: {agent_text}");
    }

    #[test]
    fn tool_call_history_item_renders_glyph_and_elapsed() {
        let cwd = std::path::Path::new(".");
        let lines = history_render::history_item_to_lines(
            &adapter::UiHistoryItem::ToolCall {
                name: "Bash".into(),
                output: "done".into(),
                error: false,
                elapsed_ms: 9,
                diff: None,
            },
            60,
            cwd,
        );
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(text.contains("✓ Bash"), "ok glyph: {text}");
        assert!(text.contains("(9ms)"), "elapsed: {text}");
    }

    /// 回归:文字与工具调用应按发生顺序交错进入 scrollback，而非割裂成两块。
    ///
    /// 旧行为:`AgentDelta` 只堆 live 区，工具即时落 history，最终文字作为一条
    /// trailing Agent 排在所有 tool 之后 → 屏上「上半工具 / 下半文字」。
    /// 新行为:每条 delta 让出的已稳定行立即 drain 成行（这里收集进 extra），
    /// 工具 begin 前再 flush 一次；history 顺序为 [Agent-text1, ToolCall, Agent-text2]。
    #[test]
    fn apply_event_interleaves_text_and_tools_chronologically() {
        use crate::adapter::{UiEvent, UiEventKind};

        let mut state = UiState::default();
        // 触发 TurnStarted，建立流式控制器。
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::TurnStarted {
                    turn_id: reflect_protocol::TurnId::default(),
                },
            },
        );

        // text1：两行完整（含换行）→ 第二条 delta 让出稳定行。
        let mut committed_text1 = Vec::new();
        committed_text1.extend(adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::AgentDelta("text1-line1\n".into()),
            },
        ));
        committed_text1.extend(adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::AgentDelta("text1-line2\n".into()),
            },
        ));
        let text1_str: String = committed_text1
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(
            text1_str.contains("text1-line1"),
            "text1 第一行应作为已稳定行流进 scrollback: {text1_str}"
        );

        // toolA begin：边界 flush 把 text1 收尾，然后记名。
        let flush_at_tool = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::ToolStarted {
                    call_id: "call-a".into(),
                    name: "Bash".into(),
                },
            },
        );
        // tool begin 不应再把 text1 重复（已在 delta 路径 drain）；允许为空。
        let _ = flush_at_tool;

        // toolA end：落一条 ToolCall 到 history。
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::ToolCompleted {
                    call_id: "call-a".into(),
                    output: "ok".into(),
                    error: false,
                    elapsed_ms: 5,
                    diff: None,
                },
            },
        );

        // text2：再一段文字。
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::AgentDelta("text2\n".into()),
            },
        );

        // TurnCompleted：把剩余（text2）落 scrollback。
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::TurnCompleted,
            },
        );

        // 断言 history 末尾顺序：ToolCall 必须排在 TurnCompleted 推入的内容之前，
        // 且不应出现「所有文字排在所有 tool 之后」的退化。这里 history 里 ToolCall
        // 应位于 Separator 之前；text 文本以 Agent 形式存在（供 transcript）。
        let tool_idx = state.history.iter().position(
            |h| matches!(h, adapter::UiHistoryItem::ToolCall { name, .. } if name == "Bash"),
        );
        assert!(tool_idx.is_some(), "应有一条 Bash ToolCall");
        let sep_idx = state
            .history
            .iter()
            .position(|h| matches!(h, adapter::UiHistoryItem::Separator { .. }));
        assert!(
            sep_idx.is_some() && sep_idx.unwrap() > tool_idx.unwrap(),
            "Separator 应在 ToolCall 之后（回合结束标记）"
        );
        // 关键回归：live 区在回合结束后应已清空，不再持有整段文字。
        assert!(
            state.live.is_empty(),
            "回合结束后 live 应清空，文字已全部流进 scrollback"
        );
        // 关键回归：history 里的文本片段按时间顺序排在 tool 之前 / 之间 / 之后，
        // 且都以 StreamedAgent 形式存在（**不**是 Agent —— 否则会与已流式滚屏内容双份）。
        let stream_idxs: Vec<usize> = state
            .history
            .iter()
            .enumerate()
            .filter_map(|(i, h)| matches!(h, adapter::UiHistoryItem::StreamedAgent(_)).then_some(i))
            .collect();
        let agent_idxs: Vec<usize> = state
            .history
            .iter()
            .enumerate()
            .filter_map(|(i, h)| matches!(h, adapter::UiHistoryItem::Agent(_)).then_some(i))
            .collect();
        assert!(
            stream_idxs.len() >= 2,
            "应有 ≥2 段 StreamedAgent（tool 前后各一段）: {stream_idxs:?}"
        );
        assert!(
            agent_idxs.is_empty(),
            "流式回合不应再走 Agent 变体（避免与滚屏双份）: {agent_idxs:?}"
        );
        // 第一段 StreamedAgent 必须在 tool 之前，第二段必须在 tool 之后。
        assert!(
            stream_idxs[0] < tool_idx.unwrap(),
            "tool 前的文字片段应排在 tool 之前: {stream_idxs:?}"
        );
        assert!(
            stream_idxs[1] > tool_idx.unwrap(),
            "tool 后的文字片段应排在 tool 之后: {stream_idxs:?}"
        );
    }

    #[test]
    fn viewport_shrink_clears_stale_live_pixels() {
        // 模拟 live 消失后视口收缩：若不 clear，旧 live 会留在 scrollback 上方形成双份。
        let width: u16 = 40;
        let height: u16 = 10;
        let backend = VT100Backend::new(width, height);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();

        // 大视口（含 live）
        let big = Rect::new(0, height - 5, width, 5);
        terminal.set_viewport_area(big);
        let mut state = UiState::default();
        state.live = "FINAL ANSWER: hello".into();
        state.status_line = Some(Line::from("status"));
        let (live_lines, live_h) = compute_live_region(&mut state, width);
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &live_lines, live_h))
            .unwrap();

        // 先 shrink（贴底），再插 history（对齐生产帧顺序 / Reflect）。
        state.live.clear();
        state
            .history
            .push(adapter::UiHistoryItem::Agent("FINAL ANSWER: hello".into()));
        let lines = history_render::history_item_to_lines(
            &state.history[0],
            width.saturating_sub(2),
            &state.cwd,
        );
        prepare_bottom_viewport(&mut terminal, width, height, 3).unwrap();
        insert_history_lines(&mut terminal, lines).unwrap();
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();

        let text = terminal.backend().vt100().screen().contents();
        // sanitize 后不应再出现 FINAL ANSWER 字样。
        assert!(
            !text.contains("FINAL ANSWER"),
            "FINAL ANSWER marker must be stripped: {text}"
        );
        let count = text.matches("hello").count();
        assert!(
            count >= 1,
            "agent reply body must appear after viewport shrink: {text}"
        );
    }

    #[test]
    fn turn_complete_does_not_leave_large_blank_gap_between_turns() {
        // user → 高 live → TurnComplete 插入短 agent → 下一轮 user，中间不应有大片空行。
        use crate::adapter::UiHistoryItem;

        let width: u16 = 48;
        let height: u16 = 20;
        let backend = VT100Backend::new(width, height);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();

        // 底部小视口起步，插入 user。
        prepare_bottom_viewport(&mut terminal, width, height, 4).unwrap();
        let mut state = UiState::default();
        state.history.push(UiHistoryItem::User("第一问".into()));
        let user_lines = history_render::history_item_to_lines(
            &state.history[0],
            width.saturating_sub(2),
            &state.cwd,
        );
        insert_history_lines(&mut terminal, user_lines).unwrap();

        // 模拟流式：抬高 live 视口。
        state.live = "line1\nline2\nline3\nline4\nline5\nFINAL ANSWER: 短答".into();
        let live_h = live_region_height(
            &history_render::sanitize_agent_display_text(&state.live),
            width,
        );
        let tall = 4u16.saturating_add(live_h).min(height);
        prepare_bottom_viewport(&mut terminal, width, height, tall).unwrap();
        let (live_lines, live_h) = compute_live_region(&mut state, width);
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &live_lines, live_h))
            .unwrap();

        // TurnComplete：清 live → 先 shrink 贴底 → 再插 agent。
        state.live.clear();
        state
            .history
            .push(UiHistoryItem::Agent("短答\n\nFINAL ANSWER: 短答".into()));
        let agent_lines = history_render::history_item_to_lines(
            &state.history[1],
            width.saturating_sub(2),
            &state.cwd,
        );
        prepare_bottom_viewport(&mut terminal, width, height, 4).unwrap();
        insert_history_lines(&mut terminal, agent_lines).unwrap();

        // 第二轮 user。
        state.history.push(UiHistoryItem::User("第二问".into()));
        let user2 = history_render::history_item_to_lines(
            &state.history[2],
            width.saturating_sub(2),
            &state.cwd,
        );
        insert_history_lines(&mut terminal, user2).unwrap();
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();

        let text = terminal.backend().vt100().screen().contents();
        assert!(text.contains("第一问"), "missing first user: {text}");
        assert!(text.contains("第二问"), "missing second user: {text}");
        assert!(!text.contains("FINAL ANSWER"), "marker leaked: {text}");
        assert_eq!(
            terminal.viewport_area.bottom(),
            height,
            "viewport must stay bottom-anchored"
        );

        // 统计「第一问」到「第二问」之间的连续空行。
        // HistoryCell 自身常在消息后留 1 行间距；允许 ≤2。大片空隙（原先 ~8+）不可接受。
        let max_blank_run = max_blank_run_between(&text, "第一问", "第二问");
        assert!(
            max_blank_run <= 2,
            "too many blank lines between turns ({max_blank_run}): {text}"
        );
    }

    #[test]
    fn long_agent_reply_does_not_split_with_large_blank_gap() {
        // 高 live → TurnComplete 中等正文：屏上「比如」与后续列表之间不应有大片空行。
        use crate::adapter::UiHistoryItem;

        let width: u16 = 48;
        let height: u16 = 24;
        let backend = VT100Backend::new(width, height);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();

        prepare_bottom_viewport(&mut terminal, width, height, 4).unwrap();
        let mut state = UiState::default();
        state
            .history
            .push(UiHistoryItem::User("你能为我做什么".into()));
        let user_lines = history_render::history_item_to_lines(
            &state.history[0],
            width.saturating_sub(2),
            &state.cwd,
        );
        insert_history_lines(&mut terminal, user_lines).unwrap();

        // 模拟高 live（接近占满屏）。
        let live = "引言一行\n二行\n三行\n四行\n五行\n六行\n七行\n八行\n九行\n十行";
        state.live = live.into();
        let live_h = live_region_height(live, width);
        let tall = 4u16.saturating_add(live_h).min(height);
        prepare_bottom_viewport(&mut terminal, width, height, tall).unwrap();
        let (live_lines, live_h) = compute_live_region(&mut state, width);
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &live_lines, live_h))
            .unwrap();

        let agent = "现在，你想让我帮你做什么？ 可以直接描述你的需求，比如：\n\n- \"帮我理解这个项目的架构\"\n- \"为 xx 功能写一个测试\"";
        state.live.clear();
        state.history.push(UiHistoryItem::Agent(agent.into()));
        let agent_lines = history_render::history_item_to_lines(
            &state.history[1],
            width.saturating_sub(2),
            &state.cwd,
        );
        prepare_bottom_viewport(&mut terminal, width, height, 4).unwrap();
        insert_history_lines(&mut terminal, agent_lines).unwrap();
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();

        let text = terminal.backend().vt100().screen().contents();
        assert!(text.contains("比如"), "missing lead-in: {text}");
        assert!(
            text.contains("帮我理解") || text.contains("架构"),
            "missing list body: {text}"
        );
        let max_blank_run = max_blank_run_between(&text, "比如", "帮我理解");
        // 列表前允许 markdown 空行；大片空洞（先 insert 再 shrink 时常见 8+）不可接受。
        assert!(
            max_blank_run <= 3,
            "agent reply split by blank gap ({max_blank_run}): {text}"
        );
        assert_eq!(terminal.viewport_area.bottom(), height);
    }

    /// 返回 `from` 与 `to` 子串之间最长的连续空行数；找不到则返回 0。
    fn max_blank_run_between(text: &str, from: &str, to: &str) -> usize {
        let Some(start) = text.find(from) else {
            return 0;
        };
        let after_from = start + from.len();
        let Some(rel_end) = text[after_from..].find(to) else {
            return 0;
        };
        let between = &text[after_from..after_from + rel_end];
        let mut max_run = 0usize;
        let mut run = 0usize;
        for line in between.lines() {
            if line.trim().is_empty() {
                run += 1;
                max_run = max_run.max(run);
            } else {
                run = 0;
            }
        }
        max_run
    }

    #[test]
    fn composer_typed_text_has_visible_fg() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let backend = VT100Backend::new(48, 8);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 48, 8));
        let mut state = UiState::default();
        // 禁用 paste-burst，逐字插入。
        state.composer =
            crate::tui_core::public_widgets::composer_input::ComposerInput::new_with_config(
                "Send a message…".into(),
                true,
            );
        for ch in "你好".chars() {
            let key = KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            };
            let _ = state.composer.input(key);
        }
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(
            text.contains('你') || text.contains("你好"),
            "typed CJK must be visible in composer buffer: {text}"
        );
    }

    #[test]
    fn render_pipeline_pushes_history_and_keeps_viewport() {
        use crate::adapter::UiHistoryItem;

        let width: u16 = 48;
        let height: u16 = 12;
        let backend = VT100Backend::new(width, height);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        let composer_h = 3u16;
        let viewport_h = composer_h + 1;
        let viewport_y = height - viewport_h;
        terminal.set_viewport_area(Rect::new(0, viewport_y, width, viewport_h));

        let mut state = UiState::default();
        state.history.push(UiHistoryItem::User("你好".to_string()));
        state
            .history
            .push(UiHistoryItem::Agent("你好！很高兴见到你。".to_string()));

        let wrap_width = width.saturating_sub(2);
        let mut pending: Vec<Line<'static>> = Vec::new();
        for item in &state.history {
            pending.extend(history_render::history_item_to_lines(
                item, wrap_width, &state.cwd,
            ));
        }
        insert_history_lines(&mut terminal, pending).expect("insert history");

        state.status_line = Some(Line::from("reflect test"));
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();

        let text = terminal.backend().vt100().screen().contents();
        assert!(
            text.contains("你好"),
            "user message should be in scrollback: {text}"
        );
        assert!(
            text.contains("很高兴见到你") || text.contains("高兴"),
            "agent reply should be in scrollback: {text}"
        );
    }

    #[test]
    fn event_mapping_turn_started() {
        let tid = reflect_protocol::TurnId::default();
        let ev = reflect_protocol::Event::new(
            "id-1",
            EventMsg::TurnStarted(reflect_protocol::TurnStartedEvent {
                turn_id: tid,
                user_message_id: None,
            }),
        );
        let ui = conversion::convert_event(ev).unwrap();
        assert!(matches!(
            ui.kind,
            UiEventKind::TurnStarted { turn_id } if turn_id == tid
        ));
    }

    #[test]
    fn event_mapping_agent_delta() {
        let ev = reflect_protocol::Event::new(
            "id-1",
            EventMsg::AgentMessageDelta(reflect_protocol::AgentMessageDelta {
                delta: "hello".into(),
            }),
        );
        let ui = conversion::convert_event(ev).unwrap();
        assert_eq!(ui.kind, UiEventKind::AgentDelta("hello".into()));
    }

    #[test]
    fn session_configured_sets_context_window() {
        // SessionConfigured(走 subscribe_session)携带 context_window_size。
        let ev = reflect_protocol::Event::new(
            "id-sc",
            EventMsg::SessionConfigured(reflect_protocol::SessionConfiguredEvent::new(
                "reflect".to_string(),
                "openai".to_string(),
            )),
        );
        let ui = conversion::convert_event(ev).unwrap();
        assert!(matches!(
            ui.kind,
            UiEventKind::ContextWindowConfigured {
                window_size: _,
                model: _
            }
        ));
        let mut state = UiState::default();
        let _ = adapter::apply_event(&mut state, ui);
        assert_eq!(
            state.context_usage.window_size, None,
            "default new() → None"
        );
    }

    #[test]
    fn session_configured_with_window_sets_state() {
        let mut sc =
            reflect_protocol::SessionConfiguredEvent::new("m".to_string(), "p".to_string());
        sc.context_window_size = Some(128_000);
        let ev = reflect_protocol::Event::new("id-sc2", EventMsg::SessionConfigured(sc));
        let ui = conversion::convert_event(ev).unwrap();
        let mut state = UiState::default();
        let _ = adapter::apply_event(&mut state, ui);
        assert_eq!(state.context_usage.window_size, Some(128_000));
    }

    #[test]
    fn session_configured_sets_status_model() {
        // SessionConfigured 携带的 model 名应写入 state.status_model,
        // 供状态栏 /status 显示(覆盖启动期从 config 读的初值)。
        let ev = reflect_protocol::Event::new(
            "id-sc-model",
            EventMsg::SessionConfigured(reflect_protocol::SessionConfiguredEvent::new(
                "minimax/minimax-m3".to_string(),
                "minimax".to_string(),
            )),
        );
        let ui = conversion::convert_event(ev).unwrap();
        let mut state = UiState::default();
        state.status_model = Some("old-model".into()); // 模拟启动期的初值
        let _ = adapter::apply_event(&mut state, ui);
        assert_eq!(
            state.status_model.as_deref(),
            Some("minimax/minimax-m3"),
            "SessionConfigured 应覆盖 status_model"
        );
    }

    #[test]
    fn token_count_sets_usage() {
        let tc = reflect_protocol::TokenCountEvent {
            input_tokens: 5000,
            output_tokens: 200,
            cached_tokens: 1000,
            cache_write_tokens: 0,
            total_tokens: 5200,
            cost_usd: None,
            provider: None,
            credential_label: None,
        };
        let ev = reflect_protocol::Event::new("id-tc", EventMsg::TokenCount(tc));
        let ui = conversion::convert_event(ev).unwrap();
        let mut state = UiState::default();
        let _ = adapter::apply_event(&mut state, ui);
        assert_eq!(state.context_usage.last_input_tokens, Some(5000));
        assert_eq!(state.context_usage.last_cached_tokens, Some(1000));
    }

    #[test]
    fn context_bar_renders_with_window_and_usage() {
        // 端到端:window + usage 都齐 → identity 行应包含 Context 进度条。
        let usage = history_render::statusline::ContextUsage {
            window_size: Some(100_000),
            last_input_tokens: Some(45_000),
            last_cached_tokens: Some(0),
            total_cost_usd: None,
        };
        let line = history_render::statusline::render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            false,
            &usage,
            false,
            reflect_protocol::PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("Context"), "context bar shown: {s}");
        assert!(s.contains("45%"), "45% usage: {s}");
    }

    #[test]
    fn slash_clear_and_help() {
        assert_eq!(
            history_render::dispatch_slash("/clear"),
            SlashOutcome::ConfirmClear
        );
        assert_eq!(history_render::dispatch_slash("/help"), SlashOutcome::Help);
        assert_eq!(history_render::dispatch_slash("/exit"), SlashOutcome::Exit);
    }

    #[test]
    fn slash_diff_opens_overlay() {
        assert_eq!(
            history_render::dispatch_slash("/diff"),
            SlashOutcome::OpenDiff
        );
    }

    #[test]
    fn slash_tasks_opens_overlay() {
        assert_eq!(
            history_render::dispatch_slash("/tasks"),
            SlashOutcome::OpenTasks
        );
        assert_eq!(
            history_render::dispatch_slash("/agents"),
            SlashOutcome::OpenTasks
        );
    }

    #[test]
    fn plan_step_event_updates_state() {
        let ev = reflect_protocol::Event::new(
            "id-ps",
            reflect_protocol::event_msg::EventMsg::PlanStep(reflect_protocol::PlanStepEvent {
                plan_id: Default::default(),
                index: 0,
                total: 2,
                status: reflect_protocol::PlanStepStatus::Done,
                title: Some("step one".into()),
            }),
        );
        let ui = conversion::convert_event(ev).unwrap();
        let mut state = UiState::default();
        let _ = adapter::apply_event(&mut state, ui);
        assert_eq!(state.plan_steps.len(), 1);
        assert_eq!(state.plan_steps[0].title, "step one");
        assert_eq!(
            state.plan_steps[0].status,
            crate::events::PlanStepStatus::Done
        );
    }

    #[test]
    fn collab_started_adds_agents() {
        let ev = reflect_protocol::Event::new(
            "id-cs",
            reflect_protocol::event_msg::EventMsg::CollabStarted(
                reflect_protocol::CollabStartedEvent {
                    id: "team1".into(),
                    participants: vec!["alice".into(), "bob".into()],
                    mode: "discuss".into(),
                },
            ),
        );
        let ui = conversion::convert_event(ev).unwrap();
        let mut state = UiState::default();
        let _ = adapter::apply_event(&mut state, ui);
        assert_eq!(state.agents.len(), 2, "two participants → two agents");
        assert!(
            state
                .agents
                .iter()
                .all(|a| a.status == crate::events::AgentStatus::Running),
            "collab started → Running"
        );
    }

    #[test]
    fn collab_finished_marks_agents_done() {
        let mut state = UiState::default();
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::CollabStarted {
                    id: "t1".into(),
                    participants: vec!["a".into()],
                },
            },
        );
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::CollabFinished {
                    id: "t1".into(),
                    outcome: "consensus".into(),
                    rounds: 2,
                },
            },
        );
        assert!(
            state
                .agents
                .iter()
                .all(|a| a.status == crate::events::AgentStatus::Done),
            "collab finished → Done"
        );
    }

    #[test]
    fn derive_diff_path_from_plus_plus_b() {
        let diff = "--- a/foo/bar.txt\n+++ b/foo/bar.txt\n@@ -1 +1 @@\n-old\n+new";
        let p = derive_diff_path(diff, 0);
        assert_eq!(p, std::path::PathBuf::from("foo/bar.txt"));
    }

    #[test]
    fn derive_diff_path_from_diff_git_header() {
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\nindex abc..def 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        let p = derive_diff_path(diff, 0);
        assert_eq!(p, std::path::PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn derive_diff_path_fallback_on_no_header() {
        let diff = "@@ -1 +1 @@\n-a\n+b";
        let p = derive_diff_path(diff, 3);
        assert_eq!(p, std::path::PathBuf::from("diff-3.txt"));
    }

    #[test]
    fn open_diff_overlay_collects_diffs_from_history() {
        let mut state = UiState::default();
        state.history.push(adapter::UiHistoryItem::ToolCall {
            name: "Edit".into(),
            output: "edited".into(),
            error: false,
            elapsed_ms: 1,
            diff: Some("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b".into()),
        });
        state
            .history
            .push(adapter::UiHistoryItem::Agent("done".into()));
        open_diff_overlay(&mut state);
        let pager = state.diff_overlay.expect("diff overlay opened");
        let s: String = pager
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(!s.is_empty(), "diff overlay 应有内容");
    }

    #[test]
    fn open_diff_overlay_empty_when_no_diffs() {
        let mut state = UiState::default();
        state
            .history
            .push(adapter::UiHistoryItem::Agent("hi".into()));
        open_diff_overlay(&mut state);
        let pager = state.diff_overlay.expect("diff overlay opened");
        let s: String = pager
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.clone())
            .collect();
        assert!(s.contains("no diffs"), "无 diff 时显示占位: {s}");
    }

    #[test]
    fn apply_event_signature_returns_flush_lines() {
        let mut state = UiState::default();
        let lines = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::Notice("hi".into()),
            },
        );
        assert!(lines.is_empty());
        assert_eq!(state.history.len(), 1);
    }

    #[test]
    fn tool_call_enters_history_with_name_from_begin() {
        // begin 携带 tool_name,end 只带 call_id:end 落定时用 begin 的名字补全。
        let mut state = UiState::default();
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::ToolStarted {
                    call_id: "c1".into(),
                    name: "Bash".into(),
                },
            },
        );
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::ToolCompleted {
                    call_id: "c1".into(),
                    output: "ok".into(),
                    error: false,
                    elapsed_ms: 7,
                    diff: None,
                },
            },
        );
        assert_eq!(state.history.len(), 1, "exactly one tool-call item");
        match &state.history[0] {
            adapter::UiHistoryItem::ToolCall {
                name, elapsed_ms, ..
            } => {
                assert_eq!(name, "Bash", "name resolved from begin event");
                assert_eq!(*elapsed_ms, 7);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    // ── v1.x Plan mode 回归测试 ──────────────────────────────────────────

    #[test]
    fn plan_ready_opens_approval_and_pushes_plan_to_history() {
        // PlanReady{plan_id, markdown} → history 含 Plan + plan_approval 打开。
        let mut state = UiState::default();
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::PlanReady {
                    plan_id: "pid-1".into(),
                    markdown: "# Plan\n1. step one".into(),
                    path: None,
                },
            },
        );
        assert!(state.plan_approval.is_some(), "plan_approval 应打开");
        let pa = state.plan_approval.as_ref().unwrap();
        assert_eq!(pa.plan_id, "pid-1");
        assert!(
            pa.task.contains("Plan"),
            "task 取 markdown 首行: {}",
            pa.task
        );
        // markdown 同时进对话流(让用户看到完整计划)。
        assert!(
            state
                .history
                .iter()
                .any(|h| matches!(h, adapter::UiHistoryItem::Plan(_))),
            "history 应含 Plan item"
        );
    }

    #[test]
    fn plan_approved_clears_approval_and_notices() {
        let mut state = UiState::default();
        state.plan_approval = Some(crate::events::PlanApprovalState::new(
            "pid-2".into(),
            "t".into(),
            "m".into(),
            None,
        ));
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::PlanApproved {
                    plan_id: "pid-2".into(),
                },
            },
        );
        assert!(state.plan_approval.is_none(), "approve 后弹窗应关闭");
        assert!(
            state
                .history
                .iter()
                .any(|h| matches!(h, adapter::UiHistoryItem::Notice(n) if n.contains("approved"))),
            "应有 approved Notice"
        );
    }

    #[test]
    fn plan_rejected_clears_approval_with_reason() {
        let mut state = UiState::default();
        state.plan_approval = Some(crate::events::PlanApprovalState::new(
            "pid-3".into(),
            "t".into(),
            "m".into(),
            None,
        ));
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::PlanRejected {
                    plan_id: "pid-3".into(),
                    reason: Some("needs more tests".into()),
                },
            },
        );
        assert!(state.plan_approval.is_none(), "reject 后弹窗应关闭");
        let notice_text: String = state
            .history
            .iter()
            .filter_map(|h| match h {
                adapter::UiHistoryItem::Notice(n) => Some(n.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            notice_text.contains("needs more tests"),
            "reject reason 应入流: {notice_text}"
        );
    }

    #[test]
    fn permission_mode_changed_updates_state() {
        // PermissionModeChanged{from,to} → state.permission_mode = to。
        let mut state = UiState::default();
        assert_eq!(
            state.permission_mode,
            reflect_protocol::PermissionMode::Auto
        );
        let _ = adapter::apply_event(
            &mut state,
            UiEvent {
                kind: UiEventKind::PermissionModeChanged {
                    from: reflect_protocol::PermissionMode::Auto,
                    to: reflect_protocol::PermissionMode::Plan,
                },
            },
        );
        assert_eq!(
            state.permission_mode,
            reflect_protocol::PermissionMode::Plan
        );
    }

    #[test]
    fn draw_renders_plan_approval_prompt() {
        // plan_approval 打开时,composer 上方应出现审批提示条(task 摘要 + 按键)。
        let backend = VT100Backend::new(80, 30);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 80, 30));
        let mut state = UiState::default();
        state.plan_approval = Some(crate::events::PlanApprovalState::new(
            "p1".into(),
            "refactor auth".into(),
            "## steps\n1. a\n2. b".into(),
            None,
        ));
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(text.contains("PLAN APPROVAL"), "标题应可见: {text}");
        assert!(text.contains("refactor auth"), "task 摘要: {text}");
        // 按键提示应可见。
        assert!(text.contains("[1] Auto"), "approve 选项: {text}");
        assert!(text.contains("[2] Manual"), "manual 选项: {text}");
        assert!(text.contains("[3]/[Esc] Revise"), "revise 选项: {text}");
    }

    #[test]
    fn draw_hides_plan_approval_when_none() {
        // 无 plan_approval 时不画审批提示条(屏上无 PLAN APPROVAL)。
        let backend = VT100Backend::new(60, 16);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 60, 16));
        let mut state = UiState::default();
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(
            !text.contains("PLAN APPROVAL"),
            "无 plan 时不应显示审批提示条: {text}"
        );
    }

    #[test]
    fn draw_renders_plan_enter_request_modal() {
        // plan_enter_request 打开时(无 plan_approval),应渲染 Enter Plan Mode 弹窗。
        // 屏高 40 让 70%×55% 弹窗(约 56×22)能完整容下 16 行内容 + 2 行边框。
        let backend = VT100Backend::new(100, 40);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 100, 40));
        let mut state = UiState::default();
        state.plan_enter_request = Some(crate::events::PlanEnterRequestState {
            plan_id: "preq-1".into(),
            task: "add login flow".into(),
        });
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        assert!(text.contains("Enter Plan Mode"), "标题应可见: {text}");
        assert!(text.contains("add login flow"), "task 摘要: {text}");
        assert!(
            text.contains("read / grep / glob"),
            "规则说明: {text}"
        );
        assert!(
            text.contains("[1] Enter plan mode"),
            "进入键位提示: {text}"
        );
        assert!(
            text.contains("[3] / [Esc] Cancel"),
            "取消键位提示: {text}"
        );
    }

    #[test]
    fn draw_plan_approval_prompt_is_three_lines() {
        // 审批提示条约 3 行:标题 / 来源 / 按键。viewport 高度应至少容纳。
        let markdown = (1..=100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let backend = VT100Backend::new(80, 30);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 80, 30));
        let mut state = UiState::default();
        state.plan_approval = Some(crate::events::PlanApprovalState::new(
            "p1".into(),
            "big plan".into(),
            markdown,
            None,
        ));
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
        let text = terminal.backend().vt100().screen().contents();
        // 长 plan 不参与提示条渲染(正文在 scrollback),提示条仍显示任务摘要和按键。
        assert!(text.contains("PLAN APPROVAL: big plan"), "标题可见: {text}");
        assert!(text.contains("[1] Auto"), "按键可见: {text}");
    }

    #[test]
    fn centered_rect_respects_bounds() {
        // 弹窗几何:居中 + 最小尺寸钳制,不溢出 area。
        let area = Rect::new(0, 0, 80, 24);
        let r = centered_rect(80, 60, area);
        assert_eq!(r.width, 64, "80% of 80");
        assert_eq!(r.height, 14, "60% of 24 ≈ 14");
        assert!(r.x > 0 && r.y > 0, "应居中(非贴左上角)");

        // 窄屏:钳到最小尺寸但不超出。
        let narrow = Rect::new(0, 0, 30, 6);
        let r2 = centered_rect(80, 60, narrow);
        assert!(r2.width <= narrow.width);
        assert!(r2.height <= narrow.height);
    }

    #[test]
    fn sync_plan_mode_visuals_sets_placeholder() {
        // plan 模式 → 占位符含 /exit-plan;auto → 默认占位符。
        let mut state = UiState::default();
        state.permission_mode = reflect_protocol::PermissionMode::Plan;
        sync_plan_mode_visuals(&mut state);
        // 占位符无法直接读取(ComposerInput 无 getter),但 draw 不应 panic,
        // 且状态切换后 composer 仍能正常渲染。这里用 draw 不 panic 间接验证。
        let backend = VT100Backend::new(40, 8);
        let mut terminal = ReflectTerminal::with_options(backend).unwrap();
        terminal.set_viewport_area(Rect::new(0, 0, 40, 8));
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();

        state.permission_mode = reflect_protocol::PermissionMode::Auto;
        sync_plan_mode_visuals(&mut state);
        terminal
            .draw(|frame| draw(frame, &mut state, frame.area().height, &[], 0))
            .unwrap();
    }
}
