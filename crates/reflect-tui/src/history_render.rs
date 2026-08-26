//! Reflect 侧历史渲染：走 Reflect `HistoryCell` 路径。
//!
//! 用 `UserHistoryCell` / `AgentMarkdownCell` / `PlainHistoryCell` 等生成
//! `display_lines`，再经 `insert_history_lines` 写入原生 scrollback，避免自制
//! 前缀与 live 区样式不一致导致的「双份」观感。

pub(crate) mod cells;
pub(crate) mod marker;
pub mod notify;
pub mod statusline;

use crate::adapter::UiHistoryItem;
use crate::tui_core::history_cell::{
    AgentMarkdownCell, HistoryCell, new_error_event, new_proposed_plan, new_user_prompt,
    new_warning_event,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::path::Path;
use std::sync::Arc;

/// Reflect/GAIA 协议收口标记；仅用于 TUI 展示剥离，不改持久化原文。
const FINAL_ANSWER_MARKER: &str = "FINAL ANSWER:";

/// 剥离 `FINAL ANSWER:` 协议标记，并在解说与答案相同时去重。
///
/// 模型常输出「解说 + FINAL ANSWER: 同文」，原样渲染会看起来像双份回复。
pub fn sanitize_agent_display_text(raw: &str) -> String {
    let Some(idx) = raw.rfind(FINAL_ANSWER_MARKER) else {
        return raw.to_string();
    };

    let before = raw[..idx].trim_end();
    let after = raw[idx + FINAL_ANSWER_MARKER.len()..].trim();

    if after.is_empty() {
        return before.to_string();
    }
    if before.is_empty() {
        return after.to_string();
    }

    // 解说已包含答案，或两者相同 → 只保留解说（去掉 marker 行）。
    if before == after || before.contains(after) {
        return before.to_string();
    }

    // 保留解说 + 答案正文，不显示 FINAL ANSWER: 字样。
    format!("{before}\n\n{after}")
}

/// 将一条 UiHistoryItem 转为 HistoryCell 并取 display_lines。
pub fn history_item_to_lines(item: &UiHistoryItem, width: u16, cwd: &Path) -> Vec<Line<'static>> {
    // StreamedAgent / StreamedThinking 的行已在流式过程中（逐行 / 整段 flush）插入
    // scrollback，这里直接返回空，避免把整段再渲染一次造成「双份」
    // （仅 transcript / copy overlay 会取用其原文）。
    if matches!(
        item,
        UiHistoryItem::StreamedAgent(_) | UiHistoryItem::StreamedThinking(_)
    ) {
        return Vec::new();
    }
    let cell: Box<dyn HistoryCell> = match item {
        UiHistoryItem::User(text) => Box::new(new_user_prompt(
            text.clone(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )),
        UiHistoryItem::Agent(text) => {
            let display = sanitize_agent_display_text(text);
            Box::new(AgentMarkdownCell::new_with_inline_visualizations(
                display, cwd, None,
            ))
        }
        // StreamedAgent / StreamedThinking 在函数入口处已提前 return Vec::new()，
        // 理论上不可达。使用 _ => panic 而非 unreachable! 以防将来重构时误删早期
        // 返回导致 panic。
        UiHistoryItem::StreamedAgent(_) | UiHistoryItem::StreamedThinking(_) => {
            panic!("StreamedAgent / StreamedThinking 不应到达此处")
        }
        UiHistoryItem::Thinking(text) => {
            Box::new(crate::history_render::cells::thinking_cell(text.clone()))
        }
        UiHistoryItem::ToolCall {
            name,
            output,
            error,
            elapsed_ms,
            diff: _,
        } => Box::new(crate::history_render::cells::tool_call_cell(
            name,
            output,
            *error,
            *elapsed_ms,
        )),
        UiHistoryItem::Separator {
            elapsed_secs,
            had_work,
        } => Box::new(crate::history_render::cells::separator_cell(
            *elapsed_secs,
            *had_work,
        )),
        UiHistoryItem::Plan(text) => Box::new(new_proposed_plan(text.clone(), cwd)),
        UiHistoryItem::Error(text) => Box::new(new_error_event(text.clone())),
        UiHistoryItem::Notice(text) => Box::new(new_warning_event(text.clone())),
        // Banner 由 `tui/mod.rs::run_async` 在启动时 push,内容已预渲染好,
        // 此处仅原样送出(避免重复绘制破坏渐变锚点)。
        UiHistoryItem::Banner(lines) => {
            let cell: Box<dyn HistoryCell> =
                Box::new(crate::history_render::cells::prebuilt_cell(lines.clone()));
            return marker::apply_legacy_markers(item, cell.display_lines(width));
        }
    };
    // 老 TUI 标记字形:把 Reflect 的 `› `/`• ` 改写成 `▶ `/`● `(彩色)。
    marker::apply_legacy_markers(item, cell.display_lines(width))
}

/// 全屏模式专用:与 [`history_item_to_lines`] 相同,但 `StreamedAgent` /
/// `StreamedThinking` 也渲染。
///
/// inline 模式靠流式逐行注入原生 scrollback,所以 `history_item_to_lines` 对这两
/// 个变体早返回空;全屏模式没有 scrollback 注入路径,历史须从 `state.history`
/// 一次性渲染,这两个变体必须输出完整内容。
pub fn fullscreen_item_to_lines(
    item: &UiHistoryItem,
    width: u16,
    cwd: &Path,
) -> Vec<Line<'static>> {
    if let UiHistoryItem::StreamedAgent(text) = item {
        let display = sanitize_agent_display_text(text);
        let cell: Box<dyn HistoryCell> = Box::new(
            AgentMarkdownCell::new_with_inline_visualizations(display, cwd, None),
        );
        return marker::apply_legacy_markers(item, cell.display_lines(width));
    }
    if let UiHistoryItem::StreamedThinking(text) = item {
        let cell: Box<dyn HistoryCell> =
            Box::new(crate::history_render::cells::thinking_cell(text.clone()));
        return marker::apply_legacy_markers(item, cell.display_lines(width));
    }
    history_item_to_lines(item, width, cwd)
}

