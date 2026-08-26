//! 运行时键映射解析引擎。
//!
//! 注：本文件超过 800 行红线，但 `impl RuntimeKeymap` 为内聚的键映射解析流水线
//! （defaults → config parse → built-in fill → conflict validation，4 个紧密耦合的方法），
//! 与上游逐行对应，按 AGENTS.md「内聚完整状态机例外」保留。

//! TUI 的运行时键映射解析。
//!
//! 本模块将反序列化的配置（`TuiKeymap`）转换为运行时输入处理器使用的具体 `RuntimeKeymap`。
//!
//! 核心职责：
//!
//! 1. 应用确定性优先级（`上下文 -> 全局回退 -> 默认值`）。
//! 2. 将规范键规范字符串解析为 `KeyBinding` 值。
//! 3. 在运行时各表面间强制唯一性，确保同一键不会在同一焦点输入路径上触发
//!    多个动作。
//! 4. 返回可操作的面向用户的错误消息，包含配置路径和后续步骤。
//!
//! 非职责：
//!
//! 1. 本模块不决定在给定屏幕中应执行哪个动作。
//!    调用方通过检查相关动作绑定集来解析动作。
//! 2. 本模块不持久化配置；仅解析已加载的配置。

use crate::config_compat::types::KeybindingsSpec;
use crate::config_compat::types::MAX_FUNCTION_KEY;
use crate::config_compat::types::TuiKeymap;
use crate::tui_core::key_hint;
use crate::tui_core::key_hint::KeyBinding;
use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use serde::Serialize;
use std::collections::HashMap;

/// TUI 输入处理器使用的运行时键映射。
///
/// 解析优先级：
///
/// 1. 上下文专用绑定（`tui.keymap.<context>`）。
/// 2. 支持全局回退的动作使用 `tui.keymap.global`。
/// 3. 内置默认值。
///
/// 这是 UI 代码分发时应使用的唯一形态。它代表已完全解析的快照，
/// 已应用解析、回退、显式取消绑定和重复键验证。如果调用方在配置
/// 变更后继续使用旧快照，可见提示和活跃处理器可能产生偏差。
#[derive(Clone, Debug)]
pub(crate) struct RuntimeKeymap {
    pub(crate) app: AppKeymap,
    pub(crate) chat: ChatKeymap,
    pub(crate) composer: ComposerKeymap,
    pub(crate) editor: EditorKeymap,
    pub(crate) vim_normal: VimNormalKeymap,
    pub(crate) vim_operator: VimOperatorKeymap,
    pub(crate) vim_text_object: VimTextObjectKeymap,
    pub(crate) pager: PagerKeymap,
    pub(crate) list: ListKeymap,
    pub(crate) approval: ApprovalKeymap,
}

#[derive(Clone, Debug)]
pub(crate) struct AppKeymap {
    /// 打开文稿覆盖层。
    pub(crate) open_transcript: Vec<KeyBinding>,
    /// 为当前草稿打开外部编辑器。
    pub(crate) open_external_editor: Vec<KeyBinding>,
    /// 复制最后一条 agent 回复到剪贴板。
    pub(crate) copy: Vec<KeyBinding>,
    /// 清除终端 UI。
    pub(crate) clear_terminal: Vec<KeyBinding>,
    /// 切换 composer 输入的 Vim 模式。
    pub(crate) toggle_vim_mode: Vec<KeyBinding>,
    /// 切换快速模式。
    pub(crate) toggle_fast_mode: Vec<KeyBinding>,
    /// 切换原始回滚模式以方便选择文稿内容。
    pub(crate) toggle_raw_output: Vec<KeyBinding>,
}

/// 在应用事件层求值的聊天级键绑定。
///
/// 这些键绑定与 `AppKeymap` 动作一起参与第一次应用作用域冲突验证，
/// 因为两者都在输入到达 composer 之前被检查。分发门控（回溯的空 composer
/// 守卫）在处理器代码中发生，不在此处。
#[derive(Clone, Debug)]
pub(crate) struct ChatKeymap {
    /// 中断活跃回合。
    pub(crate) interrupt_turn: Vec<KeyBinding>,
    /// 降低活跃推理努力。
    pub(crate) decrease_reasoning_effort: Vec<KeyBinding>,
    /// 增加活跃推理努力。
    pub(crate) increase_reasoning_effort: Vec<KeyBinding>,
    /// 编辑最近排队的消息。
    pub(crate) edit_queued_message: Vec<KeyBinding>,
}

/// 在第二次应用作用域冲突验证中校验的 Composer 级键绑定。
///
/// 应用级处理器在 composer 接收输入之前执行，因此此处绑定的任何键若
/// 也出现在 `AppKeymap` 中，将被静默拦截。冲突验证器通过检查应用 +
/// composer 唯一性来防止此情况。
#[derive(Clone, Debug)]
pub(crate) struct ComposerKeymap {
    /// 提交当前草稿。
    pub(crate) submit: Vec<KeyBinding>,
    /// 任务运行时将当前草稿排队。
    pub(crate) queue: Vec<KeyBinding>,
    /// 切换 composer 快捷键覆盖层。
    pub(crate) toggle_shortcuts: Vec<KeyBinding>,
    /// 打开反向历史搜索或移至上一匹配项。
    pub(crate) history_search_previous: Vec<KeyBinding>,
    /// 在反向历史搜索中移至下一匹配项。
    pub(crate) history_search_next: Vec<KeyBinding>,
}

/// Composer 文本区域使用的编辑器专用键绑定。
///
/// 这些绑定仅由文本编辑控件解释，不参与全局/聊天回退解析。
#[derive(Clone, Debug)]
pub(crate) struct EditorKeymap {
    pub(crate) insert_newline: Vec<KeyBinding>,
    pub(crate) move_left: Vec<KeyBinding>,
    pub(crate) move_right: Vec<KeyBinding>,
    pub(crate) move_up: Vec<KeyBinding>,
    pub(crate) move_down: Vec<KeyBinding>,
    pub(crate) move_word_left: Vec<KeyBinding>,
    pub(crate) move_word_right: Vec<KeyBinding>,
    pub(crate) move_line_start: Vec<KeyBinding>,
    pub(crate) move_line_end: Vec<KeyBinding>,
    pub(crate) delete_backward: Vec<KeyBinding>,
    pub(crate) delete_forward: Vec<KeyBinding>,
    pub(crate) delete_backward_word: Vec<KeyBinding>,
    pub(crate) delete_forward_word: Vec<KeyBinding>,
    pub(crate) kill_line_start: Vec<KeyBinding>,
    pub(crate) kill_whole_line: Vec<KeyBinding>,
    pub(crate) kill_line_end: Vec<KeyBinding>,
    pub(crate) yank: Vec<KeyBinding>,
}

/// Composer 文本区域中模态编辑的 Vim 普通模式键绑定。
///
/// 启用 Vim 时，普通模式为休息状态。在此按移动或编辑键可移动光标、
/// 触发操作符等待状态（通过 `start_delete_operator` / `start_yank_operator`），
/// 或切换到插入模式。默认绑定包含 `shift(letter)` 和 `plain(UPPERCASE)` 变体
/// 以处理跨终端 shift 报告不一致问题。
#[derive(Clone, Debug, Default)]
pub(crate) struct VimNormalKeymap {
    pub(crate) enter_insert: Vec<KeyBinding>,
    pub(crate) append_after_cursor: Vec<KeyBinding>,
    pub(crate) append_line_end: Vec<KeyBinding>,
    pub(crate) insert_line_start: Vec<KeyBinding>,
    pub(crate) open_line_below: Vec<KeyBinding>,
    pub(crate) open_line_above: Vec<KeyBinding>,
    pub(crate) move_left: Vec<KeyBinding>,
    pub(crate) move_right: Vec<KeyBinding>,
    pub(crate) move_up: Vec<KeyBinding>,
    pub(crate) move_down: Vec<KeyBinding>,
    pub(crate) move_word_forward: Vec<KeyBinding>,
    pub(crate) move_word_backward: Vec<KeyBinding>,
    pub(crate) move_word_end: Vec<KeyBinding>,
    pub(crate) move_line_start: Vec<KeyBinding>,
    pub(crate) move_line_end: Vec<KeyBinding>,
    pub(crate) delete_char: Vec<KeyBinding>,
    pub(crate) substitute_char: Vec<KeyBinding>,
    pub(crate) delete_to_line_end: Vec<KeyBinding>,
    pub(crate) change_to_line_end: Vec<KeyBinding>,
    pub(crate) yank_line: Vec<KeyBinding>,
    pub(crate) paste_after: Vec<KeyBinding>,
    pub(crate) start_delete_operator: Vec<KeyBinding>,
    pub(crate) start_yank_operator: Vec<KeyBinding>,
    pub(crate) start_change_operator: Vec<KeyBinding>,
    pub(crate) cancel_operator: Vec<KeyBinding>,
}

