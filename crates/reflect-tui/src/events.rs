use crate::adapter::UiHistoryItem;
use crate::history_render::statusline::ContextUsage;
use crate::transcript_pager::TranscriptPager;
use crate::tui_core::clipboard_copy::ClipboardLease;
use crate::tui_core::history_cell::HistoryRenderMode;
use crate::tui_core::public_widgets::composer_input::ComposerInput;
use crate::tui_core::streaming::controller::StreamController;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::task::JoinSet;

/// 待用户确认的审批请求（对齐 Reflect approval overlay 的最小交互）。
///
/// `kind` 保留原始 `ApprovalKind`（Tool/Hook/Plan），让 `handle_key` 在
/// 用户按 y/n/a 时知道该回送 `Op::ToolApproval` 还是 `Op::HookApproval`；
/// `summary` 仅用于展示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    pub id: String,
    pub summary: String,
    pub kind: reflect_protocol::ApprovalKind,
}

/// v1.x Plan mode：agent 产出 plan markdown 后请求用户审批。
///
/// 由 `EventMsg::PlanReady` 填入 `UiState.plan_approval`，在 composer 上方
/// 显示内联审批提示条。用户按 `1/2/3/Esc` 后，调用方据此发
/// `Op::PlanApproval { id, choice }`。plan 正文在 scrollback 中完整展示，
/// 因此这里不需要保存滚动状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanApprovalState {
    /// 与 `PlanReadyEvent.plan_id` 配对；`Op::PlanApproval.id` 必须回填这个值。
    pub plan_id: String,
    /// 规划任务摘要（来自最早一条 `PlanRequest.task` 或 plan markdown 首行）。
    pub task: String,
    /// 完整 plan markdown（供 history 兜底与 transcript overlay 取用）。
    pub markdown: String,
    /// plan markdown 落盘路径（`<workspace>/.reflect/plan/<plan_id>.md`）。
    /// 让 plan 成为可引用的持久产物；core 写盘失败时为 `None`。
    pub path: Option<std::path::PathBuf>,
}

impl PlanApprovalState {
    /// 构造一个新的 plan approval 状态。
    pub fn new(
        plan_id: String,
        task: String,
        markdown: String,
        path: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            plan_id,
            task,
            markdown,
            path,
        }
    }
}

/// v1.x Plan mode：进入 plan 模式的确认弹窗状态。
///
/// 由 `EventMsg::PlanRequest` 填入 `UiState.plan_enter_request`，打开一个
/// 轻量确认条（「要为 <task> 进入 plan 模式吗？1=进入 3/Esc=取消」）。
/// 用户按 `1` → 发 `Op::PlanApproval { id, AutoMode }` 切到 Plan 模式；
/// 按 `3`/`Esc` → 发 `Revise` 取消（留在当前模式）。core 的
/// `spawn_plan_enter_waiter` 阻塞在对应的 oneshot 上，必须回送 choice 才解锁。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEnterRequestState {
    /// 与 `PlanRequestEvent` 携带的 `plan_id` 配对（core 生成、等待回送）。
    pub plan_id: String,
    /// 用户要规划的任务文本（`/plan <task>` 或 LLM EnterPlanModeTool 的参数）。
    pub task: String,
}

/// v0.4+ `/loop <secs> <cmd>` 的本地调度状态。
///
/// `/loop` 把一个 slash 命令或自由文本作为「每 N 秒自动提交一次」的循环挂到
/// TUI 主循环上（不向 agent 发额外请求；agent 只看到循环提交的 user input）。
/// 任一时刻最多一个 loop —— 再次 `/loop` 会替换（并通知旧的被替换）。`/loop
/// stop` 或 `/loop 0` 清空它。
///
/// 调度由主循环的每帧 tick 推进：到 `next_fire` 时刻就提交 `command`，并把
/// `next_fire` 推后一个 `interval_secs`。`interval_secs == 0` 的 LoopState 不
/// 会被创建（`/loop stop` 直接 `take()` 掉已有 loop）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopState {
    /// 循环间隔（秒）。>= 1。
    pub interval_secs: u64,
    /// 每次触发要提交的命令/文本（原样作为 user input 提交；可以是 `/status`
    /// 这样的 slash，也可以是普通文本）。
    pub command: String,
    /// 下一次触发的时刻。主循环对比 `Instant::now()` 判定是否到点。
    pub next_fire: Instant,
}