/// 将整段会话历史转为 transcript overlay 用的 Arc cells。
pub fn history_to_cells(history: &[UiHistoryItem], cwd: &Path) -> Vec<Arc<dyn HistoryCell>> {
    history
        .iter()
        .map(|item| -> Arc<dyn HistoryCell> {
            match item {
                UiHistoryItem::User(text) => Arc::new(new_user_prompt(
                    text.clone(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )),
                UiHistoryItem::Agent(text) => {
                    let display = sanitize_agent_display_text(text);
                    Arc::new(AgentMarkdownCell::new_with_inline_visualizations(
                        display, cwd, None,
                    ))
                }
                // StreamedAgent 是流式逐行已入滚屏的片段：transcript / /copy overlay
                // 仍按完整 markdown 渲染（同一前缀风格），但不写到主屏滚屏（见
                // history_item_to_lines 的早返回）。
                UiHistoryItem::StreamedAgent(text) => {
                    let display = sanitize_agent_display_text(text);
                    Arc::new(AgentMarkdownCell::new_with_inline_visualizations(
                        display, cwd, None,
                    ))
                }
                UiHistoryItem::Thinking(text)
                | UiHistoryItem::StreamedThinking(text) => {
                    Arc::new(crate::history_render::cells::thinking_cell(text.clone()))
                }
                UiHistoryItem::ToolCall {
                    name,
                    output,
                    error,
                    elapsed_ms,
                    diff: _,
                } => Arc::new(crate::history_render::cells::tool_call_cell(
                    name,
                    output,
                    *error,
                    *elapsed_ms,
                )),
                UiHistoryItem::Separator {
                    elapsed_secs,
                    had_work,
                } => Arc::new(crate::history_render::cells::separator_cell(
                    *elapsed_secs,
                    *had_work,
                )),
                UiHistoryItem::Plan(text) => Arc::new(new_proposed_plan(text.clone(), cwd)),
                UiHistoryItem::Error(text) => Arc::new(new_error_event(text.clone())),
                UiHistoryItem::Notice(text) => Arc::new(new_warning_event(text.clone())),
                UiHistoryItem::Banner(lines) => {
                    Arc::new(crate::history_render::cells::prebuilt_cell(lines.clone()))
                }
            }
        })
        .collect()
}

/// 渲染单条历史（兼容旧 adapter 调用点）。
pub fn render_history_item(item: &UiHistoryItem, width: u16) -> Vec<Line<'static>> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    history_item_to_lines(item, width, &cwd)
}

/// Slash 命令本地处理结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashOutcome {
    /// 已处理，无需发给 agent。
    Handled { notice: Option<String> },
    /// 清屏 + 清空会话历史(需二次确认)。
    Clear,
    /// 二次确认:是否真的要清空 scrollback 和历史?
    ConfirmClear,
    /// 退出 TUI。
    Exit,
    /// 显示帮助。
    Help,
    /// 打开 diff 全屏 overlay。
    OpenDiff,
    /// 打开 tasks/agent overlay。
    OpenTasks,
    /// 切换 Vim 模式(由调用方对 composer 执行 toggle_vim_enabled,
    /// 本变体携带新状态供底部状态行回显)。
    ToggleVim { now_enabled: bool },
    /// 压缩上下文:发送 Op::Compact 到 agent thread,清空历史。
    Compact,
    /// 显示成本信息(由调用方从 state.context_usage.total_cost_usd 取数)。
    ShowCost,
    /// 显示用量信息(由调用方从 state.context_usage 取数)。
    ShowUsage,
    /// 切换到指定模型(通知 agent 更新 model 参数)。
    SetModel { model: String },
    /// v1.x Plan mode：请求进入 plan 模式(由调用方发 `Op::EnterPlanMode`)。
    EnterPlan { task: String },
    /// v1.x Plan mode：退出 plan 模式(发 `Op::ExitPlanMode`)。
    ExitPlan,
    /// v1.x：`/mode <name>` 直接设定权限模式(发 `Op::SetPermissionMode`)。
    SetMode {
        mode: reflect_protocol::PermissionMode,
    },
    /// v1.x：`/mode`(空参)/`/mode cycle`/Shift+Tab/Tab(空输入) 循环权限模式
    /// (发 `Op::CyclePermissionMode`)。
    CycleMode,
    /// v1.x `/effort low|medium|high`：设定推理深度（发 `Op::SetEffort`）。
    SetEffort { effort: ReasoningEffort },
    /// v1.x `/goal <text>`：进入目标模式（发 `Op::EnterGoalMode`）。
    EnterGoal { goal: String },
    /// v1.x `/goal clear`：退出目标模式（发 `Op::ExitGoalMode`）。
    ExitGoal,
    /// v0.4+ `/loop <secs> <cmd>`：在 TUI 内部以 interval 秒为周期重复提交 cmd。
    /// 调度是本地状态，不发 agent round-trip。`/loop stop` 取消。
    /// `interval_secs == 0` 表示取消当前 loop（语义同 stop）。
    Loop {
        interval_secs: u64,
        command: String,
    },
    /// v1.x `/review [instructions]`：注入代码审查 prompt（提交为 user msg）。
    Review { instructions: Option<String> },
    /// v1.x `/init`：注入 init prompt（生成 AGENTS.md）。
    Init,
    /// v1.x `/new`：开始新会话（清空 + 二次确认）。
    NewSession,
    /// v1.x `/archive`：归档当前会话并退出。
    Archive,
    /// v1.x `/delete`：删除当前会话（红色确认 + 退出）。
    Delete,
    /// v1.x `/resume [id]`：恢复会话。
    Resume { id: Option<String> },
    /// v1.x `/fork [name]`：分叉当前会话。
    Fork { name: Option<String> },
    /// v1.x `/rename [name]`：重命名当前会话。
    Rename { name: Option<String> },
    /// v1.x `/mention <path>`：在 composer 中插入 `@<path>`。
    Mention { path: Option<String> },
    /// v1.x `/copy [N]`：复制最后/第 N 条 agent 回复到剪贴板。
    Copy { n: Option<usize> },
    /// v1.x `/raw [on|off]`：切换 raw scrollback 模式（本地状态）。
    ToggleRaw { on: Option<bool> },
    /// v1.x `/think`：切换 thinking 块显示（本地状态）。
    ToggleThink,
    /// v1.x `/theme [name|cycle|picker]`：切换/选择主题。
    Theme { arg: Option<String> },
    /// v1.x `/personality [name]`：选择 personality。
    Personality { arg: Option<String> },
    /// v1.x `/version`：显示 TUI 版本 + git sha + rustc。
    ShowVersion,
    /// v1.x `/doctor`：环境自检。
    RunDoctor,
    /// v1.x `/history [N]`：显示最近 N 条提交记录。
    ShowHistory { n: Option<usize> },
    /// v1.x `/files`：`git ls-files` 前 50。
    ShowFiles,
    /// v1.x `/branch [list|<name>|create <name>]`：git 分支操作。
    Branch { arg: Option<String> },
    /// v1.x `/skills [ls|hub|show|enable|disable <name>]`。
    Skills { arg: Option<String> },
    /// v1.x `/ide`：切换 IDE 集成（占位 + 状态）。
    ToggleIde,
    /// v1.x Tier 4.5 `/image [ls|path]`：打开 image picker overlay。
    OpenImagePicker { arg: Option<String> },
    /// v1.x Tier 4.5 `/keymap [ls|picker|bind|unbind|reset]`：打开 keymap picker。
    OpenKeymapPicker { arg: Option<String> },
    /// v1.x Tier 4.5 `/statusline [ls|picker|set|reset]`：打开 statusline picker。
    OpenStatuslinePicker { arg: Option<String> },
    /// v1.x `/checkpoint`：打开 checkpoint overlay。
    OpenCheckpoint,
    /// v1.x `/rewind`：打开 rewind select overlay（选择历史 user prompt 回填）。
    OpenRewindSelect,
    /// v1.x `/traces`：打开 traces overlay（显示最近 span 事件）。
    OpenTraces,
    /// v1.x `/plugin`：打开 plugin overlay（显示已安装插件）。
    OpenPlugin,
    /// v1.x `/mcp`：打开 mcp overlay（显示 MCP 服务器状态）。
    OpenMcp,
    /// 显示完整状态信息。
    ShowStatus,
    /// 不是已知 slash，原样提交。
    SubmitAsUser(String),
}