/// 普通模式下 `d` 或 `y` 之后激活的 Vim 操作符等待键绑定。
///
/// 按下操作符（`start_delete_operator` 或 `start_yank_operator`）后，
/// 下一次按键与此上下文匹配以确定运动范围。重复操作符键（`dd`、`yy`）
/// 作用于整行。`Esc` 取消等待的操作符并返回普通模式。
#[derive(Clone, Debug, Default)]
pub(crate) struct VimOperatorKeymap {
    pub(crate) delete_line: Vec<KeyBinding>,
    pub(crate) yank_line: Vec<KeyBinding>,
    pub(crate) motion_left: Vec<KeyBinding>,
    pub(crate) motion_right: Vec<KeyBinding>,
    pub(crate) motion_up: Vec<KeyBinding>,
    pub(crate) motion_down: Vec<KeyBinding>,
    pub(crate) motion_word_forward: Vec<KeyBinding>,
    pub(crate) motion_word_backward: Vec<KeyBinding>,
    pub(crate) motion_word_end: Vec<KeyBinding>,
    pub(crate) motion_line_start: Vec<KeyBinding>,
    pub(crate) motion_line_end: Vec<KeyBinding>,
    pub(crate) select_inner_text_object: Vec<KeyBinding>,
    pub(crate) select_around_text_object: Vec<KeyBinding>,
    pub(crate) cancel: Vec<KeyBinding>,
}

/// 操作符加上 inner/around 前缀后激活的 Vim 文本对象键绑定。
#[derive(Clone, Debug, Default)]
pub(crate) struct VimTextObjectKeymap {
    pub(crate) word: Vec<KeyBinding>,
    pub(crate) big_word: Vec<KeyBinding>,
    pub(crate) parentheses: Vec<KeyBinding>,
    pub(crate) brackets: Vec<KeyBinding>,
    pub(crate) braces: Vec<KeyBinding>,
    pub(crate) double_quote: Vec<KeyBinding>,
    pub(crate) single_quote: Vec<KeyBinding>,
    pub(crate) backtick: Vec<KeyBinding>,
    pub(crate) cancel: Vec<KeyBinding>,
}

/// 文稿和静态帮助视图的分页器/覆盖层键绑定。
#[derive(Clone, Debug)]
pub(crate) struct PagerKeymap {
    pub(crate) scroll_up: Vec<KeyBinding>,
    pub(crate) scroll_down: Vec<KeyBinding>,
    pub(crate) page_up: Vec<KeyBinding>,
    pub(crate) page_down: Vec<KeyBinding>,
    pub(crate) half_page_up: Vec<KeyBinding>,
    pub(crate) half_page_down: Vec<KeyBinding>,
    pub(crate) jump_top: Vec<KeyBinding>,
    pub(crate) jump_bottom: Vec<KeyBinding>,
    pub(crate) close: Vec<KeyBinding>,
    pub(crate) close_transcript: Vec<KeyBinding>,
}

/// 在弹出列表视图间共享的通用列表选择器键绑定。
///
/// 这些动作描述列表意图而非特定控件布局。垂直动作移动高亮行，翻页和
/// 跳转动作在当前过滤的行集合中移动，水平动作可用于暴露相邻选择的视图，
/// 如标签页、工具栏值或有序项目移动。也接受搜索文本的视图负责在分发
/// 纯字符绑定前检查 `is_plain_text_key_event`，以免配置的 `j`、`k`、
/// `h` 或 `l` 抢占查询输入。
#[derive(Clone, Debug)]
pub(crate) struct ListKeymap {
    pub(crate) move_up: Vec<KeyBinding>,
    pub(crate) move_down: Vec<KeyBinding>,
    pub(crate) move_left: Vec<KeyBinding>,
    pub(crate) move_right: Vec<KeyBinding>,
    pub(crate) page_up: Vec<KeyBinding>,
    pub(crate) page_down: Vec<KeyBinding>,
    pub(crate) jump_top: Vec<KeyBinding>,
    pub(crate) jump_bottom: Vec<KeyBinding>,
    pub(crate) accept: Vec<KeyBinding>,
    pub(crate) cancel: Vec<KeyBinding>,
}

/// 审批模态键绑定。
///
/// 涵盖选择动作以及大型审批负载的「全屏查看详细信息」逃生口。
#[derive(Clone, Debug)]
pub(crate) struct ApprovalKeymap {
    pub(crate) open_fullscreen: Vec<KeyBinding>,
    pub(crate) open_thread: Vec<KeyBinding>,
    pub(crate) approve: Vec<KeyBinding>,
    pub(crate) approve_for_session: Vec<KeyBinding>,
    pub(crate) approve_for_prefix: Vec<KeyBinding>,
    pub(crate) deny: Vec<KeyBinding>,
    pub(crate) decline: Vec<KeyBinding>,
    pub(crate) cancel: Vec<KeyBinding>,
}

/// 返回第一个绑定，用作动作的主要 UI 提示。
///
/// 渲染代码应优先使用它获取简洁提示，同时保留所有绑定用于实际输入匹配。
pub(crate) fn primary_binding(bindings: &[KeyBinding]) -> Option<KeyBinding> {
    bindings.first().copied()
}

/// 从配置解析单个上下文局部动作绑定。
///
/// 展开为 `resolve_bindings(...)`，包含：
/// - 配置的源：`tui.keymap.<context>.<action>`
/// - 回退源：内置默认值中的相同动作
/// - 错误路径：用于面向用户诊断的稳定字符串路径
///
/// 这保持解析表简洁，同时保证路径字符串与字段名保持同步。
macro_rules! resolve_local {
    ($keymap:expr, $defaults:expr, $context:ident, $action:ident) => {
        resolve_bindings(
            ($keymap).$context.$action.as_ref(),
            &($defaults).$context.$action,
            concat!(
                "tui.keymap.",
                stringify!($context),
                ".",
                stringify!($action)
            ),
        )?
    };
}

/// 使用全局回退解析一个动作绑定。
///
/// 展开为 `resolve_bindings_with_global_fallback(...)`，优先级为：
/// 1. `tui.keymap.<context>.<action>`
/// 2. `tui.keymap.global.<action>`
/// 3. `<context>.<action>` 的内置默认值
///
/// 仅用于有意支持全局复用的动作。上下文本地空列表仍视为配置值，
/// 因此它们取消绑定动作而非回退到 `global`。
macro_rules! resolve_with_global {
    ($keymap:expr, $defaults:expr, $context:ident, $action:ident) => {
        resolve_bindings_with_global_fallback(
            ($keymap).$context.$action.as_ref(),
            ($keymap).global.$action.as_ref(),
            &($defaults).$context.$action,
            concat!(
                "tui.keymap.",
                stringify!($context),
                ".",
                stringify!($action)
            ),
        )?
    };
}

/// 将默认表中的单个绑定条目展开为 [`KeyBinding`]。
///
/// 这是 `key_hint::{plain, ctrl, alt, shift}` 之上的小型声明层，
/// 由 `default_bindings!` 使用，使 `built_in_defaults` 保持可读。
///
/// 支持的形式：
/// - `plain(<KeyCode>)`
/// - `ctrl(<KeyCode>)`
/// - `alt(<KeyCode>)`
/// - `shift(<KeyCode>)`
/// - `raw(<KeyBinding expression>)` 用于不匹配辅助宏的绑定
///   （例如组合修饰符 Ctrl+Shift）。
macro_rules! default_binding {
    (plain($key:expr)) => {
        key_hint::plain($key)
    };
    (ctrl($key:expr)) => {
        key_hint::ctrl($key)
    };
    (alt($key:expr)) => {
        key_hint::alt($key)
    };
    (shift($key:expr)) => {
        key_hint::shift($key)
    };
    (raw($binding:expr)) => {
        $binding
    };
}

/// 为内置默认值构建 `Vec<KeyBinding>`。
///
/// 此宏有意限定于内置键映射。运行时配置解析仍通过 `parse_bindings(...)` 进行，
/// 以便使用感知配置路径的诊断报告用户错误。
macro_rules! default_bindings {
    ($($kind:ident($($arg:tt)*)),* $(,)?) => {
        vec![$(default_binding!($kind($($arg)*))),*]
    };
}

impl Default for RuntimeKeymap {
    fn default() -> Self {
        Self::defaults()
    }
}

impl RuntimeKeymap {
    /// 返回内置默认值。
    ///
    /// 在加载用户配置之前，此为测试和引导 UI 状态的便捷方式。
    /// 解析 `TuiKeymap` 后不应将其用作回退，否则会忽略用户的显式
    /// 取消绑定和冲突诊断。
    pub(crate) fn defaults() -> Self {
        Self::built_in_defaults()
    }