/// 稳定版 TUI 事件循环的顶级 UI 状态。
///
/// `composer` 替换了原来的纯 `String` 输入，提供了多行
/// 编辑、光标定位、斜杠命令弹出窗口和底部提示，与
/// 上游 Reflect TUI 体验保持一致。
pub struct UiState {
    pub history: Vec<UiHistoryItem>,
    /// 流式增量原文（与 StreamController 同步，供 live 区高度估算）。
    pub live: String,
    /// reasoning（LLM 思考）流式累积缓冲。
    ///
    /// 每个 `ThinkingDelta` 累积到这里，**不**直接 push history。live 区每帧
    /// 据 `live_thinking` 实时渲染思考过程；在边界（agent 正文 / 工具调用 /
    /// 回合结束）flush 成一条 `UiHistoryItem::Thinking` 落进 scrollback，
    /// 保证整段思考显示为一条消息，而非每个 delta 碎片化成独立 `… ` 行。
    pub live_thinking: String,
    /// Reflect 流式控制器：stable 行刷入 scrollback，tail 留在 viewport。
    pub(crate) stream: Option<StreamController>,
    /// 最近一次创建/调整 StreamController 时用的内容宽度（`终端宽度 - 2`）。
    ///
    /// 流式文本在 `display_lines(width)` 折行时必须与控制器渲染宽度一致，否则
    /// 行长错位。主循环在 `term_size` 变化时更新它，并 `set_width` 同步控制器。
    pub(crate) last_stream_width: u16,
    /// `state.live` 中**已流式提交进 scrollback** 的字节长度。
    ///
    /// live 区只显示 `state.live[committed_live_len..]`（尚未提交的部分，通常是
    /// 「正在输入的当前行」）。`drain_committed`/`finalize` 推进它；`clear_stream`
    /// /新回合重置。这样无论 live 如何被填充（流式增量 / 直接赋值），live 区都不与
    /// 已进滚屏的内容重复。
    pub(crate) committed_live_len: usize,
    /// 当前文字片段的起点（上一个 tool/turn 边界处的 `committed_live_len`）。
    ///
    /// 逐行增量提交时不动它；只在边界（tool begin / turn end）处把
    /// `live[segment_start..committed_live_len]` 记成一条 `StreamedAgent`（供
    /// overlay），再推进到 `committed_live_len`。从而 overlay 看到的是按 tool 切片
    /// 的完整原文，与滚屏的逐行交错一致。
    pub(crate) segment_start: usize,
    pub composer: ComposerInput,
    pub busy: bool,
    pub status_line: Option<ratatui::text::Line<'static>>,
    /// HUD 第 2 行(metrics:工具调用统计)。
    pub metrics_line: Option<ratatui::text::Line<'static>>,
    /// Ctrl+T transcript overlay。
    pub(crate) transcript: Option<TranscriptPager>,
    /// `/diff` diff 全屏 overlay(复用 TranscriptPager 渲染 diff 行)。
    pub(crate) diff_overlay: Option<TranscriptPager>,
    /// 待审批（y/n）。
    pub(crate) pending_approval: Option<PendingApproval>,
    pub(crate) cwd: PathBuf,
    /// 当前回合开始时刻（用于回合分隔符的 "Worked for Nm"）。
    pub(crate) turn_started_at: Option<Instant>,
    /// 本回合是否出现过工具调用（决定是否画分隔符）。
    pub(crate) had_tool_this_turn: bool,
    /// 已写入 history 的工具调用 call_id（去重：begin/end 只写一条 end）。
    pub(crate) seen_tool_calls: HashSet<String>,
    /// begin 携带 tool_name、end 只带 call_id:用 call_id→name 映射补名。
    pub(crate) tool_names: HashMap<String, String>,
    /// 上下文用量(供 HUD identity 行的 Context █░░ 进度条)。
    pub(crate) context_usage: ContextUsage,
    /// 计划步骤(供 /tasks overlay 的 checkbox 列表)。
    pub(crate) plan_steps: Vec<PlanStep>,
    /// 子 agent / 协作参与者(供 /tasks overlay 的 agent 面板)。
    pub(crate) agents: Vec<AgentEntry>,
    /// `/tasks` overlay(alt-screen,左 message_list + 右 task/agent 面板)。
    pub(crate) tasks_overlay: Option<crate::tasks_pager::TasksPager>,
    /// Vim 模式开关(底部状态行右侧显示 `INSERT` / `NORMAL`)。
    /// 与 `composer.toggle_vim_enabled()` 同步,启动期默认 off。
    pub vim_enabled: bool,
    /// `/clear` 二次确认标志:输入 `/clear` 后显示确认提示,
    /// 按 `y` 确认清除,按 `n` 取消。
    pub pending_clear_confirm: bool,
    /// 从 SessionConfigured 获取的模型名,显示在状态栏。
    /// `None` 时 fallback 到环境变量(REFLECT_MODEL_DISPLAY / OPENAI_MODEL / ANTHROPIC_MODEL)。
    pub status_model: Option<String>,
    /// v1.x:当前会话级 `PermissionMode`(Plan / Auto / Prompt / AcceptEdits …)。
    /// 启动期从 `thread.config().permission_mode()` 读初值,后续由
    /// `PermissionModeChanged` 事件回流更新。驱动 HUD mode 段与 composer 占位符。
    pub permission_mode: reflect_protocol::PermissionMode,
    /// v1.x Plan mode:`PlanReady` 到来时填入,打开审批弹窗;
    /// 用户 `1/2/3/Esc` 或 `PlanApproved/Rejected` 后清空。
    pub plan_approval: Option<PlanApprovalState>,
    /// v1.x `/raw [on|off]`：切换 raw scrollback 模式（绕过 markdown / glyph 改写）。
    pub raw_output: bool,
    /// 全屏模式：`history` 区距底部的滚动偏移（0 = 贴底，新消息可见）。
    /// 仅全屏 alt-screen 模式使用；inline 模式靠终端原生 scrollback，此值恒为 0。
    pub history_scroll_offset: u16,
    /// v1.x `/think`：切换 thinking 块显示（控制是否在 history 里画 thinking 行）。
    pub show_thinking: bool,
    /// v1.x Tier 4.5：session overlay（/new /archive /delete /resume /fork /rename）。
    /// 复用 `TasksPager` 的滚动状态;list 内容来自 `state.session_entries`。
    pub session_overlay: Option<crate::tasks_pager::TasksPager>,
    /// session list（由 `/resume` 打开 overlay 时填充）。
    pub session_entries: Vec<SessionEntry>,
    /// v1.x Tier 4.5：`/image` picker overlay。
    pub image_picker: Option<ImagePickerState>,
    /// v1.x Tier 4.5：`/keymap` picker overlay。
    pub keymap_picker: Option<KeymapPickerState>,
    /// v1.x Tier 4.5：`/statusline` picker overlay。
    pub statusline_picker: Option<StatuslinePickerState>,
    /// v1.x Tier 4.5：`/theme` picker overlay。
    pub theme_picker: Option<ThemePickerState>,
    /// v1.x Tier 5：`/checkpoint` overlay — git 快照列表。
    pub checkpoint_overlay: Option<crate::picker::checkpoint_overlay::CheckpointOverlayState>,
    /// v1.x Tier 5：checkpoint rewind 待确认 sha(二次确认)。
    pub checkpoint_rewind_sha: Option<String>,
    /// v1.x Tier 5: fork/rewind select overlay — 从历史 user prompt 选择。
    pub fork_rewind: Option<crate::picker::fork_rewind::ForkRewindState>,
    /// v1.x Tier 5: `/copy` overlay — 列出 agent 回复，选中后复制到剪贴板。
    pub copy_history: Option<crate::picker::copy_history::CopyHistoryState>,
    /// v1.x Tier 5: `/skills hub` overlay — 技能列表。
    pub skills_hub: Option<crate::picker::skills_hub::SkillsHubState>,
    /// 活跃的剪贴板所有权 lease(Linux/X11 与部分 Wayland 需要写进程保持 handle)。
    ///
    /// `copy_to_clipboard` 在这些后端返回 `Some(ClipboardLease)`;若被立即丢弃,
    /// 剪贴板内容会在用户粘贴前消失(模块文档明确警告)。这里存住最近一次 copy
    /// 的 lease,下次 copy 时覆盖并 drop 旧的,让 lease 活到下一次复制或会话结束。
    /// 非 Linux 平台 lease 恒为 `None`,零成本。
    pub(crate) clipboard_lease: Option<ClipboardLease>,
    /// 后台异步任务追踪：`emit_op` / 内联 spawn 产生的 fire-and-forget tasks。
    /// TUI exit 时 `abort_all` 防止事件丢失。使用 `Arc<Mutex<JoinSet>>` 使其可
    /// 在多个函数（handle_event / dispatch_slash_command / handle_key）中共享，
    /// 避免级联签名变更。
    pub(crate) background_tasks: Arc<Mutex<JoinSet<()>>>,
    /// v1.x fork 接线:当前会话的 ThreadId(从 `thread.config().session_id`
    /// 读初值)。fork overlay 用它作 `fork_with_history` 的 parent_id。
    pub session_id: Option<reflect_protocol::ThreadId>,
    /// v1.x fork 接线:最近一次 `TurnStarted` 的 turn_id。fork overlay 选定
    /// user prompt 后,据此查 `user_prompt_turns` 得到截断 turn。
    pub(crate) current_turn_id: Option<reflect_protocol::TurnId>,
    /// v1.x fork 接线:每条 user prompt 对应的 turn_id(与 history 中
    /// `UiHistoryItem::User` 项同序)。fork overlay 用 `user_prompt_turns[selected]`
    /// 作 `fork_with_history` 的 `up_to_turn_id`(在该 turn 含处截断)。
    pub(crate) user_prompt_turns: Vec<reflect_protocol::TurnId>,
    /// 最近一次「picker/overlay 打开时」按下的 Ctrl+C 时刻。
    ///
    /// 因为 9 个 picker 打开时第一次 Ctrl+C 只关 picker(走各自 handle_key),
    /// 用户难以一次退出。记录该时刻后,若 1.5s 内第二次 Ctrl+C 仍命中 picker 分支,
    /// 直接退出 TUI —— 实现「按一次关 picker,紧接再按一次退出」。普通(无 picker)
    /// Ctrl+C 仍经 keymap 一次退出,不受此字段影响。
    /// 终端窗口大小变化时设为 true,让下一帧在主循环里:
    /// 1. 清空 scrollback + 可见屏(`clear_scrollback_and_visible_screen_ansi`),
    ///    消除已入 scrollback 的旧宽度历史行的错位/重叠/重复;
    /// 2. 重置 `history_tail = 0`,让 `state.history` 以**新宽度**重新走一遍
    ///    `history_item_to_lines → pending → insert_history_lines`,重建 scrollback;
    /// 3. 视口 buffer 在 `set_viewport_area` 已被 reset(单独的修复),下一帧
    ///    `draw` 用新宽度完整渲染,无 diff 损坏。
    /// 不修的话:旧宽度历史行永久错位 + 新宽度 viewport 内容与 scrollback 宽度不一,
    /// 出现重叠/残影/重复显示,且永远不会自动恢复(只能重启 TUI)。
    pub(crate) needs_terminal_reflow: bool,
    pub(crate) last_quit_press: Option<Instant>,
    /// v0.4+ `/loop <secs> <cmd>` 的本地调度状态。`None` 表示无活跃 loop。
    /// 由文本提交路径的 `SlashOutcome::Loop` 分支设置/清空，主循环每帧检查
    /// `next_fire` 到点则提交 `command`。
    pub(crate) loop_state: Option<LoopState>,
    /// v1.x Plan mode:`PlanRequest` 到来时填入,打开进入 plan 模式的确认条。
    /// 用户 `1/3/Esc` 后回送 `Op::PlanApproval` 并清空。
    pub(crate) plan_enter_request: Option<PlanEnterRequestState>,
}