/// v1.x：reasoning effort（映射到 `Op::SetEffort::effort`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
    /// 解析 `/effort <name>`，未知 → None。
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
    /// 映射到 `reflect_protocol::item::ReasoningEffortMirror`。
    pub fn to_mirror(self) -> reflect_protocol::item::ReasoningEffortMirror {
        use reflect_protocol::item::ReasoningEffortMirror as M;
        match self {
            Self::Low => M::Low,
            Self::Medium => M::Medium,
            Self::High => M::High,
        }
    }
}

/// Slash 命令名(不含前导 `/`)中,由 [`dispatch_slash`] 处理、但**未**登记进
/// `SlashCommand` 枚举(驱动 composer 弹窗/校验的那套)的命令集合。
///
/// 背景:TUI 有两套 slash 分发 —— composer 侧的 `SlashCommand` 枚举(新)与本
/// 模块的 `dispatch_slash`(旧,字符串匹配)。枚举里没有的命令(如 `/help`
/// `/mode` `/cost` `/exit-plan`)会被 composer 的 `validate_submission` 当成
/// "Unrecognized command" 拒绝,从而永远到不了本该处理它们的 `dispatch_slash`。
///
/// 这个谓词让 composer 把这些「旧命令」当作合法命令放行,以纯文本形式 submit,
/// 再由 [`dispatch_slash`] 实际执行。集合必须与 `dispatch_slash` 的 match 臂
/// 保持同步(只收录枚举里**没有**的命令名;两套都有的命令走枚举路径,不在此列)。
pub fn is_legacy_passthrough_slash(name: &str) -> bool {
    matches!(
        name,
        "help" | "?"
            | "tasks"
            | "agents"
            | "cost"
            | "exit-plan"
            | "exitplan"
            | "mode"
            | "effort"
            | "ref"
            | "rm-session"
    )
}

/// 旧式 slash 命令（由 `dispatch_slash` 处理,不在 `SlashCommand` 枚举里），
/// 这些命令需要在命令弹窗中展示,以便用户发现。
///
/// 仅列出枚举中真正缺失的命令 —— 枚举命令的别名（如 `/ref` → `/mention`,
/// `/rm-session` → `/delete`）不列出,因为标准命令已经在弹窗中。
///
/// 描述请保持简短 —— 它们会显示在弹窗的描述列。
pub fn legacy_popup_commands() -> &'static [(&'static str, &'static str)] {
    &[
        ("help", "show available commands and shortcuts"),
        ("mode", "cycle or set permission mode (auto/plan/prompt)"),
        ("cost", "show token usage and cost summary"),
        ("tasks", "open the task list panel"),
        ("effort", "set reasoning effort (low/medium/high)"),
        ("exit-plan", "exit plan mode"),
    ]
}