    /// 从配置解析运行时键映射，应用优先级和验证。
    ///
    /// 以下情况返回错误：
    ///
    /// 1. 键绑定规范无法解析。
    /// 2. 某上下文存在歧义绑定（同一键分配给多个动作）。
    ///
    /// 错误文本包含相关配置路径和具体的下一步操作。调用代码在
    /// 分发前不应合并无关上下文间的绑定，否则此解析器的冲突保证
    /// 将不再成立。
    pub(crate) fn from_config(keymap: &TuiKeymap) -> Result<Self, String> {
        let defaults = Self::built_in_defaults();

        let app = AppKeymap {
            open_transcript: resolve_bindings(
                keymap.global.open_transcript.as_ref(),
                &defaults.app.open_transcript,
                "tui.keymap.global.open_transcript",
            )?,
            open_external_editor: resolve_bindings(
                keymap.global.open_external_editor.as_ref(),
                &defaults.app.open_external_editor,
                "tui.keymap.global.open_external_editor",
            )?,
            copy: resolve_bindings(
                keymap.global.copy.as_ref(),
                &defaults.app.copy,
                "tui.keymap.global.copy",
            )?,
            clear_terminal: resolve_bindings(
                keymap.global.clear_terminal.as_ref(),
                &defaults.app.clear_terminal,
                "tui.keymap.global.clear_terminal",
            )?,
            toggle_vim_mode: resolve_bindings(
                keymap.global.toggle_vim_mode.as_ref(),
                &defaults.app.toggle_vim_mode,
                "tui.keymap.global.toggle_vim_mode",
            )?,
            toggle_fast_mode: resolve_bindings(
                keymap.global.toggle_fast_mode.as_ref(),
                &defaults.app.toggle_fast_mode,
                "tui.keymap.global.toggle_fast_mode",
            )?,
            toggle_raw_output: resolve_bindings(
                keymap.global.toggle_raw_output.as_ref(),
                &defaults.app.toggle_raw_output,
                "tui.keymap.global.toggle_raw_output",
            )?,
        };

        let mut chat = ChatKeymap {
            interrupt_turn: resolve_bindings(
                keymap.chat.interrupt_turn.as_ref(),
                &defaults.chat.interrupt_turn,
                "tui.keymap.chat.interrupt_turn",
            )?,
            decrease_reasoning_effort: resolve_bindings(
                keymap.chat.decrease_reasoning_effort.as_ref(),
                &defaults.chat.decrease_reasoning_effort,
                "tui.keymap.chat.decrease_reasoning_effort",
            )?,
            increase_reasoning_effort: resolve_bindings(
                keymap.chat.increase_reasoning_effort.as_ref(),
                &defaults.chat.increase_reasoning_effort,
                "tui.keymap.chat.increase_reasoning_effort",
            )?,
            edit_queued_message: resolve_bindings(
                keymap.chat.edit_queued_message.as_ref(),
                &defaults.chat.edit_queued_message,
                "tui.keymap.chat.edit_queued_message",
            )?,
        };

        let composer = ComposerKeymap {
            submit: resolve_with_global!(keymap, defaults, composer, submit),
            queue: resolve_with_global!(keymap, defaults, composer, queue),
            toggle_shortcuts: resolve_with_global!(keymap, defaults, composer, toggle_shortcuts),
            history_search_previous: resolve_local!(
                keymap,
                defaults,
                composer,
                history_search_previous
            ),
            history_search_next: resolve_local!(keymap, defaults, composer, history_search_next),
        };

        let editor = EditorKeymap {
            insert_newline: resolve_local!(keymap, defaults, editor, insert_newline),
            move_left: resolve_local!(keymap, defaults, editor, move_left),
            move_right: resolve_local!(keymap, defaults, editor, move_right),
            move_up: resolve_local!(keymap, defaults, editor, move_up),
            move_down: resolve_local!(keymap, defaults, editor, move_down),
            move_word_left: resolve_local!(keymap, defaults, editor, move_word_left),
            move_word_right: resolve_local!(keymap, defaults, editor, move_word_right),
            move_line_start: resolve_local!(keymap, defaults, editor, move_line_start),
            move_line_end: resolve_local!(keymap, defaults, editor, move_line_end),
            delete_backward: resolve_local!(keymap, defaults, editor, delete_backward),
            delete_forward: resolve_local!(keymap, defaults, editor, delete_forward),
            delete_backward_word: resolve_local!(keymap, defaults, editor, delete_backward_word),
            delete_forward_word: resolve_local!(keymap, defaults, editor, delete_forward_word),
            kill_line_start: resolve_local!(keymap, defaults, editor, kill_line_start),
            kill_whole_line: resolve_local!(keymap, defaults, editor, kill_whole_line),
            kill_line_end: resolve_local!(keymap, defaults, editor, kill_line_end),
            yank: resolve_local!(keymap, defaults, editor, yank),
        };

        let mut vim_normal = VimNormalKeymap {
            enter_insert: resolve_local!(keymap, defaults, vim_normal, enter_insert),
            append_after_cursor: resolve_local!(keymap, defaults, vim_normal, append_after_cursor),
            append_line_end: resolve_local!(keymap, defaults, vim_normal, append_line_end),
            insert_line_start: resolve_local!(keymap, defaults, vim_normal, insert_line_start),
            open_line_below: resolve_local!(keymap, defaults, vim_normal, open_line_below),
            open_line_above: resolve_local!(keymap, defaults, vim_normal, open_line_above),
            move_left: resolve_local!(keymap, defaults, vim_normal, move_left),
            move_right: resolve_local!(keymap, defaults, vim_normal, move_right),
            move_up: resolve_local!(keymap, defaults, vim_normal, move_up),
            move_down: resolve_local!(keymap, defaults, vim_normal, move_down),
            move_word_forward: resolve_local!(keymap, defaults, vim_normal, move_word_forward),
            move_word_backward: resolve_local!(keymap, defaults, vim_normal, move_word_backward),
            move_word_end: resolve_local!(keymap, defaults, vim_normal, move_word_end),
            move_line_start: resolve_local!(keymap, defaults, vim_normal, move_line_start),
            move_line_end: resolve_local!(keymap, defaults, vim_normal, move_line_end),
            delete_char: resolve_local!(keymap, defaults, vim_normal, delete_char),
            substitute_char: resolve_local!(keymap, defaults, vim_normal, substitute_char),
            delete_to_line_end: resolve_local!(keymap, defaults, vim_normal, delete_to_line_end),
            change_to_line_end: resolve_local!(keymap, defaults, vim_normal, change_to_line_end),
            yank_line: resolve_local!(keymap, defaults, vim_normal, yank_line),
            paste_after: resolve_local!(keymap, defaults, vim_normal, paste_after),
            start_delete_operator: resolve_local!(
                keymap,
                defaults,
                vim_normal,
                start_delete_operator
            ),
            start_yank_operator: resolve_local!(keymap, defaults, vim_normal, start_yank_operator),
            start_change_operator: resolve_local!(
                keymap,
                defaults,
                vim_normal,
                start_change_operator
            ),
            cancel_operator: resolve_local!(keymap, defaults, vim_normal, cancel_operator),
        };

        let configured_vim_normal_bindings_to_preserve = configured_bindings_to_preserve([
            (
                keymap.vim_normal.enter_insert.as_ref(),
                vim_normal.enter_insert.as_slice(),
            ),
            (
                keymap.vim_normal.append_after_cursor.as_ref(),
                vim_normal.append_after_cursor.as_slice(),
            ),
            (
                keymap.vim_normal.append_line_end.as_ref(),
                vim_normal.append_line_end.as_slice(),
            ),
            (
                keymap.vim_normal.insert_line_start.as_ref(),
                vim_normal.insert_line_start.as_slice(),
            ),
            (
                keymap.vim_normal.open_line_below.as_ref(),
                vim_normal.open_line_below.as_slice(),
            ),
            (
                keymap.vim_normal.open_line_above.as_ref(),
                vim_normal.open_line_above.as_slice(),
            ),
            (
                keymap.vim_normal.move_left.as_ref(),
                vim_normal.move_left.as_slice(),
            ),
            (
                keymap.vim_normal.move_right.as_ref(),
                vim_normal.move_right.as_slice(),
            ),
            (
                keymap.vim_normal.move_up.as_ref(),
                vim_normal.move_up.as_slice(),
            ),
            (
                keymap.vim_normal.move_down.as_ref(),
                vim_normal.move_down.as_slice(),
            ),
            (
                keymap.vim_normal.move_word_forward.as_ref(),
                vim_normal.move_word_forward.as_slice(),
            ),
            (
                keymap.vim_normal.move_word_backward.as_ref(),
                vim_normal.move_word_backward.as_slice(),
            ),
            (
                keymap.vim_normal.move_word_end.as_ref(),
                vim_normal.move_word_end.as_slice(),
            ),
            (
                keymap.vim_normal.move_line_start.as_ref(),
                vim_normal.move_line_start.as_slice(),
            ),
            (
                keymap.vim_normal.move_line_end.as_ref(),
                vim_normal.move_line_end.as_slice(),
            ),
            (
                keymap.vim_normal.delete_char.as_ref(),
                vim_normal.delete_char.as_slice(),
            ),
            (
                keymap.vim_normal.change_to_line_end.as_ref(),
                vim_normal.change_to_line_end.as_slice(),
            ),
            (
                keymap.vim_normal.delete_to_line_end.as_ref(),
                vim_normal.delete_to_line_end.as_slice(),
            ),
            (
                keymap.vim_normal.yank_line.as_ref(),
                vim_normal.yank_line.as_slice(),
            ),
            (
                keymap.vim_normal.paste_after.as_ref(),
                vim_normal.paste_after.as_slice(),
            ),
            (
                keymap.vim_normal.start_delete_operator.as_ref(),
                vim_normal.start_delete_operator.as_slice(),
            ),
            (
                keymap.vim_normal.start_yank_operator.as_ref(),
                vim_normal.start_yank_operator.as_slice(),
            ),
            (
                keymap.vim_normal.start_change_operator.as_ref(),
                vim_normal.start_change_operator.as_slice(),
            ),
            (
                keymap.vim_normal.cancel_operator.as_ref(),
                vim_normal.cancel_operator.as_slice(),
            ),
        ]);

        if keymap.vim_normal.start_change_operator.is_none() {
            vim_normal
                .start_change_operator
                .retain(|binding| !configured_vim_normal_bindings_to_preserve.contains(binding));
        }
        if keymap.vim_normal.substitute_char.is_none() {
            vim_normal
                .substitute_char
                .retain(|binding| !configured_vim_normal_bindings_to_preserve.contains(binding));
        }

        let mut vim_operator = VimOperatorKeymap {
            delete_line: resolve_local!(keymap, defaults, vim_operator, delete_line),
            yank_line: resolve_local!(keymap, defaults, vim_operator, yank_line),
            motion_left: resolve_local!(keymap, defaults, vim_operator, motion_left),
            motion_right: resolve_local!(keymap, defaults, vim_operator, motion_right),
            motion_up: resolve_local!(keymap, defaults, vim_operator, motion_up),
            motion_down: resolve_local!(keymap, defaults, vim_operator, motion_down),
            motion_word_forward: resolve_local!(
                keymap,
                defaults,
                vim_operator,
                motion_word_forward
            ),
            motion_word_backward: resolve_local!(
                keymap,
                defaults,
                vim_operator,
                motion_word_backward
            ),
            motion_word_end: resolve_local!(keymap, defaults, vim_operator, motion_word_end),
            motion_line_start: resolve_local!(keymap, defaults, vim_operator, motion_line_start),
            motion_line_end: resolve_local!(keymap, defaults, vim_operator, motion_line_end),
            select_inner_text_object: resolve_local!(
                keymap,
                defaults,
                vim_operator,
                select_inner_text_object
            ),
            select_around_text_object: resolve_local!(
                keymap,
                defaults,
                vim_operator,
                select_around_text_object
            ),
            cancel: resolve_local!(keymap, defaults, vim_operator, cancel),
        };

        let configured_vim_operator_bindings_to_preserve = configured_bindings_to_preserve([
            (
                keymap.vim_operator.delete_line.as_ref(),
                vim_operator.delete_line.as_slice(),
            ),
            (
                keymap.vim_operator.yank_line.as_ref(),
                vim_operator.yank_line.as_slice(),
            ),
            (
                keymap.vim_operator.motion_left.as_ref(),
                vim_operator.motion_left.as_slice(),
            ),
            (
                keymap.vim_operator.motion_right.as_ref(),
                vim_operator.motion_right.as_slice(),
            ),
            (
                keymap.vim_operator.motion_up.as_ref(),
                vim_operator.motion_up.as_slice(),
            ),
            (
                keymap.vim_operator.motion_down.as_ref(),
                vim_operator.motion_down.as_slice(),
            ),
            (
                keymap.vim_operator.motion_word_forward.as_ref(),
                vim_operator.motion_word_forward.as_slice(),
            ),
            (
                keymap.vim_operator.motion_word_backward.as_ref(),
                vim_operator.motion_word_backward.as_slice(),
            ),
            (
                keymap.vim_operator.motion_word_end.as_ref(),
                vim_operator.motion_word_end.as_slice(),
            ),
            (
                keymap.vim_operator.motion_line_start.as_ref(),
                vim_operator.motion_line_start.as_slice(),
            ),
            (
                keymap.vim_operator.motion_line_end.as_ref(),
                vim_operator.motion_line_end.as_slice(),
            ),
            (
                keymap.vim_operator.cancel.as_ref(),
                vim_operator.cancel.as_slice(),
            ),
        ]);

        if keymap.vim_operator.select_inner_text_object.is_none() {
            vim_operator
                .select_inner_text_object
                .retain(|binding| !configured_vim_operator_bindings_to_preserve.contains(binding));
        }
        if keymap.vim_operator.select_around_text_object.is_none() {
            vim_operator
                .select_around_text_object
                .retain(|binding| !configured_vim_operator_bindings_to_preserve.contains(binding));
        }

        let vim_text_object = VimTextObjectKeymap {
            word: resolve_local!(keymap, defaults, vim_text_object, word),
            big_word: resolve_local!(keymap, defaults, vim_text_object, big_word),
            parentheses: resolve_local!(keymap, defaults, vim_text_object, parentheses),
            brackets: resolve_local!(keymap, defaults, vim_text_object, brackets),
            braces: resolve_local!(keymap, defaults, vim_text_object, braces),
            double_quote: resolve_local!(keymap, defaults, vim_text_object, double_quote),
            single_quote: resolve_local!(keymap, defaults, vim_text_object, single_quote),
            backtick: resolve_local!(keymap, defaults, vim_text_object, backtick),
            cancel: resolve_local!(keymap, defaults, vim_text_object, cancel),
        };

        // 推理箭头别名是回退默认值：同一输入路径上的已有显式
        // 绑定保留按键，而显式推理绑定保持权威。
        if keymap.chat.decrease_reasoning_effort.is_none()
            && configured_main_surface_alias_is_used(keymap, "shift-down")
        {
            chat.decrease_reasoning_effort
                .retain(|binding| *binding != key_hint::shift(KeyCode::Down));
        }
        if keymap.chat.increase_reasoning_effort.is_none()
            && configured_main_surface_alias_is_used(keymap, "shift-up")
        {
            chat.increase_reasoning_effort
                .retain(|binding| *binding != key_hint::shift(KeyCode::Up));
        }

        let pager = PagerKeymap {
            scroll_up: resolve_local!(keymap, defaults, pager, scroll_up),
            scroll_down: resolve_local!(keymap, defaults, pager, scroll_down),
            page_up: resolve_local!(keymap, defaults, pager, page_up),
            page_down: resolve_local!(keymap, defaults, pager, page_down),
            half_page_up: resolve_local!(keymap, defaults, pager, half_page_up),
            half_page_down: resolve_local!(keymap, defaults, pager, half_page_down),
            jump_top: resolve_local!(keymap, defaults, pager, jump_top),
            jump_bottom: resolve_local!(keymap, defaults, pager, jump_bottom),
            close: resolve_local!(keymap, defaults, pager, close),
            close_transcript: resolve_local!(keymap, defaults, pager, close_transcript),
        };

        let approval = ApprovalKeymap {
            open_fullscreen: resolve_local!(keymap, defaults, approval, open_fullscreen),
            open_thread: resolve_local!(keymap, defaults, approval, open_thread),
            approve: resolve_local!(keymap, defaults, approval, approve),
            approve_for_session: resolve_local!(keymap, defaults, approval, approve_for_session),
            approve_for_prefix: resolve_local!(keymap, defaults, approval, approve_for_prefix),
            deny: resolve_local!(keymap, defaults, approval, deny),
            decline: resolve_local!(keymap, defaults, approval, decline),
            cancel: resolve_local!(keymap, defaults, approval, cancel),
        };

        let list_move_up = resolve_local!(keymap, defaults, list, move_up);
        let list_move_down = resolve_local!(keymap, defaults, list, move_down);
        let list_accept = resolve_local!(keymap, defaults, list, accept);
        let list_cancel = resolve_local!(keymap, defaults, list, cancel);
        let configured_bindings_to_preserve = configured_bindings_to_preserve([
            (
                keymap.global.open_transcript.as_ref(),
                app.open_transcript.as_slice(),
            ),
            (
                keymap.global.open_external_editor.as_ref(),
                app.open_external_editor.as_slice(),
            ),
            (keymap.global.copy.as_ref(), app.copy.as_slice()),
            (
                keymap.global.clear_terminal.as_ref(),
                app.clear_terminal.as_slice(),
            ),
            (
                keymap.global.toggle_vim_mode.as_ref(),
                app.toggle_vim_mode.as_slice(),
            ),
            (
                keymap.global.toggle_fast_mode.as_ref(),
                app.toggle_fast_mode.as_slice(),
            ),
            (
                keymap.global.toggle_raw_output.as_ref(),
                app.toggle_raw_output.as_slice(),
            ),
            (keymap.list.move_up.as_ref(), list_move_up.as_slice()),
            (keymap.list.move_down.as_ref(), list_move_down.as_slice()),
            (keymap.list.accept.as_ref(), list_accept.as_slice()),
            (keymap.list.cancel.as_ref(), list_cancel.as_slice()),
            (
                keymap.approval.open_fullscreen.as_ref(),
                approval.open_fullscreen.as_slice(),
            ),
            (
                keymap.approval.open_thread.as_ref(),
                approval.open_thread.as_slice(),
            ),
            (
                keymap.approval.approve.as_ref(),
                approval.approve.as_slice(),
            ),
            (
                keymap.approval.approve_for_session.as_ref(),
                approval.approve_for_session.as_slice(),
            ),
            (
                keymap.approval.approve_for_prefix.as_ref(),
                approval.approve_for_prefix.as_slice(),
            ),
            (keymap.approval.deny.as_ref(), approval.deny.as_slice()),
            (
                keymap.approval.decline.as_ref(),
                approval.decline.as_slice(),
            ),
            (keymap.approval.cancel.as_ref(), approval.cancel.as_slice()),
        ]);

        let list = ListKeymap {
            move_up: list_move_up,
            move_down: list_move_down,
            move_left: resolve_new_default_bindings(
                keymap.list.move_left.as_ref(),
                &defaults.list.move_left,
                &configured_bindings_to_preserve,
                "tui.keymap.list.move_left",
            )?,
            move_right: resolve_new_default_bindings(
                keymap.list.move_right.as_ref(),
                &defaults.list.move_right,
                &configured_bindings_to_preserve,
                "tui.keymap.list.move_right",
            )?,
            page_up: resolve_new_default_bindings(
                keymap.list.page_up.as_ref(),
                &defaults.list.page_up,
                &configured_bindings_to_preserve,
                "tui.keymap.list.page_up",
            )?,
            page_down: resolve_new_default_bindings(
                keymap.list.page_down.as_ref(),
                &defaults.list.page_down,
                &configured_bindings_to_preserve,
                "tui.keymap.list.page_down",
            )?,
            jump_top: resolve_new_default_bindings(
                keymap.list.jump_top.as_ref(),
                &defaults.list.jump_top,
                &configured_bindings_to_preserve,
                "tui.keymap.list.jump_top",
            )?,
            jump_bottom: resolve_new_default_bindings(
                keymap.list.jump_bottom.as_ref(),
                &defaults.list.jump_bottom,
                &configured_bindings_to_preserve,
                "tui.keymap.list.jump_bottom",
            )?,
            accept: list_accept,
            cancel: list_cancel,
        };

        let resolved = Self {
            app,
            chat,
            composer,
            editor,
            vim_normal,
            vim_operator,
            vim_text_object,
            pager,
            list,
            approval,
        };

        resolved.validate_conflicts()?;
        Ok(resolved)
    }

