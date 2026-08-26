//! 轻量键位映射层:全局快捷键的单一事实来源。
//!
//! 不接入 vendored 的 Reflect `RuntimeKeymap`(它耦合 `TuiKeymap` config 解析 +
//! 冲突校验 + ChatWidget UI,且完全未接线)。这里做一个自包含的薄层:
//! `resolve_global_key(key) -> Option<KeyAction>` 把全局快捷键(Ctrl-C / Ctrl-T /
//! /命令前的输入处理)集中到一个可测的纯函数,`handle_event` 顶层先 resolve,
//! 命中则分发,未命中 fallthrough 到 composer。
//!
//! 后续若要支持 config 驱动的重映射,只需把 `default_bindings()` 换成从配置加载,
//! 不动调用方。

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

/// 全局快捷键动作(主屏,overlay 关闭时)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// 退出 TUI。
    Exit,
    /// 打开 transcript overlay(Ctrl+T)。
    OpenTranscript,
    /// v1.x:循环切换 `PermissionMode`(Shift+Tab / 空输入下 Tab)。
    CycleMode,
    /// v1.x:复制最后一条 agent 回复到剪贴板(Ctrl+O)。
    CopyLastReply,
    /// v1.x:打开外部编辑器编辑当前 input draft(Ctrl+G)。
    OpenExternalEditor,
    /// v1.x:强制重绘整屏(Ctrl+L),绕开 ratatui `Terminal: !Send` 限制。
    ForceRedraw,
    /// v1.x:打开 diff 全屏 overlay(Ctrl+Shift+D),等价 `/diff`。
    OpenDiff,
    /// v1.x:切换当前选中的 tool call 的 diff 展开(Ctrl+Shift+D 长按,暂未接)。
    /// 保留为 `OpenDiff` 的别名,统一在 overlay 内部处理展开。
    ToggleDiffView,
}

/// 解析全局快捷键。命中返回 `Some(action)`,否则 `None`(交回 composer)。
///
/// 注意:这是「无 overlay 打开时」的全局键。overlay 打开时,键位由 overlay
/// 独占(`handle_event` 里先检查 overlay_open,本函数此时不被调)。
///
/// **Shift+Tab(BackTab)在主循环里无条件命中**;**Tab 仅在输入为空时命中**
/// —— Tab 的「空输入」判定需要 composer 文本,不能在纯键映射里完成,
/// 故 `resolve_global_key` 只认 BackTab,Tab 由主循环在 `composer.is_empty()`
/// 成立时单独发 `CycleMode`。
pub fn resolve_global_key(key: Event) -> Option<KeyAction> {
    let key = match key {
        Event::Key(k) => k,
        _ => return None,
    };
    // Ctrl-C / Ctrl-Shift-C:退出。
    if is_exit_key(key) {
        return Some(KeyAction::Exit);
    }
    // Ctrl-D(不含 Shift):也退出(类 shell EOF)。Ctrl-Shift-D 仍归 OpenDiff,
    // 故此处要求不含 SHIFT,避免与 diff overlay 快捷键冲突。
    if key.code == KeyCode::Char('d')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::SHIFT)
    {
        return Some(KeyAction::Exit);
    }
    // Ctrl-T:transcript overlay。
    if key.code == KeyCode::Char('t') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(KeyAction::OpenTranscript);
    }
    // Ctrl-O:复制最后一条 agent 回复。
    if key.code == KeyCode::Char('o')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
    {
        return Some(KeyAction::CopyLastReply);
    }
    // Ctrl-G:打开外部编辑器编辑当前 input draft。
    if key.code == KeyCode::Char('g')
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
    {
        return Some(KeyAction::OpenExternalEditor);
    }
    // Ctrl-L:强制 redraw。
    if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(KeyAction::ForceRedraw);
    }
    // Ctrl+Shift-D:打开 diff 全屏 overlay。
    if (key.code == KeyCode::Char('d') || key.code == KeyCode::Char('D'))
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.modifiers.contains(KeyModifiers::SHIFT)
    {
        return Some(KeyAction::OpenDiff);
    }
    // Shift+Tab(BackTab):循环 permission mode。
    if key.code == KeyCode::BackTab {
        return Some(KeyAction::CycleMode);
    }
    None
}

/// 退出键判定(Ctrl-C)。与 `terminal::is_exit_key` 对齐,独立定义避免循环依赖。
fn is_exit_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl(ch: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL))
    }

    fn ctrl_shift(ch: char) -> Event {
        Event::Key(KeyEvent::new(
            KeyCode::Char(ch),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ))
    }

    fn plain(ch: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
    }

    #[test]
    fn ctrl_c_exits() {
        assert_eq!(resolve_global_key(ctrl('c')), Some(KeyAction::Exit));
    }

    #[test]
    fn ctrl_t_opens_transcript() {
        assert_eq!(
            resolve_global_key(ctrl('t')),
            Some(KeyAction::OpenTranscript)
        );
    }

    #[test]
    fn ctrl_o_copies_last_reply() {
        assert_eq!(
            resolve_global_key(ctrl('o')),
            Some(KeyAction::CopyLastReply)
        );
    }

    #[test]
    fn ctrl_g_opens_external_editor() {
        assert_eq!(
            resolve_global_key(ctrl('g')),
            Some(KeyAction::OpenExternalEditor)
        );
    }

    #[test]
    fn ctrl_l_force_redraw() {
        assert_eq!(resolve_global_key(ctrl('l')), Some(KeyAction::ForceRedraw));
    }

    #[test]
    fn ctrl_shift_d_opens_diff() {
        assert_eq!(
            resolve_global_key(ctrl_shift('d')),
            Some(KeyAction::OpenDiff)
        );
        // D 也匹配(优先小写,但为了安全也接受大写)。
        assert_eq!(
            resolve_global_key(ctrl_shift('D')),
            Some(KeyAction::OpenDiff)
        );
    }

    #[test]
    fn ctrl_d_alone_exits_and_not_diff() {
        // Ctrl+D(无 Shift)→ 退出(类 shell EOF),且不触发 OpenDiff。
        assert_eq!(resolve_global_key(ctrl('d')), Some(KeyAction::Exit));
    }

    #[test]
    fn backtab_cycles_mode() {
        // Shift+Tab = BackTab,无条件循环 permission mode。
        assert_eq!(
            resolve_global_key(Event::Key(KeyEvent::new(
                KeyCode::BackTab,
                KeyModifiers::SHIFT
            ))),
            Some(KeyAction::CycleMode)
        );
    }

    #[test]
    fn plain_tab_not_global() {
        // 裸 Tab 不在全局映射里(由主循环在 composer 空时单独处理),
        // 保证有内容时 Tab 继续走 composer 的自动补全/缩进。
        assert_eq!(
            resolve_global_key(Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))),
            None
        );
    }

    #[test]
    fn plain_keys_not_global() {
        assert_eq!(resolve_global_key(plain('a')), None);
        assert_eq!(resolve_global_key(plain('t')), None);
        assert_eq!(resolve_global_key(plain('c')), None);
    }

    #[test]
    fn enter_not_global() {
        assert_eq!(
            resolve_global_key(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE
            ))),
            None
        );
    }

    #[test]
    fn ctrl_other_not_global() {
        // Ctrl+Shift+D 单独处理；单独的 Ctrl+D 不属于全局快捷键。
        assert_eq!(resolve_global_key(ctrl('x')), None);
    }

    #[test]
    fn non_key_event_returns_none() {
        // 窗口调整事件不应被解释为按键动作。
        assert_eq!(resolve_global_key(Event::Resize(80, 24)), None);
    }
}