/// v1.x Tier 4.5:一条 session 记录（用于 `/resume` / `/fork` 等 overlay 列表）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    /// session id（UUID / 短 id）。
    pub id: String,
    /// 人类可读名（用户通过 `/rename` 设置）。
    pub name: String,
    /// 创建时间(ISO 8601 / Unix 时间戳字符串)。
    pub created_at: String,
    /// last activity hint（"today" / "3d ago" / "2026-07-25"）。
    pub last_active: String,
    /// 当前 session 高亮。
    pub is_current: bool,
}

impl SessionEntry {
    pub fn checkbox(&self) -> &'static str {
        if self.is_current { "●" } else { "○" }
    }
}

/// v1.x Tier 4.5:`/image` picker 状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePickerState {
    pub cwd: std::path::PathBuf,
    pub files: Vec<ImageFileEntry>,
    pub selected: usize,
    /// `[1..=10]` MiB 体积上限。
    pub max_size_mib: u64,
}

impl Default for ImagePickerState {
    fn default() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            files: Vec::new(),
            selected: 0,
            max_size_mib: 10,
        }
    }
}

/// v1.x Tier 4.5:image picker 中的一行（一个 png/jpg/gif/webp 文件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFileEntry {
    pub path: std::path::PathBuf,
    pub size_bytes: u64,
    pub mime: String,
}