    /// 内置键映射默认值。
    ///
    /// 部分动作有意包含兼容性变体（例如 `?` 和 `shift-?` 都有），因为终端对
    /// 某些可打印/控制组合键是否保留 SHIFT 存在分歧。
    fn built_in_defaults() -> Self {
        Self {
            app: AppKeymap {
                open_transcript: default_bindings![ctrl(KeyCode::Char('t'))],
                open_external_editor: default_bindings![ctrl(KeyCode::Char('g'))],
                copy: default_bindings![ctrl(KeyCode::Char('o'))],
                clear_terminal: default_bindings![ctrl(KeyCode::Char('l'))],
                toggle_vim_mode: default_bindings![],
                toggle_fast_mode: default_bindings![],
                toggle_raw_output: default_bindings![alt(KeyCode::Char('r'))],
            },
            chat: ChatKeymap {
                interrupt_turn: default_bindings![plain(KeyCode::Esc)],
                decrease_reasoning_effort: default_bindings![
                    alt(KeyCode::Char(',')),
                    shift(KeyCode::Down)
                ],
                increase_reasoning_effort: default_bindings![
                    alt(KeyCode::Char('.')),
                    shift(KeyCode::Up)
                ],
                edit_queued_message: default_bindings![alt(KeyCode::Up), shift(KeyCode::Left)],
            },
            composer: ComposerKeymap {
                submit: default_bindings![plain(KeyCode::Enter)],
                queue: default_bindings![plain(KeyCode::Tab)],
                toggle_shortcuts: default_bindings![
                    plain(KeyCode::Char('?')),
                    shift(KeyCode::Char('?'))
                ],
                history_search_previous: default_bindings![ctrl(KeyCode::Char('r'))],
                history_search_next: default_bindings![ctrl(KeyCode::Char('s'))],
            },
            editor: EditorKeymap {
                insert_newline: default_bindings![
                    ctrl(KeyCode::Char('j')),
                    ctrl(KeyCode::Char('m')),
                    plain(KeyCode::Enter),
                    shift(KeyCode::Enter),
                    alt(KeyCode::Enter)
                ],
                move_left: default_bindings![plain(KeyCode::Left), ctrl(KeyCode::Char('b'))],
                move_right: default_bindings![plain(KeyCode::Right), ctrl(KeyCode::Char('f'))],
                move_up: default_bindings![plain(KeyCode::Up), ctrl(KeyCode::Char('p'))],
                move_down: default_bindings![plain(KeyCode::Down), ctrl(KeyCode::Char('n'))],
                move_word_left: default_bindings![
                    alt(KeyCode::Char('b')),
                    raw(KeyBinding::new(KeyCode::Left, KeyModifiers::ALT)),
                    raw(KeyBinding::new(KeyCode::Left, KeyModifiers::CONTROL))
                ],
                move_word_right: default_bindings![
                    alt(KeyCode::Char('f')),
                    raw(KeyBinding::new(KeyCode::Right, KeyModifiers::ALT)),
                    raw(KeyBinding::new(KeyCode::Right, KeyModifiers::CONTROL))
                ],
                move_line_start: default_bindings![plain(KeyCode::Home), ctrl(KeyCode::Char('a'))],
                move_line_end: default_bindings![plain(KeyCode::End), ctrl(KeyCode::Char('e'))],
                delete_backward: default_bindings![
                    plain(KeyCode::Backspace),
                    shift(KeyCode::Backspace),
                    ctrl(KeyCode::Char('h'))
                ],
                delete_forward: default_bindings![
                    plain(KeyCode::Delete),
                    shift(KeyCode::Delete),
                    ctrl(KeyCode::Char('d'))
                ],
                delete_backward_word: default_bindings![
                    alt(KeyCode::Backspace),
                    ctrl(KeyCode::Backspace),
                    raw(KeyBinding::new(
                        KeyCode::Backspace,
                        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                    )),
                    ctrl(KeyCode::Char('w')),
                    raw(KeyBinding::new(
                        KeyCode::Char('h'),
                        KeyModifiers::CONTROL | KeyModifiers::ALT,
                    ))
                ],
                delete_forward_word: default_bindings![
                    alt(KeyCode::Delete),
                    ctrl(KeyCode::Delete),
                    raw(KeyBinding::new(
                        KeyCode::Delete,
                        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                    )),
                    alt(KeyCode::Char('d'))
                ],
                kill_line_start: default_bindings![ctrl(KeyCode::Char('u'))],
                kill_whole_line: default_bindings![],
                kill_line_end: default_bindings![ctrl(KeyCode::Char('k'))],
                yank: default_bindings![ctrl(KeyCode::Char('y'))],
            },
            vim_normal: VimNormalKeymap {
                enter_insert: default_bindings![plain(KeyCode::Char('i')), plain(KeyCode::Insert)],
                append_after_cursor: default_bindings![plain(KeyCode::Char('a'))],
                append_line_end: default_bindings![
                    shift(KeyCode::Char('a')),
                    plain(KeyCode::Char('A'))
                ],
                insert_line_start: default_bindings![
                    shift(KeyCode::Char('i')),
                    plain(KeyCode::Char('I'))
                ],
                open_line_below: default_bindings![plain(KeyCode::Char('o'))],
                open_line_above: default_bindings![
                    shift(KeyCode::Char('o')),
                    plain(KeyCode::Char('O'))
                ],
                move_left: default_bindings![plain(KeyCode::Char('h')), plain(KeyCode::Left)],
                move_right: default_bindings![plain(KeyCode::Char('l')), plain(KeyCode::Right)],
                move_up: default_bindings![plain(KeyCode::Char('k')), plain(KeyCode::Up)],
                move_down: default_bindings![plain(KeyCode::Char('j')), plain(KeyCode::Down)],
                move_word_forward: default_bindings![plain(KeyCode::Char('w'))],
                move_word_backward: default_bindings![plain(KeyCode::Char('b'))],
                move_word_end: default_bindings![plain(KeyCode::Char('e'))],
                move_line_start: default_bindings![plain(KeyCode::Char('0'))],
                move_line_end: default_bindings![
                    plain(KeyCode::Char('$')),
                    shift(KeyCode::Char('$'))
                ],
                delete_char: default_bindings![plain(KeyCode::Char('x'))],
                substitute_char: default_bindings![plain(KeyCode::Char('s'))],
                delete_to_line_end: default_bindings![
                    shift(KeyCode::Char('d')),
                    plain(KeyCode::Char('D'))
                ],
                change_to_line_end: default_bindings![
                    shift(KeyCode::Char('c')),
                    plain(KeyCode::Char('C'))
                ],
                yank_line: default_bindings![shift(KeyCode::Char('y')), plain(KeyCode::Char('Y'))],
                paste_after: default_bindings![plain(KeyCode::Char('p'))],
                start_delete_operator: default_bindings![plain(KeyCode::Char('d'))],
                start_yank_operator: default_bindings![plain(KeyCode::Char('y'))],
                start_change_operator: default_bindings![plain(KeyCode::Char('c'))],
                cancel_operator: default_bindings![plain(KeyCode::Esc)],
            },
            vim_operator: VimOperatorKeymap {
                delete_line: default_bindings![plain(KeyCode::Char('d'))],
                yank_line: default_bindings![plain(KeyCode::Char('y'))],
                motion_left: default_bindings![plain(KeyCode::Char('h'))],
                motion_right: default_bindings![plain(KeyCode::Char('l'))],
                motion_up: default_bindings![plain(KeyCode::Char('k'))],
                motion_down: default_bindings![plain(KeyCode::Char('j'))],
                motion_word_forward: default_bindings![plain(KeyCode::Char('w'))],
                motion_word_backward: default_bindings![plain(KeyCode::Char('b'))],
                motion_word_end: default_bindings![plain(KeyCode::Char('e'))],
                motion_line_start: default_bindings![plain(KeyCode::Char('0'))],
                motion_line_end: default_bindings![
                    plain(KeyCode::Char('$')),
                    shift(KeyCode::Char('$'))
                ],
                select_inner_text_object: default_bindings![plain(KeyCode::Char('i'))],
                select_around_text_object: default_bindings![plain(KeyCode::Char('a'))],
                cancel: default_bindings![plain(KeyCode::Esc)],
            },
            vim_text_object: VimTextObjectKeymap {
                word: default_bindings![plain(KeyCode::Char('w'))],
                big_word: default_bindings![shift(KeyCode::Char('w')), plain(KeyCode::Char('W'))],
                parentheses: default_bindings![
                    plain(KeyCode::Char('(')),
                    shift(KeyCode::Char('(')),
                    plain(KeyCode::Char(')')),
                    shift(KeyCode::Char(')')),
                    plain(KeyCode::Char('b'))
                ],
                brackets: default_bindings![plain(KeyCode::Char('[')), plain(KeyCode::Char(']'))],
                braces: default_bindings![
                    plain(KeyCode::Char('{')),
                    shift(KeyCode::Char('{')),
                    plain(KeyCode::Char('}')),
                    shift(KeyCode::Char('}')),
                    shift(KeyCode::Char('b')),
                    plain(KeyCode::Char('B'))
                ],
                double_quote: default_bindings![
                    plain(KeyCode::Char('"')),
                    shift(KeyCode::Char('"'))
                ],
                single_quote: default_bindings![plain(KeyCode::Char('\''))],
                backtick: default_bindings![plain(KeyCode::Char('`'))],
                cancel: default_bindings![plain(KeyCode::Esc)],
            },
            pager: PagerKeymap {
                scroll_up: default_bindings![plain(KeyCode::Up), plain(KeyCode::Char('k'))],
                scroll_down: default_bindings![plain(KeyCode::Down), plain(KeyCode::Char('j'))],
                page_up: default_bindings![
                    plain(KeyCode::PageUp),
                    shift(KeyCode::Char(' ')),
                    ctrl(KeyCode::Char('b'))
                ],
                page_down: default_bindings![
                    plain(KeyCode::PageDown),
                    plain(KeyCode::Char(' ')),
                    ctrl(KeyCode::Char('f'))
                ],
                half_page_up: default_bindings![ctrl(KeyCode::Char('u'))],
                half_page_down: default_bindings![ctrl(KeyCode::Char('d'))],
                jump_top: default_bindings![plain(KeyCode::Home)],
                jump_bottom: default_bindings![plain(KeyCode::End)],
                close: default_bindings![plain(KeyCode::Char('q')), ctrl(KeyCode::Char('c'))],
                close_transcript: default_bindings![ctrl(KeyCode::Char('t'))],
            },
            list: ListKeymap {
                move_up: default_bindings![
                    plain(KeyCode::Up),
                    ctrl(KeyCode::Char('p')),
                    ctrl(KeyCode::Char('k')),
                    plain(KeyCode::Char('k'))
                ],
                move_down: default_bindings![
                    plain(KeyCode::Down),
                    ctrl(KeyCode::Char('n')),
                    ctrl(KeyCode::Char('j')),
                    plain(KeyCode::Char('j'))
                ],
                move_left: default_bindings![plain(KeyCode::Left), ctrl(KeyCode::Char('h'))],
                move_right: default_bindings![plain(KeyCode::Right), ctrl(KeyCode::Char('l'))],
                page_up: default_bindings![plain(KeyCode::PageUp), ctrl(KeyCode::Char('b'))],
                page_down: default_bindings![plain(KeyCode::PageDown), ctrl(KeyCode::Char('f'))],
                jump_top: default_bindings![plain(KeyCode::Home)],
                jump_bottom: default_bindings![plain(KeyCode::End)],
                accept: default_bindings![plain(KeyCode::Enter)],
                cancel: default_bindings![plain(KeyCode::Esc)],
            },
            approval: ApprovalKeymap {
                open_fullscreen: default_bindings![
                    ctrl(KeyCode::Char('a')),
                    raw(KeyBinding::new(
                        KeyCode::Char('a'),
                        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                    ))
                ],
                open_thread: default_bindings![plain(KeyCode::Char('o'))],
                approve: default_bindings![plain(KeyCode::Char('y'))],
                approve_for_session: default_bindings![plain(KeyCode::Char('a'))],
                approve_for_prefix: default_bindings![plain(KeyCode::Char('p'))],
                deny: default_bindings![plain(KeyCode::Char('d'))],
                decline: default_bindings![plain(KeyCode::Esc), plain(KeyCode::Char('n'))],
                cancel: default_bindings![plain(KeyCode::Char('c'))],
            },
        }
    }

