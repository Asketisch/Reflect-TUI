use super::*;
use std::collections::HashMap;

/// 便携式 TUI 键位映射配置支持的最大功能键编号。
/// 存储为 `u32`，与 `keymap.rs` 中调用点的范围检查保持一致。
pub const MAX_FUNCTION_KEY: u32 = 24;

/// 当终端回滚缓冲区大小未知时使用的回退行数上限。
pub const DEFAULT_TERMINAL_RESIZE_REFLOW_FALLBACK_MAX_ROWS: u32 = 1_000;

// ----- 键位映射结构 ---------------------------------------------------

/// 单个按键事件的规范化字符串表示（例如 `ctrl-a`）。
/// 采用元组结构体，使内置调用点可以直接写
/// `KeybindingSpec("ctrl-a".to_string())`。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(transparent)]
pub struct KeybindingSpec(pub String);

impl KeybindingSpec {
    /// 规范的按键规格字符串（例如 `ctrl-a`）。
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// 配置中某个动作绑定的值：单个按键规格或按键规格列表。
/// 空的 `Many([])` 表示显式解除该动作的绑定。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum KeybindingsSpec {
    One(KeybindingSpec),
    Many(Vec<KeybindingSpec>),
}

impl Default for KeybindingsSpec {
    fn default() -> Self {
        Self::Many(Vec::new())
    }
}

impl KeybindingsSpec {
    /// 某个动作已配置的全部按键规格，按声明顺序排列。
    pub fn specs(&self) -> Vec<&KeybindingSpec> {
        match self {
            Self::One(spec) => vec![spec],
            Self::Many(specs) => specs.iter().collect(),
        }
    }
}