/// 解析 Reflect 兼容 slash（/help /clear /exit /compact 等）。
pub fn dispatch_slash(text: &str) -> SlashOutcome {
    let trimmed = text.trim();
    if !trimmed.starts_with('/') {
        return SlashOutcome::SubmitAsUser(text.to_string());
    }
    let cmd = trimmed
        .split_whitespace()
        .next()
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    match cmd.as_str() {
        "/help" | "/?" => SlashOutcome::Help,
        "/clear" => SlashOutcome::ConfirmClear,
        "/diff" => SlashOutcome::OpenDiff,
        "/tasks" | "/agents" => SlashOutcome::OpenTasks,
        "/exit" | "/quit" => SlashOutcome::Exit,
        // `/vim` 的真正副作用由 tui/mod.rs::handle_event 在拿到
        // ToggleVim 时调用 `composer.toggle_vim_enabled()` 完成。
        // 本函数无法直接访问 composer(避免在 history_render 引入对 TUI 状态的依赖),
        // 因此仅产出意图,真实翻转在调用方完成并把新状态回填 notice。
        "/vim" => SlashOutcome::ToggleVim {
            // 占位:实际值由 handle_event 填入;dispatch 仅作意图标识。
            now_enabled: false,
        },
        "/compact" => SlashOutcome::Compact,
        "/cost" => SlashOutcome::ShowCost,
        "/usage" => SlashOutcome::ShowUsage,
        // /model <name> 切换到指定模型。
        "/model" => {
            let args: Vec<&str> = trimmed.split_whitespace().skip(1).collect();
            let model = args.first().unwrap_or(&"").to_string();
            if model.is_empty() {
                return SlashOutcome::Handled {
                    notice: Some("/model: Missing model name. Usage: /model <model-name>".into()),
                };
            }
            SlashOutcome::SetModel { model }
        }
        // v1.x Plan mode:`/plan <task>` 请求进入 plan 模式;空 task → 用法提示。
        "/plan" => {
            let task: String = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string();
            if task.is_empty() {
                SlashOutcome::Handled {
                    notice: Some(
                        "/plan: usage: /plan <task description>  (e.g. /plan refactor auth module)"
                            .into(),
                    ),
                }
            } else {
                SlashOutcome::EnterPlan { task }
            }
        }
        // v1.x Plan mode:退出 plan 模式。
        "/exit-plan" | "/exitplan" => SlashOutcome::ExitPlan,
        // /mode：空参或 `cycle` → 循环；`<name>` → 直达。
        "/mode" => {
            let arg = trimmed
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_ascii_lowercase();
            if arg.is_empty() || arg == "cycle" {
                SlashOutcome::CycleMode
            } else {
                match parse_permission_mode(&arg) {
                    Some(mode) => SlashOutcome::SetMode { mode },
                    None => SlashOutcome::Handled {
                        notice: Some(format!(
                            "/mode: unknown mode '{arg}'. Try: auto / prompt / plan / accept_edits / bypass"
                        )),
                    },
                }
            }
        }
        "/status" => SlashOutcome::ShowStatus,

        // ── v1.x Tier 4: 高价值 Op / 状态命令 ──────────────────────────
        // /effort <low|medium|high>
        "/effort" => {
            let arg = trimmed
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_ascii_lowercase();
            if arg.is_empty() {
                return SlashOutcome::Handled {
                    notice: Some("/effort: usage: /effort <low|medium|high>".into()),
                };
            }
            match ReasoningEffort::parse(&arg) {
                Some(effort) => SlashOutcome::SetEffort { effort },
                None => SlashOutcome::Handled {
                    notice: Some(format!(
                        "/effort: unknown effort '{arg}'. Try: low / medium / high"
                    )),
                },
            }
        }
        // /goal <text>  /goal clear
        "/goal" => {
            let rest = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .unwrap_or("")
                .trim();
            if rest.is_empty() {
                return SlashOutcome::Handled {
                    notice: Some(
                        "/goal: usage: /goal <objective>  (e.g. /goal refactor auth module)".into(),
                    ),
                };
            }
            if rest.eq_ignore_ascii_case("clear") {
                SlashOutcome::ExitGoal
            } else {
                SlashOutcome::EnterGoal {
                    goal: rest.to_string(),
                }
            }
        }
        // /loop <secs> <cmd>  /loop stop  /loop
        // 调度是 TUI 本地状态——不向 agent 发请求。
        // 用法:
        //   /loop 30 /status         → 每 30s 自动执行 /status
        //   /loop 60 reply with pong → 每 60s 自动发 "reply with pong"
        //   /loop stop / /loop 0     → 停止当前 loop
        // interval 必须 >= 1s(防止 busy loop),否则返回用法提示。
        "/loop" => {
            let rest = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .unwrap_or("")
                .trim();
            if rest.is_empty() {
                return SlashOutcome::Handled {
                    notice: Some(
                        "/loop: usage: /loop <secs> <cmd>  (e.g. /loop 30 /status)  \
                         /loop stop to cancel"
                            .into(),
                    ),
                };
            }
            // 解析第一段为 interval(整数秒)
            let mut parts = rest.splitn(2, char::is_whitespace);
            let first = parts.next().unwrap_or("").trim();
            let rest_after = parts.next().unwrap_or("").trim();
            if first.eq_ignore_ascii_case("stop") || first == "0" {
                SlashOutcome::Loop {
                    interval_secs: 0,
                    command: String::new(),
                }
            } else {
                let secs: u64 = match first.parse() {
                    Ok(n) if n >= 1 => n,
                    _ => {
                        return SlashOutcome::Handled {
                            notice: Some(format!(
                                "/loop: invalid interval '{first}'. Use /loop <secs> <cmd> \
                                 (secs must be >= 1)"
                            )),
                        };
                    }
                };
                if rest_after.is_empty() {
                    return SlashOutcome::Handled {
                        notice: Some(
                            "/loop: missing command. Use /loop <secs> <cmd>".into(),
                        ),
                    };
                }
                SlashOutcome::Loop {
                    interval_secs: secs,
                    command: rest_after.to_string(),
                }
            }
        }
        // /review [instructions]
        "/review" => {
            let rest = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .unwrap_or("")
                .trim();
            if rest.is_empty() {
                SlashOutcome::Review { instructions: None }
            } else {
                SlashOutcome::Review {
                    instructions: Some(rest.to_string()),
                }
            }
        }
        "/init" => SlashOutcome::Init,
        "/new" => SlashOutcome::NewSession,
        "/archive" => SlashOutcome::Archive,
        "/delete" | "/rm-session" => SlashOutcome::Delete,
        // /resume [id]
        "/resume" => {
            let id = trimmed.split_whitespace().nth(1).map(str::to_string);
            SlashOutcome::Resume { id }
        }
        // /fork [name]
        "/fork" => {
            let name = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::Fork { name }
        }
        // /rename [name]
        "/rename" => {
            let name = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::Rename { name }
        }
        // /mention [path]  /ref [path]
        "/mention" | "/ref" => {
            let path = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::Mention { path }
        }
        // /copy [N]
        "/copy" => {
            let n = trimmed
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<usize>().ok());
            SlashOutcome::Copy { n }
        }
        // /raw [on|off]
        "/raw" => {
            let arg = trimmed
                .split_whitespace()
                .nth(1)
                .map(str::to_ascii_lowercase);
            let on = match arg.as_deref() {
                Some("on") | Some("enable") | Some("true") => Some(true),
                Some("off") | Some("disable") | Some("false") => Some(false),
                None => None, // toggle
                _ => {
                    return SlashOutcome::Handled {
                        notice: Some("/raw: usage: /raw [on|off] (omit to toggle)".into()),
                    };
                }
            };
            SlashOutcome::ToggleRaw { on }
        }
        // /think  /thinking
        "/think" | "/thinking" => SlashOutcome::ToggleThink,
        // /theme [name|cycle|picker]
        "/theme" => {
            let arg = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::Theme { arg }
        }
        // /personality [name]
        "/personality" => {
            let arg = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::Personality { arg }
        }
        // /version
        "/version" => SlashOutcome::ShowVersion,
        // /doctor
        "/doctor" => SlashOutcome::RunDoctor,
        // /history [N]
        "/history" => {
            let n = trimmed
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse::<usize>().ok());
            SlashOutcome::ShowHistory { n }
        }
        // /files
        "/files" => SlashOutcome::ShowFiles,
        // /branch [list|<name>|create <name>]
        "/branch" => {
            let arg = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::Branch { arg }
        }
        // /skills [ls|hub|show|enable|disable <name>]
        "/skills" => {
            let arg = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::Skills { arg }
        }
        // /ide
        "/ide" => SlashOutcome::ToggleIde,
        // /image [ls|path]  /img [ls|path]
        "/image" | "/img" => {
            let arg = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::OpenImagePicker { arg }
        }
        // /keymap [ls|picker|bind|unbind|reset]
        "/keymap" => {
            let arg = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::OpenKeymapPicker { arg }
        }
        // /statusline [ls|picker|set|reset]
        "/statusline" => {
            let arg = trimmed
                .splitn(2, char::is_whitespace)
                .nth(1)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            SlashOutcome::OpenStatuslinePicker { arg }
        }
        // /checkpoint — 打开 checkpoint overlay
        "/checkpoint" => SlashOutcome::OpenCheckpoint,
        // /rewind — 打开 rewind select overlay（选择历史 user prompt 回填）
        "/rewind" => SlashOutcome::OpenRewindSelect,
        // /traces — 打开 traces overlay（显示最近 span 事件）
        "/traces" => SlashOutcome::OpenTraces,
        // /plugin — 打开 plugin overlay（显示已安装插件）
        "/plugin" => SlashOutcome::OpenPlugin,
        // /mcp — 打开 mcp overlay（显示 MCP 服务器状态）
        "/mcp" => SlashOutcome::OpenMcp,

        _ => {
            // 未知 slash：仍作为用户消息提交，让 runtime / agent 处理。
            SlashOutcome::SubmitAsUser(text.to_string())
        }
    }
}