/// v1.x Tier 4.5:`/keymap` picker 状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapPickerState {
    /// 当前快捷键(action) → 绑定键 的可读摘要。
    pub bindings: Vec<KeymapBinding>,
    pub selected: usize,
}

impl Default for KeymapPickerState {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
            selected: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapBinding {
    pub context: String, // "App" / "Composer" / "Pager" …
    pub action: String,  // "CycleMode" / "CopyLastReply" …
    pub key: String,     // "Shift+Tab" / "Ctrl+O" …
}

/// v1.x Tier 4.5:`/statusline` picker 状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatuslinePickerState {
    pub identity_items: Vec<StatuslineItem>,
    pub metrics_items: Vec<StatuslineItem>,
    pub selected_section: StatuslineSection,
    pub selected_index: usize,
}

impl Default for StatuslinePickerState {
    fn default() -> Self {
        Self {
            identity_items: vec![
                StatuslineItem {
                    name: "model".into(),
                    enabled: true,
                },
                StatuslineItem {
                    name: "workspace".into(),
                    enabled: true,
                },
                StatuslineItem {
                    name: "branch".into(),
                    enabled: true,
                },
                StatuslineItem {
                    name: "mode".into(),
                    enabled: true,
                },
                StatuslineItem {
                    name: "theme".into(),
                    enabled: false,
                },
            ],
            metrics_items: vec![
                StatuslineItem {
                    name: "context-bar".into(),
                    enabled: true,
                },
                StatuslineItem {
                    name: "tool-count".into(),
                    enabled: true,
                },
                StatuslineItem {
                    name: "tokens".into(),
                    enabled: false,
                },
                StatuslineItem {
                    name: "cost".into(),
                    enabled: false,
                },
                StatuslineItem {
                    name: "status".into(),
                    enabled: false,
                },
            ],
            selected_section: StatuslineSection::Identity,
            selected_index: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatuslineSection {
    Identity,
    Metrics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatuslineItem {
    pub name: String,
    pub enabled: bool,
}

/// v1.x Tier 4.5:`/theme` picker 状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemePickerState {
    pub themes: Vec<ThemeEntry>,
    pub selected: usize,
    /// 当前应用的主题名（与 `state.permission_mode` 旁的 mode 段类似）。
    pub current: String,
}

impl Default for ThemePickerState {
    fn default() -> Self {
        let themes = vec![
            ("dark", "Dark (default)"),
            ("light", "Light"),
            ("no-color", "No color (raw)"),
        ]
        .into_iter()
        .map(|(id, label)| ThemeEntry {
            id: id.to_string(),
            label: label.to_string(),
        })
        .collect();
        Self {
            themes,
            selected: 0,
            current: "dark".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeEntry {
    pub id: String,
    pub label: String,
}

/// 一个计划步骤(`/tasks` overlay 渲染 `[ ]/[~]/[x]/[-]` checkbox)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStep {
    pub index: usize,
    pub status: PlanStepStatus,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Done,
    Skipped,
}

impl PlanStepStatus {
    /// checkbox 字形:Pending `[ ]`、InProgress `[~]`、Done `[x]`、Skipped `[-]`。
    pub fn checkbox(self) -> &'static str {
        match self {
            PlanStepStatus::Pending => "[ ]",
            PlanStepStatus::InProgress => "[~]",
            PlanStepStatus::Done => "[x]",
            PlanStepStatus::Skipped => "[-]",
        }
    }
}

/// 子 agent / 协作参与者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEntry {
    pub id: String,
    pub status: AgentStatus,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Running,
    Done,
}

impl AgentStatus {
    pub fn glyph(self) -> &'static str {
        match self {
            AgentStatus::Idle => "○",
            AgentStatus::Running => "◐",
            AgentStatus::Done => "●",
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            history: Vec::new(),
            live: String::new(),
            live_thinking: String::new(),
            stream: None,
            last_stream_width: 120,
            committed_live_len: 0,
            segment_start: 0,
            history_scroll_offset: 0,
            composer: ComposerInput::new_with_config(
                "Send a message… (shift+enter for newline)".to_string(),
                /*disable_paste_burst*/ false,
            ),
            busy: false,
            status_line: None,
            metrics_line: None,
            transcript: None,
            diff_overlay: None,
            pending_approval: None,
            cwd,
            turn_started_at: None,
            had_tool_this_turn: false,
            seen_tool_calls: HashSet::new(),
            tool_names: HashMap::new(),
            context_usage: ContextUsage::default(),
            plan_steps: Vec::new(),
            agents: Vec::new(),
            tasks_overlay: None,
            vim_enabled: false,
            pending_clear_confirm: false,
            status_model: None,
            permission_mode: reflect_protocol::PermissionMode::default(),
            plan_approval: None,
            raw_output: false,
            show_thinking: true,
            session_overlay: None,
            session_entries: Vec::new(),
            image_picker: None,
            keymap_picker: None,
            statusline_picker: None,
            theme_picker: None,
            checkpoint_overlay: None,
            checkpoint_rewind_sha: None,
            fork_rewind: None,
            copy_history: None,
            skills_hub: None,
            clipboard_lease: None,
            background_tasks: Arc::new(Mutex::new(JoinSet::new())),
            session_id: None,
            current_turn_id: None,
            user_prompt_turns: Vec::new(),
            last_quit_press: None,
            needs_terminal_reflow: false,
            loop_state: None,
            plan_enter_request: None,
        }
    }
}

impl UiState {
    /// 确保 StreamController 已按当前宽度创建。
    pub fn ensure_stream(&mut self, width: u16) {
        let content_width = width.max(2).saturating_sub(2);
        if self.stream.is_none() {
            self.stream = Some(StreamController::new_with_inline_visualizations(
                Some(usize::from(content_width)),
                &self.cwd,
                HistoryRenderMode::Rich,
                None,
            ));
        }
        // 记录本次内容宽度，供 adapter 边界 flush 时 display_lines 折行使用。
        self.last_stream_width = content_width.max(1);
    }

    /// 终端宽度变化时同步控制器渲染宽度（重新渲染已稳定区，保持行长一致）。
    pub fn sync_stream_width(&mut self, width: u16) {
        let content_width = width.max(2).saturating_sub(2);
        self.last_stream_width = content_width.max(1);
        if let Some(stream) = self.stream.as_mut() {
            stream.set_width(Some(usize::from(content_width)));
        }
    }

    pub fn clear_stream(&mut self) {
        self.stream = None;
        self.live.clear();
        self.live_thinking.clear();
        self.committed_live_len = 0;
        self.segment_start = 0;
    }
}