    /// 拒绝在同一作用域中被求值的歧义绑定。
    ///
    /// 我们分多轮验证，因为运行时处理具有混合优先级：
    ///
    /// 1. `app` 动作可遮蔽 composer 动作，因为应用检查在转发到
    ///    composer 之前运行。
    /// 2. 具有硬编码序列行为的上下文（如编辑上一条回溯）
    ///    有意留在此可配置键映射之外。
    fn validate_conflicts(&self) -> Result<(), String> {
        validate_unique(
            "app",
            [
                ("open_transcript", self.app.open_transcript.as_slice()),
                (
                    "open_external_editor",
                    self.app.open_external_editor.as_slice(),
                ),
                ("copy", self.app.copy.as_slice()),
                ("clear_terminal", self.app.clear_terminal.as_slice()),
                ("toggle_vim_mode", self.app.toggle_vim_mode.as_slice()),
                ("toggle_fast_mode", self.app.toggle_fast_mode.as_slice()),
                ("toggle_raw_output", self.app.toggle_raw_output.as_slice()),
                ("chat.interrupt_turn", self.chat.interrupt_turn.as_slice()),
                (
                    "chat.decrease_reasoning_effort",
                    self.chat.decrease_reasoning_effort.as_slice(),
                ),
                (
                    "chat.increase_reasoning_effort",
                    self.chat.increase_reasoning_effort.as_slice(),
                ),
                (
                    "chat.edit_queued_message",
                    self.chat.edit_queued_message.as_slice(),
                ),
                ("composer.submit", self.composer.submit.as_slice()),
                ("composer.queue", self.composer.queue.as_slice()),
                (
                    "composer.toggle_shortcuts",
                    self.composer.toggle_shortcuts.as_slice(),
                ),
                (
                    "composer.history_search_previous",
                    self.composer.history_search_previous.as_slice(),
                ),
                (
                    "composer.history_search_next",
                    self.composer.history_search_next.as_slice(),
                ),
            ],
        )?;

        validate_no_reserved(
            "main",
            [
                ("open_transcript", self.app.open_transcript.as_slice()),
                (
                    "open_external_editor",
                    self.app.open_external_editor.as_slice(),
                ),
                ("copy", self.app.copy.as_slice()),
                ("clear_terminal", self.app.clear_terminal.as_slice()),
                ("toggle_vim_mode", self.app.toggle_vim_mode.as_slice()),
                ("toggle_fast_mode", self.app.toggle_fast_mode.as_slice()),
                ("toggle_raw_output", self.app.toggle_raw_output.as_slice()),
                ("chat.interrupt_turn", self.chat.interrupt_turn.as_slice()),
                (
                    "chat.decrease_reasoning_effort",
                    self.chat.decrease_reasoning_effort.as_slice(),
                ),
                (
                    "chat.increase_reasoning_effort",
                    self.chat.increase_reasoning_effort.as_slice(),
                ),
                (
                    "chat.edit_queued_message",
                    self.chat.edit_queued_message.as_slice(),
                ),
                ("composer.submit", self.composer.submit.as_slice()),
                ("composer.queue", self.composer.queue.as_slice()),
                (
                    "composer.toggle_shortcuts",
                    self.composer.toggle_shortcuts.as_slice(),
                ),
                (
                    "composer.history_search_previous",
                    self.composer.history_search_previous.as_slice(),
                ),
                (
                    "composer.history_search_next",
                    self.composer.history_search_next.as_slice(),
                ),
            ],
            MAIN_RESERVED_BINDINGS,
            [(
                "chat.interrupt_turn",
                "fixed.backtrack",
                key_hint::plain(KeyCode::Esc),
            )],
        )?;

        validate_no_shadow_with_allowed_overlaps(
            "app",
            [
                ("open_transcript", self.app.open_transcript.as_slice()),
                (
                    "open_external_editor",
                    self.app.open_external_editor.as_slice(),
                ),
                ("copy", self.app.copy.as_slice()),
                ("clear_terminal", self.app.clear_terminal.as_slice()),
                ("toggle_vim_mode", self.app.toggle_vim_mode.as_slice()),
                ("toggle_fast_mode", self.app.toggle_fast_mode.as_slice()),
                ("toggle_raw_output", self.app.toggle_raw_output.as_slice()),
            ],
            [
                ("list.move_up", self.list.move_up.as_slice()),
                ("list.move_down", self.list.move_down.as_slice()),
                ("list.move_left", self.list.move_left.as_slice()),
                ("list.move_right", self.list.move_right.as_slice()),
                ("list.page_up", self.list.page_up.as_slice()),
                ("list.page_down", self.list.page_down.as_slice()),
                ("list.jump_top", self.list.jump_top.as_slice()),
                ("list.jump_bottom", self.list.jump_bottom.as_slice()),
                ("list.accept", self.list.accept.as_slice()),
                ("list.cancel", self.list.cancel.as_slice()),
                (
                    "approval.open_fullscreen",
                    self.approval.open_fullscreen.as_slice(),
                ),
                ("approval.open_thread", self.approval.open_thread.as_slice()),
                ("approval.approve", self.approval.approve.as_slice()),
                (
                    "approval.approve_for_session",
                    self.approval.approve_for_session.as_slice(),
                ),
                (
                    "approval.approve_for_prefix",
                    self.approval.approve_for_prefix.as_slice(),
                ),
                ("approval.deny", self.approval.deny.as_slice()),
                ("approval.decline", self.approval.decline.as_slice()),
                ("approval.cancel", self.approval.cancel.as_slice()),
            ],
            [(
                "clear_terminal",
                "list.move_right",
                key_hint::ctrl(KeyCode::Char('l')),
            )],
        )?;

        // 请求用户输入覆盖层在可配置的问题导航到达其列表处理器之前
        // 消费回合中断。
        validate_no_shadow_with_allowed_overlaps(
            "request_user_input",
            [("chat.interrupt_turn", self.chat.interrupt_turn.as_slice())],
            [
                ("list.move_left", self.list.move_left.as_slice()),
                ("list.move_right", self.list.move_right.as_slice()),
            ],
            [],
        )?;

        // composer 聚焦期间，这些主界面处理器会
        // 在事件到达 textarea 编辑器之前先消费匹配的按键。
        validate_no_shadow_with_allowed_overlaps(
            "main",
            [
                ("open_transcript", self.app.open_transcript.as_slice()),
                (
                    "open_external_editor",
                    self.app.open_external_editor.as_slice(),
                ),
                ("copy", self.app.copy.as_slice()),
                ("clear_terminal", self.app.clear_terminal.as_slice()),
                ("chat.interrupt_turn", self.chat.interrupt_turn.as_slice()),
                (
                    "chat.decrease_reasoning_effort",
                    self.chat.decrease_reasoning_effort.as_slice(),
                ),
                (
                    "chat.increase_reasoning_effort",
                    self.chat.increase_reasoning_effort.as_slice(),
                ),
                ("composer.submit", self.composer.submit.as_slice()),
                ("toggle_vim_mode", self.app.toggle_vim_mode.as_slice()),
                ("toggle_fast_mode", self.app.toggle_fast_mode.as_slice()),
                ("toggle_raw_output", self.app.toggle_raw_output.as_slice()),
                (
                    "composer.history_search_previous",
                    self.composer.history_search_previous.as_slice(),
                ),
            ],
            [
                (
                    "editor.insert_newline",
                    self.editor.insert_newline.as_slice(),
                ),
                ("editor.move_left", self.editor.move_left.as_slice()),
                ("editor.move_right", self.editor.move_right.as_slice()),
                ("editor.move_up", self.editor.move_up.as_slice()),
                ("editor.move_down", self.editor.move_down.as_slice()),
                (
                    "editor.move_word_left",
                    self.editor.move_word_left.as_slice(),
                ),
                (
                    "editor.move_word_right",
                    self.editor.move_word_right.as_slice(),
                ),
                (
                    "editor.move_line_start",
                    self.editor.move_line_start.as_slice(),
                ),
                ("editor.move_line_end", self.editor.move_line_end.as_slice()),
                (
                    "editor.delete_backward",
                    self.editor.delete_backward.as_slice(),
                ),
                (
                    "editor.delete_forward",
                    self.editor.delete_forward.as_slice(),
                ),
                (
                    "editor.delete_backward_word",
                    self.editor.delete_backward_word.as_slice(),
                ),
                (
                    "editor.delete_forward_word",
                    self.editor.delete_forward_word.as_slice(),
                ),
                (
                    "editor.kill_line_start",
                    self.editor.kill_line_start.as_slice(),
                ),
                (
                    "editor.kill_whole_line",
                    self.editor.kill_whole_line.as_slice(),
                ),
                ("editor.kill_line_end", self.editor.kill_line_end.as_slice()),
                ("editor.yank", self.editor.yank.as_slice()),
            ],
            [(
                "composer.submit",
                "editor.insert_newline",
                key_hint::plain(KeyCode::Enter),
            )],
        )?;

        validate_unique(
            "editor",
            [
                ("insert_newline", self.editor.insert_newline.as_slice()),
                ("move_left", self.editor.move_left.as_slice()),
                ("move_right", self.editor.move_right.as_slice()),
                ("move_up", self.editor.move_up.as_slice()),
                ("move_down", self.editor.move_down.as_slice()),
                ("move_word_left", self.editor.move_word_left.as_slice()),
                ("move_word_right", self.editor.move_word_right.as_slice()),
                ("move_line_start", self.editor.move_line_start.as_slice()),
                ("move_line_end", self.editor.move_line_end.as_slice()),
                ("delete_backward", self.editor.delete_backward.as_slice()),
                ("delete_forward", self.editor.delete_forward.as_slice()),
                (
                    "delete_backward_word",
                    self.editor.delete_backward_word.as_slice(),
                ),
                (
                    "delete_forward_word",
                    self.editor.delete_forward_word.as_slice(),
                ),
                ("kill_line_start", self.editor.kill_line_start.as_slice()),
                ("kill_whole_line", self.editor.kill_whole_line.as_slice()),
                ("kill_line_end", self.editor.kill_line_end.as_slice()),
                ("yank", self.editor.yank.as_slice()),
            ],
        )?;

        validate_unique(
            "vim_normal",
            [
                ("enter_insert", self.vim_normal.enter_insert.as_slice()),
                (
                    "append_after_cursor",
                    self.vim_normal.append_after_cursor.as_slice(),
                ),
                (
                    "append_line_end",
                    self.vim_normal.append_line_end.as_slice(),
                ),
                (
                    "insert_line_start",
                    self.vim_normal.insert_line_start.as_slice(),
                ),
                (
                    "open_line_below",
                    self.vim_normal.open_line_below.as_slice(),
                ),
                (
                    "open_line_above",
                    self.vim_normal.open_line_above.as_slice(),
                ),
                ("move_left", self.vim_normal.move_left.as_slice()),
                ("move_right", self.vim_normal.move_right.as_slice()),
                ("move_up", self.vim_normal.move_up.as_slice()),
                ("move_down", self.vim_normal.move_down.as_slice()),
                (
                    "move_word_forward",
                    self.vim_normal.move_word_forward.as_slice(),
                ),
                (
                    "move_word_backward",
                    self.vim_normal.move_word_backward.as_slice(),
                ),
                ("move_word_end", self.vim_normal.move_word_end.as_slice()),
                (
                    "move_line_start",
                    self.vim_normal.move_line_start.as_slice(),
                ),
                ("move_line_end", self.vim_normal.move_line_end.as_slice()),
                ("delete_char", self.vim_normal.delete_char.as_slice()),
                (
                    "substitute_char",
                    self.vim_normal.substitute_char.as_slice(),
                ),
                (
                    "delete_to_line_end",
                    self.vim_normal.delete_to_line_end.as_slice(),
                ),
                (
                    "change_to_line_end",
                    self.vim_normal.change_to_line_end.as_slice(),
                ),
                ("yank_line", self.vim_normal.yank_line.as_slice()),
                ("paste_after", self.vim_normal.paste_after.as_slice()),
                (
                    "start_delete_operator",
                    self.vim_normal.start_delete_operator.as_slice(),
                ),
                (
                    "start_yank_operator",
                    self.vim_normal.start_yank_operator.as_slice(),
                ),
                (
                    "start_change_operator",
                    self.vim_normal.start_change_operator.as_slice(),
                ),
                (
                    "cancel_operator",
                    self.vim_normal.cancel_operator.as_slice(),
                ),
            ],
        )?;

        validate_unique(
            "vim_operator",
            [
                ("delete_line", self.vim_operator.delete_line.as_slice()),
                ("yank_line", self.vim_operator.yank_line.as_slice()),
                ("motion_left", self.vim_operator.motion_left.as_slice()),
                ("motion_right", self.vim_operator.motion_right.as_slice()),
                ("motion_up", self.vim_operator.motion_up.as_slice()),
                ("motion_down", self.vim_operator.motion_down.as_slice()),
                (
                    "motion_word_forward",
                    self.vim_operator.motion_word_forward.as_slice(),
                ),
                (
                    "motion_word_backward",
                    self.vim_operator.motion_word_backward.as_slice(),
                ),
                (
                    "motion_word_end",
                    self.vim_operator.motion_word_end.as_slice(),
                ),
                (
                    "motion_line_start",
                    self.vim_operator.motion_line_start.as_slice(),
                ),
                (
                    "motion_line_end",
                    self.vim_operator.motion_line_end.as_slice(),
                ),
                (
                    "select_inner_text_object",
                    self.vim_operator.select_inner_text_object.as_slice(),
                ),
                (
                    "select_around_text_object",
                    self.vim_operator.select_around_text_object.as_slice(),
                ),
                ("cancel", self.vim_operator.cancel.as_slice()),
            ],
        )?;

        validate_unique(
            "vim_text_object",
            [
                ("word", self.vim_text_object.word.as_slice()),
                ("big_word", self.vim_text_object.big_word.as_slice()),
                ("parentheses", self.vim_text_object.parentheses.as_slice()),
                ("brackets", self.vim_text_object.brackets.as_slice()),
                ("braces", self.vim_text_object.braces.as_slice()),
                ("double_quote", self.vim_text_object.double_quote.as_slice()),
                ("single_quote", self.vim_text_object.single_quote.as_slice()),
                ("backtick", self.vim_text_object.backtick.as_slice()),
                ("cancel", self.vim_text_object.cancel.as_slice()),
            ],
        )?;

        validate_unique(
            "pager",
            [
                ("scroll_up", self.pager.scroll_up.as_slice()),
                ("scroll_down", self.pager.scroll_down.as_slice()),
                ("page_up", self.pager.page_up.as_slice()),
                ("page_down", self.pager.page_down.as_slice()),
                ("half_page_up", self.pager.half_page_up.as_slice()),
                ("half_page_down", self.pager.half_page_down.as_slice()),
                ("jump_top", self.pager.jump_top.as_slice()),
                ("jump_bottom", self.pager.jump_bottom.as_slice()),
                ("close", self.pager.close.as_slice()),
                ("close_transcript", self.pager.close_transcript.as_slice()),
            ],
        )?;

        validate_no_reserved(
            "pager",
            [
                ("scroll_up", self.pager.scroll_up.as_slice()),
                ("scroll_down", self.pager.scroll_down.as_slice()),
                ("page_up", self.pager.page_up.as_slice()),
                ("page_down", self.pager.page_down.as_slice()),
                ("half_page_up", self.pager.half_page_up.as_slice()),
                ("half_page_down", self.pager.half_page_down.as_slice()),
                ("jump_top", self.pager.jump_top.as_slice()),
                ("jump_bottom", self.pager.jump_bottom.as_slice()),
                ("close", self.pager.close.as_slice()),
                ("close_transcript", self.pager.close_transcript.as_slice()),
            ],
            TRANSCRIPT_BACKTRACK_RESERVED_BINDINGS,
            [],
        )?;

        validate_unique(
            "list",
            [
                ("move_up", self.list.move_up.as_slice()),
                ("move_down", self.list.move_down.as_slice()),
                ("move_left", self.list.move_left.as_slice()),
                ("move_right", self.list.move_right.as_slice()),
                ("page_up", self.list.page_up.as_slice()),
                ("page_down", self.list.page_down.as_slice()),
                ("jump_top", self.list.jump_top.as_slice()),
                ("jump_bottom", self.list.jump_bottom.as_slice()),
                ("accept", self.list.accept.as_slice()),
                ("cancel", self.list.cancel.as_slice()),
            ],
        )?;

        validate_unique(
            "approval",
            [
                ("open_fullscreen", self.approval.open_fullscreen.as_slice()),
                ("open_thread", self.approval.open_thread.as_slice()),
                ("approve", self.approval.approve.as_slice()),
                (
                    "approve_for_session",
                    self.approval.approve_for_session.as_slice(),
                ),
                (
                    "approve_for_prefix",
                    self.approval.approve_for_prefix.as_slice(),
                ),
                ("deny", self.approval.deny.as_slice()),
                ("decline", self.approval.decline.as_slice()),
                ("cancel", self.approval.cancel.as_slice()),
            ],
        )?;

        let mut seen: HashMap<(KeyCode, KeyModifiers), &'static str> = HashMap::new();
        for (action, bindings) in [
            ("list.move_up", self.list.move_up.as_slice()),
            ("list.move_down", self.list.move_down.as_slice()),
            ("list.move_left", self.list.move_left.as_slice()),
            ("list.move_right", self.list.move_right.as_slice()),
            ("list.page_up", self.list.page_up.as_slice()),
            ("list.page_down", self.list.page_down.as_slice()),
            ("list.jump_top", self.list.jump_top.as_slice()),
            ("list.jump_bottom", self.list.jump_bottom.as_slice()),
            ("list.accept", self.list.accept.as_slice()),
            ("list.cancel", self.list.cancel.as_slice()),
            (
                "approval.open_fullscreen",
                self.approval.open_fullscreen.as_slice(),
            ),
            ("approval.open_thread", self.approval.open_thread.as_slice()),
            ("approval.approve", self.approval.approve.as_slice()),
            (
                "approval.approve_for_session",
                self.approval.approve_for_session.as_slice(),
            ),
            (
                "approval.approve_for_prefix",
                self.approval.approve_for_prefix.as_slice(),
            ),
            ("approval.deny", self.approval.deny.as_slice()),
            ("approval.decline", self.approval.decline.as_slice()),
            ("approval.cancel", self.approval.cancel.as_slice()),
        ] {
            for binding in bindings {
                let key = binding.parts();
                if let Some(previous) = seen.insert(key, action) {
                    // 审批覆盖层有意保留 Esc 作为稳定的
                    // 取消路径，即使拒绝选项在安全上下文中
                    // 也可能显示它。
                    if previous == "list.cancel"
                        && action == "approval.decline"
                        && key == (KeyCode::Esc, KeyModifiers::NONE)
                    {
                        continue;
                    }
                    return Err(format!(
                        "Ambiguous approval overlay keymap bindings: `{previous}` and `{action}` use the same key. \
Set unique keys in `~/.reflect/config.toml` and retry. \
See the Reflect keymap documentation for supported actions and examples."
                    ));
                }
            }
        }

        Ok(())
    }
}

// ── 绑定解析/验证子系统（外移子模块） ──
mod binding_resolution;
use binding_resolution::*;

#[cfg(test)]
mod tests;