/// 解析 `/mode <name>` 的名称 → `PermissionMode`。
///
/// 接受 `as_str()` 形式（`auto` / `prompt` / `plan` / `accept_edits` / `bubble` / `bypass`）
/// 以及常见别名（`accept`、`accept-edits`、`yes`）。未知 → `None`（调用方提示用法）。
pub fn parse_permission_mode(name: &str) -> Option<reflect_protocol::PermissionMode> {
    use reflect_protocol::PermissionMode as P;
    match name.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(P::Auto),
        "prompt" => Some(P::Prompt),
        "deny" => Some(P::Deny),
        "plan" => Some(P::Plan),
        "accept_edits" | "accept-edits" | "accept" | "auto_edits" | "yes" => Some(P::AcceptEdits),
        "bubble" => Some(P::Bubble),
        "bypass" => Some(P::Bypass),
        _ => None,
    }
}

pub fn help_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "Reflect slash commands",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  /help         Show this help"),
        Line::from("  /clear        Clear scrollback + session history"),
        Line::from("  /status       Show TUI status"),
        Line::from("  /compact      Request context compaction"),
        Line::from("  /diff         Show diff overlay (tool patches)"),
        Line::from("  /tasks        Show tasks & agents overlay"),
        Line::from("  /plan <t>     Enter plan mode for a task"),
        Line::from("  /exit-plan    Leave plan mode"),
        Line::from("  /mode [n]     Cycle or set permission mode"),
        Line::from("  /effort <lvl> Set reasoning effort (low/medium/high)"),
        Line::from("  /goal <text>  Enter goal mode"),
        Line::from("  /goal clear   Leave goal mode"),
        Line::from("  /loop <s> <c> Repeat a command every <s> seconds"),
        Line::from("  /loop stop    Stop the current loop"),
        Line::from("  /review [p]   Review current changes (optional prompt)"),
        Line::from("  /init         Create AGENTS.md"),
        Line::from("  /new          Start new session (confirm)"),
        Line::from("  /archive      Archive session & exit"),
        Line::from("  /delete       Delete session (confirm)"),
        Line::from("  /resume [id]  Resume session"),
        Line::from("  /fork [name]  Fork current session"),
        Line::from("  /rename [n]   Rename current session"),
        Line::from("  /mention <p>  Insert @<path> in composer"),
        Line::from("  /copy [N]     Copy Nth (or last) agent reply"),
        Line::from("  /raw [on|off] Toggle raw scrollback mode"),
        Line::from("  /think        Toggle thinking blocks display"),
        Line::from("  /theme [n]    Switch theme (or picker)"),
        Line::from("  /personality  Set personality"),
        Line::from("  /version      Show TUI version + git sha"),
        Line::from("  /doctor       Environment self-check"),
        Line::from("  /history [N]  Show last N user messages"),
        Line::from("  /files        List tracked files (git)"),
        Line::from("  /branch [arg] List/create git branches"),
        Line::from("  /skills       List skills"),
        Line::from("  /ide          Toggle IDE integration"),
        Line::from("  /vim          Toggle vim mode"),
        Line::from("  /exit         Exit the TUI"),
        Line::from(Span::styled(
            "Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Ctrl+T        Transcript overlay (alt-screen pager)"),
        Line::from("  Shift+Tab     Cycle permission mode"),
        Line::from("  Ctrl+O        Copy last agent reply"),
        Line::from("  Ctrl+G        Open external editor"),
        Line::from("  Ctrl+L        Force redraw"),
        Line::from("  Ctrl+Shift+D  Open diff overlay"),
        Line::from("  Ctrl+C        Exit"),
        Line::from("  Enter         Submit  ·  Shift+Enter newline"),
    ]
}

#[cfg(test)]
mod sanitize_tests {
    use super::*;