// ----- 各上下文的键位映射表 --------------------------------------
//
// 每个上下文都是一个由 `Option<KeybindingsSpec>` 槽位组成的普通结构体。
// 字段名与 `keymap_setup/actions.rs` 中 `binding_slot` 所引用的
// 规范动作名一致。

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiGlobalKeymap {
    pub open_transcript: Option<KeybindingsSpec>,
    pub open_external_editor: Option<KeybindingsSpec>,
    pub copy: Option<KeybindingsSpec>,
    pub clear_terminal: Option<KeybindingsSpec>,
    pub submit: Option<KeybindingsSpec>,
    pub queue: Option<KeybindingsSpec>,
    pub toggle_shortcuts: Option<KeybindingsSpec>,
    pub toggle_vim_mode: Option<KeybindingsSpec>,
    pub toggle_fast_mode: Option<KeybindingsSpec>,
    pub toggle_raw_output: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiChatKeymap {
    pub interrupt_turn: Option<KeybindingsSpec>,
    pub decrease_reasoning_effort: Option<KeybindingsSpec>,
    pub increase_reasoning_effort: Option<KeybindingsSpec>,
    pub edit_queued_message: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiComposerKeymap {
    pub submit: Option<KeybindingsSpec>,
    pub queue: Option<KeybindingsSpec>,
    pub toggle_shortcuts: Option<KeybindingsSpec>,
    pub history_search_previous: Option<KeybindingsSpec>,
    pub history_search_next: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiEditorKeymap {
    pub insert_newline: Option<KeybindingsSpec>,
    pub move_left: Option<KeybindingsSpec>,
    pub move_right: Option<KeybindingsSpec>,
    pub move_up: Option<KeybindingsSpec>,
    pub move_down: Option<KeybindingsSpec>,
    pub move_word_left: Option<KeybindingsSpec>,
    pub move_word_right: Option<KeybindingsSpec>,
    pub move_line_start: Option<KeybindingsSpec>,
    pub move_line_end: Option<KeybindingsSpec>,
    pub delete_backward: Option<KeybindingsSpec>,
    pub delete_forward: Option<KeybindingsSpec>,
    pub delete_backward_word: Option<KeybindingsSpec>,
    pub delete_forward_word: Option<KeybindingsSpec>,
    pub kill_line_start: Option<KeybindingsSpec>,
    pub kill_whole_line: Option<KeybindingsSpec>,
    pub kill_line_end: Option<KeybindingsSpec>,
    pub yank: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiVimNormalKeymap {
    pub enter_insert: Option<KeybindingsSpec>,
    pub append_after_cursor: Option<KeybindingsSpec>,
    pub append_line_end: Option<KeybindingsSpec>,
    pub insert_line_start: Option<KeybindingsSpec>,
    pub open_line_below: Option<KeybindingsSpec>,
    pub open_line_above: Option<KeybindingsSpec>,
    pub move_left: Option<KeybindingsSpec>,
    pub move_right: Option<KeybindingsSpec>,
    pub move_up: Option<KeybindingsSpec>,
    pub move_down: Option<KeybindingsSpec>,
    pub move_word_forward: Option<KeybindingsSpec>,
    pub move_word_backward: Option<KeybindingsSpec>,
    pub move_word_end: Option<KeybindingsSpec>,
    pub move_line_start: Option<KeybindingsSpec>,
    pub move_line_end: Option<KeybindingsSpec>,
    pub delete_char: Option<KeybindingsSpec>,
    pub substitute_char: Option<KeybindingsSpec>,
    pub delete_to_line_end: Option<KeybindingsSpec>,
    pub change_to_line_end: Option<KeybindingsSpec>,
    pub yank_line: Option<KeybindingsSpec>,
    pub paste_after: Option<KeybindingsSpec>,
    pub start_delete_operator: Option<KeybindingsSpec>,
    pub start_yank_operator: Option<KeybindingsSpec>,
    pub start_change_operator: Option<KeybindingsSpec>,
    pub cancel_operator: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiVimOperatorKeymap {
    pub delete_line: Option<KeybindingsSpec>,
    pub yank_line: Option<KeybindingsSpec>,
    pub motion_left: Option<KeybindingsSpec>,
    pub motion_right: Option<KeybindingsSpec>,
    pub motion_up: Option<KeybindingsSpec>,
    pub motion_down: Option<KeybindingsSpec>,
    pub motion_word_forward: Option<KeybindingsSpec>,
    pub motion_word_backward: Option<KeybindingsSpec>,
    pub motion_word_end: Option<KeybindingsSpec>,
    pub motion_line_start: Option<KeybindingsSpec>,
    pub motion_line_end: Option<KeybindingsSpec>,
    pub select_inner_text_object: Option<KeybindingsSpec>,
    pub select_around_text_object: Option<KeybindingsSpec>,
    pub cancel: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiVimTextObjectKeymap {
    pub word: Option<KeybindingsSpec>,
    pub big_word: Option<KeybindingsSpec>,
    pub parentheses: Option<KeybindingsSpec>,
    pub brackets: Option<KeybindingsSpec>,
    pub braces: Option<KeybindingsSpec>,
    pub double_quote: Option<KeybindingsSpec>,
    pub single_quote: Option<KeybindingsSpec>,
    pub backtick: Option<KeybindingsSpec>,
    pub cancel: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiPagerKeymap {
    pub scroll_up: Option<KeybindingsSpec>,
    pub scroll_down: Option<KeybindingsSpec>,
    pub page_up: Option<KeybindingsSpec>,
    pub page_down: Option<KeybindingsSpec>,
    pub half_page_up: Option<KeybindingsSpec>,
    pub half_page_down: Option<KeybindingsSpec>,
    pub jump_top: Option<KeybindingsSpec>,
    pub jump_bottom: Option<KeybindingsSpec>,
    pub close: Option<KeybindingsSpec>,
    pub close_transcript: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiListKeymap {
    pub move_up: Option<KeybindingsSpec>,
    pub move_down: Option<KeybindingsSpec>,
    pub move_left: Option<KeybindingsSpec>,
    pub move_right: Option<KeybindingsSpec>,
    pub page_up: Option<KeybindingsSpec>,
    pub page_down: Option<KeybindingsSpec>,
    pub jump_top: Option<KeybindingsSpec>,
    pub jump_bottom: Option<KeybindingsSpec>,
    pub accept: Option<KeybindingsSpec>,
    pub cancel: Option<KeybindingsSpec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiApprovalKeymap {
    pub open_fullscreen: Option<KeybindingsSpec>,
    pub open_thread: Option<KeybindingsSpec>,
    pub approve: Option<KeybindingsSpec>,
    pub approve_for_session: Option<KeybindingsSpec>,
    pub approve_for_prefix: Option<KeybindingsSpec>,
    pub deny: Option<KeybindingsSpec>,
    pub decline: Option<KeybindingsSpec>,
    pub cancel: Option<KeybindingsSpec>,
}

/// `[tui.keymap]` 在磁盘上的持久化形态。字段名与 `keymap_setup/actions.rs`
/// 中 `binding_slot` 所使用的上下文标识符一致。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TuiKeymap {
    #[serde(default)]
    pub global: TuiGlobalKeymap,
    #[serde(default)]
    pub chat: TuiChatKeymap,
    #[serde(default)]
    pub composer: TuiComposerKeymap,
    #[serde(default)]
    pub editor: TuiEditorKeymap,
    #[serde(default)]
    pub vim_normal: TuiVimNormalKeymap,
    #[serde(default)]
    pub vim_operator: TuiVimOperatorKeymap,
    #[serde(default)]
    pub vim_text_object: TuiVimTextObjectKeymap,
    #[serde(default)]
    pub pager: TuiPagerKeymap,
    #[serde(default)]
    pub list: TuiListKeymap,
    #[serde(default)]
    pub approval: TuiApprovalKeymap,
}

// ----- 审批 / 认证 / 通知 --------------------------------

/// 配置审批请求路由给谁进行审查。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalsReviewer {
    /// 将审批提示路由给交互式用户（默认）。
    #[default]
    User,
    /// 将审批提示路由给自动审查子代理。
    #[serde(alias = "guardian_subagent")]
    AutoReview,
}

impl std::fmt::Display for ApprovalsReviewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::AutoReview => write!(f, "auto_review"),
        }
    }
}

impl ApprovalsReviewer {
    pub fn iter(&self) -> impl Iterator<Item = Self> {
        [Self::User, Self::AutoReview].into_iter()
    }
}

impl IntoIterator for ApprovalsReviewer {
    type Item = Self;
    type IntoIter = std::array::IntoIter<Self, 2>;
    fn into_iter(self) -> Self::IntoIter {
        [Self::User, Self::AutoReview].into_iter()
    }
}

impl<'a> IntoIterator for &'a ApprovalsReviewer {
    type Item = ApprovalsReviewer;
    type IntoIter = std::array::IntoIter<ApprovalsReviewer, 2>;
    fn into_iter(self) -> Self::IntoIter {
        [ApprovalsReviewer::User, ApprovalsReviewer::AutoReview].into_iter()
    }
}

/// 恢复或分叉会话时使用的工作目录。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResumeCwdMode {
    /// 使用启动 Reflect 时所在的目录（默认）。
    #[default]
    Current,
    /// 使用所选会话中记录的最新工作目录。
    Session,
}

/// Reflect 存储 CLI 认证凭据的位置。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthCredentialsStoreMode {
    /// 优先使用系统密钥串（keyring），失败时回退到 `$REFLECT_HOME` 中的文件（默认）。
    #[default]
    Auto,
    /// 将凭据持久化到 `$REFLECT_HOME/auth.json`。
    File,
    /// 将凭据持久化到操作系统密钥串；不可用时报错。
    Keyring,
    /// 仅在当前进程的内存中保留凭据。
    Ephemeral,
}

impl std::fmt::Display for WindowsSandboxModeToml {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Elevated => "elevated",
            Self::Unelevated => "unelevated",
        })
    }
}