    #[test]
    fn sanitize_dedupes_identical_final_answer() {
        let raw = "你好！有什么我可以帮你的吗？😊\n\nFINAL ANSWER: 你好！有什么我可以帮你的吗？😊";
        let out = sanitize_agent_display_text(raw);
        assert_eq!(out, "你好！有什么我可以帮你的吗？😊");
        assert!(!out.contains("FINAL ANSWER"));
    }

    #[test]
    fn sanitize_keeps_commentary_and_distinct_answer() {
        let raw = "根据搜索结果，答案是 42。\n\nFINAL ANSWER: 42";
        let out = sanitize_agent_display_text(raw);
        assert!(!out.contains("FINAL ANSWER"));
        // before.contains(after) → 只用 before
        assert_eq!(out, "根据搜索结果，答案是 42。");
    }

    #[test]
    fn sanitize_keeps_both_when_answer_not_in_commentary() {
        let raw = "让我查一下。\n\nFINAL ANSWER: green, white";
        let out = sanitize_agent_display_text(raw);
        assert!(!out.contains("FINAL ANSWER"));
        assert!(out.contains("让我查一下"));
        assert!(out.contains("green, white"));
    }

    #[test]
    fn sanitize_only_final_answer_line() {
        let out = sanitize_agent_display_text("FINAL ANSWER: 42");
        assert_eq!(out, "42");
    }

    #[test]
    fn sanitize_passthrough_without_marker() {
        let raw = "普通回复，无标记";
        assert_eq!(sanitize_agent_display_text(raw), raw);
    }

    #[test]
    fn history_item_to_lines_banner_passes_through() {
        // banner 在 `tui/mod.rs` 启动期被 push 进 history,
        // `history_item_to_lines` 必须原样透传,不被 marker 改写。
        let cell = cells::banner_cell("0.4.0-test");
        let banner_lines = cell.display_lines(80);
        let item = UiHistoryItem::Banner(banner_lines.clone());
        let out = history_item_to_lines(&item, 80, Path::new("."));
        assert_eq!(
            out.len(),
            banner_lines.len(),
            "banner 应原样透传, 行数不变化"
        );
        // 透传内容应保留首字符渐变色(Color::Rgb),不被 marker 改写成纯文本。
        let first_span_fg = out[9].spans[0].style.fg;
        assert!(
            matches!(first_span_fg, Some(Color::Rgb(..))),
            "banner 渐变色应被透传, 实为 {first_span_fg:?}"
        );
    }

    #[test]
    fn dispatch_slash_vim_produces_toggle_intent() {
        // `/vim` 应产出 ToggleVim 变体,由调用方在 composer 上真实翻转。
        match dispatch_slash("/vim") {
            SlashOutcome::ToggleVim { .. } => {}
            other => panic!("/vim 应产出 ToggleVim, 实为 {other:?}"),
        }
        match dispatch_slash("/VIM") {
            SlashOutcome::ToggleVim { .. } => {}
            other => panic!("/VIM 大小写不敏感, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_compact_produces_compact() {
        match dispatch_slash("/compact") {
            SlashOutcome::Compact => {}
            other => panic!("/compact 应产出 Compact, 实为 {other:?}"),
        }
        match dispatch_slash("/COMPACT") {
            SlashOutcome::Compact => {}
            other => panic!("/COMPACT 大小写不敏感, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_cost_produces_show_cost() {
        match dispatch_slash("/cost") {
            SlashOutcome::ShowCost => {}
            other => panic!("/cost 应产出 ShowCost, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_usage_produces_show_usage() {
        match dispatch_slash("/usage") {
            SlashOutcome::ShowUsage => {}
            other => panic!("/usage 应产出 ShowUsage, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_model_produces_set_model() {
        match dispatch_slash("/model claude") {
            SlashOutcome::SetModel { model } => {
                assert_eq!(model, "claude");
            }
            other => panic!("/model claude 应产出 SetModel, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_mode_cycles_when_no_arg() {
        // /mode 空参 → CycleMode（等价 /mode cycle / Shift+Tab）。
        match dispatch_slash("/mode") {
            SlashOutcome::CycleMode => {}
            other => panic!("/mode 应产出 CycleMode, 实为 {other:?}"),
        }
        match dispatch_slash("/mode cycle") {
            SlashOutcome::CycleMode => {}
            other => panic!("/mode cycle 应产出 CycleMode, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_mode_sets_named_mode() {
        use reflect_protocol::PermissionMode;
        match dispatch_slash("/mode plan") {
            SlashOutcome::SetMode { mode } => assert_eq!(mode, PermissionMode::Plan),
            other => panic!("/mode plan 应产出 SetMode, 实为 {other:?}"),
        }
        // 别名 accept-edits / accept 也应解析。
        match dispatch_slash("/mode accept-edits") {
            SlashOutcome::SetMode { mode } => assert_eq!(mode, PermissionMode::AcceptEdits),
            other => panic!("/mode accept-edits 应产出 SetMode, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_mode_unknown_name_handled_with_notice() {
        match dispatch_slash("/mode wat") {
            SlashOutcome::Handled { notice } => {
                assert!(notice.unwrap().contains("unknown mode"));
            }
            other => panic!("未知 mode 名应产出 Handled, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_plan_requires_task() {
        match dispatch_slash("/plan") {
            SlashOutcome::Handled { notice } => {
                assert!(notice.unwrap().contains("usage"));
            }
            other => panic!("/plan 空参应产出 Handled, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_plan_carries_task() {
        match dispatch_slash("/plan refactor the auth module") {
            SlashOutcome::EnterPlan { task } => {
                assert_eq!(task, "refactor the auth module");
            }
            other => panic!("/plan <task> 应产出 EnterPlan, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_exit_plan() {
        assert_eq!(dispatch_slash("/exit-plan"), SlashOutcome::ExitPlan);
        assert_eq!(dispatch_slash("/exitplan"), SlashOutcome::ExitPlan);
    }

    #[test]
    fn dispatch_slash_status_produces_show_status() {
        match dispatch_slash("/status") {
            SlashOutcome::ShowStatus => {}
            other => panic!("/status 应产出 ShowStatus, 实为 {other:?}"),
        }
    }

    // ── v1.x Tier 4: 新增命令的派发测试 ──────────────────────────────

    #[test]
    fn dispatch_slash_effort_parses_level() {
        match dispatch_slash("/effort low") {
            SlashOutcome::SetEffort { effort } => {
                assert_eq!(effort, ReasoningEffort::Low);
                assert_eq!(effort.as_str(), "low");
            }
            other => panic!("/effort low 应产出 SetEffort, 实为 {other:?}"),
        }
        match dispatch_slash("/effort high") {
            SlashOutcome::SetEffort { effort } => assert_eq!(effort, ReasoningEffort::High),
            other => panic!("/effort high 应产出 SetEffort, 实为 {other:?}"),
        }
        // 未知值 → Handled
        match dispatch_slash("/effort ultra") {
            SlashOutcome::Handled { notice } => {
                assert!(notice.unwrap().contains("unknown"));
            }
            other => panic!("/effort ultra 应产出 Handled, 实为 {other:?}"),
        }
        // 空参 → Handled
        match dispatch_slash("/effort") {
            SlashOutcome::Handled { notice } => assert!(notice.unwrap().contains("usage")),
            other => panic!("/effort 应产出 Handled, 实为 {other:?}"),
        }
    }

    #[test]
    fn reasoning_effort_to_mirror() {
        assert_eq!(
            ReasoningEffort::Low.to_mirror(),
            reflect_protocol::item::ReasoningEffortMirror::Low
        );
        assert_eq!(
            ReasoningEffort::High.to_mirror(),
            reflect_protocol::item::ReasoningEffortMirror::High
        );
    }

    #[test]
    fn dispatch_slash_goal_enter_and_clear() {
        match dispatch_slash("/goal refactor auth") {
            SlashOutcome::EnterGoal { goal } => assert_eq!(goal, "refactor auth"),
            other => panic!("/goal 应产出 EnterGoal, 实为 {other:?}"),
        }
        match dispatch_slash("/goal clear") {
            SlashOutcome::ExitGoal => {}
            other => panic!("/goal clear 应产出 ExitGoal, 实为 {other:?}"),
        }
        // 大小写不敏感
        match dispatch_slash("/goal CLEAR") {
            SlashOutcome::ExitGoal => {}
            other => panic!("/goal CLEAR 应产出 ExitGoal, 实为 {other:?}"),
        }
        // 空参 → Handled
        match dispatch_slash("/goal") {
            SlashOutcome::Handled { notice } => assert!(notice.unwrap().contains("usage")),
            other => panic!("/goal 空参应产出 Handled, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_loop_parses_args_and_stop() {
        // 正常路径
        match dispatch_slash("/loop 30 /status") {
            SlashOutcome::Loop { interval_secs, command } => {
                assert_eq!(interval_secs, 30);
                assert_eq!(command, "/status");
            }
            other => panic!("/loop 30 /status 应产出 Loop, 实为 {other:?}"),
        }
        // stop 关键字
        match dispatch_slash("/loop stop") {
            SlashOutcome::Loop { interval_secs, command } => {
                assert_eq!(interval_secs, 0);
                assert!(command.is_empty());
            }
            other => panic!("/loop stop 应产出 Loop(0, _), 实为 {other:?}"),
        }
        // 大小写不敏感
        match dispatch_slash("/loop STOP") {
            SlashOutcome::Loop { interval_secs, .. } => assert_eq!(interval_secs, 0),
            other => panic!("/loop STOP 应大小写不敏感, 实为 {other:?}"),
        }
        // 0 也视为 stop
        match dispatch_slash("/loop 0 /anything") {
            SlashOutcome::Loop { interval_secs, .. } => assert_eq!(interval_secs, 0),
            other => panic!("/loop 0 应视作 stop, 实为 {other:?}"),
        }
        // 空参 → Handled
        match dispatch_slash("/loop") {
            SlashOutcome::Handled { notice } => assert!(notice.unwrap().contains("usage")),
            other => panic!("/loop 空参应产出 Handled, 实为 {other:?}"),
        }
        // interval < 1 或非数字 → Handled
        match dispatch_slash("/loop abc /status") {
            SlashOutcome::Handled { notice } => assert!(notice.unwrap().contains("invalid interval")),
            other => panic!("/loop abc 应 Handled, 实为 {other:?}"),
        }
        // interval 合法但缺 command → Handled
        match dispatch_slash("/loop 10") {
            SlashOutcome::Handled { notice } => assert!(notice.unwrap().contains("missing command")),
            other => panic!("/loop 10 应 Handled, 实为 {other:?}"),
        }
        // 多词 command 完整保留
        match dispatch_slash("/loop 5 reply with the single word PONG") {
            SlashOutcome::Loop { interval_secs, command } => {
                assert_eq!(interval_secs, 5);
                assert_eq!(command, "reply with the single word PONG");
            }
            other => panic!("/loop 5 reply… 应保留完整 cmd, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_review_carries_instructions() {
        match dispatch_slash("/review") {
            SlashOutcome::Review { instructions } => assert!(instructions.is_none()),
            other => panic!("/review 应产出 Review, 实为 {other:?}"),
        }
        match dispatch_slash("/review focus on security") {
            SlashOutcome::Review { instructions } => {
                assert_eq!(instructions.unwrap(), "focus on security");
            }
            other => panic!("/review 带参数应产出 Review, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_session_commands() {
        assert_eq!(dispatch_slash("/init"), SlashOutcome::Init);
        assert_eq!(dispatch_slash("/new"), SlashOutcome::NewSession);
        assert_eq!(dispatch_slash("/archive"), SlashOutcome::Archive);
        assert_eq!(dispatch_slash("/delete"), SlashOutcome::Delete);
        assert_eq!(dispatch_slash("/rm-session"), SlashOutcome::Delete);

        // /resume [id]
        match dispatch_slash("/resume") {
            SlashOutcome::Resume { id } => assert!(id.is_none()),
            other => panic!("/resume 应产出 Resume, 实为 {other:?}"),
        }
        match dispatch_slash("/resume abc-123") {
            SlashOutcome::Resume { id } => assert_eq!(id.unwrap(), "abc-123"),
            other => panic!("/resume id 应产出 Resume, 实为 {other:?}"),
        }

        // /fork [name]
        match dispatch_slash("/fork") {
            SlashOutcome::Fork { name } => assert!(name.is_none()),
            other => panic!("/fork 应产出 Fork, 实为 {other:?}"),
        }
        match dispatch_slash("/fork experiment") {
            SlashOutcome::Fork { name } => assert_eq!(name.unwrap(), "experiment"),
            other => panic!("/fork name 应产出 Fork, 实为 {other:?}"),
        }

        // /rename [name]
        match dispatch_slash("/rename session-1") {
            SlashOutcome::Rename { name } => assert_eq!(name.unwrap(), "session-1"),
            other => panic!("/rename 应产出 Rename, 实为 {other:?}"),
        }

        // /mention [path]
        match dispatch_slash("/mention src/lib.rs") {
            SlashOutcome::Mention { path } => assert_eq!(path.unwrap(), "src/lib.rs"),
            other => panic!("/mention 应产出 Mention, 实为 {other:?}"),
        }
        // /ref 别名
        match dispatch_slash("/ref foo.rs") {
            SlashOutcome::Mention { path } => assert_eq!(path.unwrap(), "foo.rs"),
            other => panic!("/ref 应产出 Mention, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_copy_carries_n() {
        match dispatch_slash("/copy") {
            SlashOutcome::Copy { n } => assert!(n.is_none()),
            other => panic!("/copy 应产出 Copy, 实为 {other:?}"),
        }
        match dispatch_slash("/copy 3") {
            SlashOutcome::Copy { n } => assert_eq!(n.unwrap(), 3),
            other => panic!("/copy 3 应产出 Copy, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_raw_parses_arg() {
        match dispatch_slash("/raw") {
            SlashOutcome::ToggleRaw { on } => assert!(on.is_none()),
            other => panic!("/raw 应产出 ToggleRaw(None), 实为 {other:?}"),
        }
        match dispatch_slash("/raw on") {
            SlashOutcome::ToggleRaw { on } => assert_eq!(on, Some(true)),
            other => panic!("/raw on 应产出 ToggleRaw(Some(true)), 实为 {other:?}"),
        }
        match dispatch_slash("/raw off") {
            SlashOutcome::ToggleRaw { on } => assert_eq!(on, Some(false)),
            other => panic!("/raw off 应产出 ToggleRaw(Some(false)), 实为 {other:?}"),
        }
        match dispatch_slash("/raw bogus") {
            SlashOutcome::Handled { notice } => assert!(notice.unwrap().contains("usage")),
            other => panic!("/raw bogus 应产出 Handled, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_toggles_and_info() {
        assert_eq!(dispatch_slash("/think"), SlashOutcome::ToggleThink);
        assert_eq!(dispatch_slash("/thinking"), SlashOutcome::ToggleThink);
        assert_eq!(dispatch_slash("/ide"), SlashOutcome::ToggleIde);
        assert_eq!(dispatch_slash("/version"), SlashOutcome::ShowVersion);
        assert_eq!(dispatch_slash("/doctor"), SlashOutcome::RunDoctor);
        assert_eq!(dispatch_slash("/files"), SlashOutcome::ShowFiles);
    }

    #[test]
    fn dispatch_slash_history_with_n() {
        match dispatch_slash("/history") {
            SlashOutcome::ShowHistory { n } => assert!(n.is_none()),
            other => panic!("/history 应产出 ShowHistory, 实为 {other:?}"),
        }
        match dispatch_slash("/history 20") {
            SlashOutcome::ShowHistory { n } => assert_eq!(n.unwrap(), 20),
            other => panic!("/history 20 应产出 ShowHistory, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_branch_with_arg() {
        match dispatch_slash("/branch") {
            SlashOutcome::Branch { arg } => assert!(arg.is_none()),
            other => panic!("/branch 应产出 Branch, 实为 {other:?}"),
        }
        match dispatch_slash("/branch list") {
            SlashOutcome::Branch { arg } => assert_eq!(arg.unwrap(), "list"),
            other => panic!("/branch list 应产出 Branch, 实为 {other:?}"),
        }
        match dispatch_slash("/branch create feature-x") {
            SlashOutcome::Branch { arg } => assert_eq!(arg.unwrap(), "create feature-x"),
            other => panic!("/branch create 应产出 Branch, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_theme_and_personality() {
        match dispatch_slash("/theme") {
            SlashOutcome::Theme { arg } => assert!(arg.is_none()),
            other => panic!("/theme 应产出 Theme, 实为 {other:?}"),
        }
        match dispatch_slash("/theme dark") {
            SlashOutcome::Theme { arg } => assert_eq!(arg.unwrap(), "dark"),
            other => panic!("/theme dark 应产出 Theme, 实为 {other:?}"),
        }
        match dispatch_slash("/personality friendly") {
            SlashOutcome::Personality { arg } => assert_eq!(arg.unwrap(), "friendly"),
            other => panic!("/personality 应产出 Personality, 实为 {other:?}"),
        }
    }

    #[test]
    fn dispatch_slash_skills_with_arg() {
        match dispatch_slash("/skills") {
            SlashOutcome::Skills { arg } => assert!(arg.is_none()),
            other => panic!("/skills 应产出 Skills, 实为 {other:?}"),
        }
        match dispatch_slash("/skills ls") {
            SlashOutcome::Skills { arg } => assert_eq!(arg.unwrap(), "ls"),
            other => panic!("/skills ls 应产出 Skills, 实为 {other:?}"),
        }
    }

    #[test]
    fn help_lines_covers_tier4_commands() {
        // /help 文本应包含 Tier 4 新增的命令名（避免遗漏）。
        let text: String = help_lines()
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.clone())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        for cmd in &[
            "/effort",
            "/goal",
            "/review",
            "/init",
            "/new",
            "/archive",
            "/delete",
            "/resume",
            "/fork",
            "/rename",
            "/mention",
            "/copy",
            "/raw",
            "/think",
            "/theme",
            "/personality",
            "/version",
            "/doctor",
            "/history",
            "/files",
            "/branch",
            "/skills",
            "/ide",
            "Ctrl+O",
            "Ctrl+G",
            "Ctrl+L",
            "Ctrl+Shift+D",
        ] {
            assert!(text.contains(cmd), "/help 应包含 '{cmd}', 实际:\n{text}");
        }
    }
}