/// 在 Windows 下运行时所选的 Windows 沙箱实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowsSandboxModeToml {
    Elevated,
    Unelevated,
}

/// 终端桌面通知的发出方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationMethod {
    /// 自动检测最佳后端（默认）。
    #[default]
    Auto,
    /// 使用 OSC 9 转义序列。
    Osc9,
    /// 使用终端铃声。
    Bel,
}

impl std::fmt::Display for NotificationMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Osc9 => "osc9",
            Self::Bel => "bel",
        })
    }
}

/// TUI 桌面通知的启用配置：可以是布尔值（全部启用/禁用），
/// 也可以是显式的事件类型名称允许列表。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum Notifications {
    Enabled(bool),
    Custom(Vec<String>),
}

impl Default for Notifications {
    fn default() -> Self {
        Self::Enabled(true)
    }
}

/// 宠物动画在 TUI 视口上的锚定位置。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TuiPetAnchor {
    /// 锚定到当前输入区（composer）视口的底部（默认）。
    #[default]
    Composer,
    /// 锚定到终端屏幕的物理底部。
    ScreenBottom,
}

// ----- MCP 服务器配置 ----------------------------------------------

/// 单个 MCP 服务器的传输配置。各变体镜像上游的 `untagged` 枚举，
/// 使内置测试中的 `toml::Value::try_into` 反序列化继续
/// 正常工作。字段名与上游 TOML 键一致。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged, deny_unknown_fields, rename_all = "snake_case")]
pub enum McpServerTransportConfig {
    /// stdio 传输：启动一个本地子进程。
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env_vars: Vec<McpServerEnvVar>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// streamable-http 传输：连接到远程 HTTP 端点。
    StreamableHttp {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer_token_env_var: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http_headers: Option<HashMap<String, String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env_http_headers: Option<HashMap<String, String>>,
    },
}

impl Default for McpServerTransportConfig {
    fn default() -> Self {
        Self::Stdio {
            command: String::new(),
            args: Vec::new(),
            env: None,
            env_vars: Vec::new(),
            cwd: None,
        }
    }
}

/// stdio MCP 服务器配置携带的环境变量声明。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct McpServerEnvVar {
    pub name: String,
    pub value: String,
}

/// 单个已配置的 MCP 服务器。默认值会生成一个命令为空的 stdio 传输，
/// 使内置的 `McpServerConfig::default()` 继续正常工作。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct McpServerConfig {
    #[serde(default)]
    pub transport: McpServerTransportConfig,
}

// ----- 为兼容性保留的其他存根枚举 ----------------------

/// `[sandbox]` 配置所携带的沙箱模式。为兼容旧引用而保留。
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum SandboxMode {
    #[default]
    Workspace,
    DangerFullAccess,
    ReadOnly,
}

/// 模型提供方 API 的服务层级。为兼容旧引用而保留。
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum ServiceTier {
    #[default]
    Standard,
    Pro,
    Priority,
}
