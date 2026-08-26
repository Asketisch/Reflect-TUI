//! ChatWidget 审批/权限/token 用量/指标提取辅助函数簇。从 chatwidget/mod.rs 抽出。

use super::*;

pub(super) fn exec_approval_request_from_params(
    params: CommandExecutionRequestApprovalParams,
    fallback_cwd: &AbsolutePathBuf,
) -> ExecApprovalRequestEvent {
    // TODO(anp): 一旦 `tui::approval_events::ExecApprovalRequestEvent` 与审批渲染
    // 支持外部路径，就保持其为 PathUri。
    let cwd = params
        .cwd
        .and_then(|cwd| Some(cwd.to_inferred_abs_path()))
        .unwrap_or_else(|| fallback_cwd.clone());
    ExecApprovalRequestEvent {
        call_id: params.item_id,
        command: params
            .command
            .as_deref()
            .map(split_command_string)
            .unwrap_or_default(),
        cwd,
        reason: params.reason,
        network_approval_context: params.network_approval_context,
        additional_permissions: params.additional_permissions,
        turn_id: params.turn_id,
        approval_id: params.approval_id,
        environment_id: params.environment_id,
        proposed_execpolicy_amendment: params.proposed_execpolicy_amendment,
        proposed_network_policy_amendments: params.proposed_network_policy_amendments,
        available_decisions: params.available_decisions,
    }
}

pub(super) fn patch_approval_request_from_params(
    params: FileChangeRequestApprovalParams,
) -> ApplyPatchApprovalRequestEvent {
    ApplyPatchApprovalRequestEvent {
        call_id: params.item_id,
        turn_id: params.turn_id,
        changes: HashMap::new(),
        reason: params.reason,
        grant_root: params.grant_root,
    }
}

pub(super) fn request_permissions_from_params(
    params: crate::app_server_protocol::PermissionsRequestApprovalParams,
) -> std::io::Result<RequestPermissionsEvent> {
    Ok(RequestPermissionsEvent {
        turn_id: params.turn_id,
        call_id: params.item_id,
        environment_id: params.environment_id,
        started_at_ms: params.started_at_ms,
        reason: params.reason,
        permissions: params.permissions.try_into().map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::Other, "permission conversion failed")
        })?,
        cwd: Some(params.cwd.display().to_string()),
    })
}

pub(super) fn token_usage_info_from_app_server(token_usage: ThreadTokenUsage) -> TokenUsageInfo {
    TokenUsageInfo {
        total_token_usage: TokenUsage {
            total_tokens: token_usage.total.total_tokens,
            input_tokens: token_usage.total.input_tokens,
            cached_input_tokens: token_usage.total.cached_input_tokens,
            output_tokens: token_usage.total.output_tokens,
            reasoning_output_tokens: token_usage.total.reasoning_output_tokens,
        },
        last_token_usage: TokenUsage {
            total_tokens: token_usage.last.total_tokens,
            input_tokens: token_usage.last.input_tokens,
            cached_input_tokens: token_usage.last.cached_input_tokens,
            output_tokens: token_usage.last.output_tokens,
            reasoning_output_tokens: token_usage.last.reasoning_output_tokens,
        },
        model_context_window: token_usage.model_context_window,
    }
}

pub(super) fn has_websocket_timing_metrics(summary: &RuntimeMetricsSummary) -> bool {
    summary.responses_api_overhead_ms > 0.0
        || summary.responses_api_inference_time_ms > 0.0
        || summary.responses_api_engine_iapi_ttft_ms > 0.0
        || summary.responses_api_engine_service_ttft_ms > 0.0
        || summary.responses_api_engine_iapi_tbt_ms > 0.0
        || summary.responses_api_engine_service_tbt_ms > 0.0
}

// 从 `s` 中提取 **...** 形式的第一个粗体（Markdown）元素。
// 找到时返回内部文本；否则返回 `None`。
pub(super) fn extract_first_bold(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            let start = i + 2;
            let mut j = start;
            while j + 1 < bytes.len() {
                if bytes[j] == b'*' && bytes[j + 1] == b'*' {
                    // 找到闭合的 **
                    let inner = &s[start..j];
                    let trimmed = inner.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    } else {
                        return None;
                    }
                }
                j += 1;
            }
            // 没有闭合；停止搜索（等待更多增量）
            return None;
        }
        i += 1;
    }
    None
}

/// 选择用于编辑最近排队消息的按键绑定。
///
/// Apple Terminal、Warp 和 VSCode 集成终端会拦截或静默
/// 吞掉 Alt+Up，而 tmux 无法可靠地传递该组合键。在这些环境中我们回退
/// 到 Shift+Left，同时在其他地方保留更易发现的
/// Alt+Up。
///
/// 该匹配是穷尽的，因此新增 `TerminalName` 变体将强制
/// 明确决定该终端应使用哪个绑定。
pub(super) fn queued_message_edit_binding_for_terminal(terminal_info: TerminalInfo) -> KeyBinding {
    if matches!(
        terminal_info.multiplexer.as_ref(),
        Some(Multiplexer::Tmux { .. })
    ) {
        return key_hint::shift(KeyCode::Left);
    }

    match terminal_info.name {
        TerminalName::AppleTerminal | TerminalName::WarpTerminal | TerminalName::VsCode => {
            key_hint::shift(KeyCode::Left)
        }
        TerminalName::Ghostty
        | TerminalName::Iterm2
        | TerminalName::WezTerm
        | TerminalName::Kitty
        | TerminalName::Alacritty
        | TerminalName::Konsole
        | TerminalName::GnomeTerminal
        | TerminalName::Vte
        | TerminalName::WindowsTerminal
        | TerminalName::Dumb
        | TerminalName::Unknown => key_hint::alt(KeyCode::Up),
        _ => key_hint::alt(KeyCode::Up),
    }
}

pub(super) fn queued_message_edit_hint_binding(
    bindings: &[KeyBinding],
    terminal_info: TerminalInfo,
) -> Option<KeyBinding> {
    let terminal_binding = queued_message_edit_binding_for_terminal(terminal_info);
    bindings
        .contains(&terminal_binding)
        .then_some(terminal_binding)
        .or_else(|| bindings.first().copied())
}

pub(super) fn normalize_thread_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// 返回 `text` 是否包含独立单词 `plan`。
///
/// 这有意镜像 App 的建议启发式，而不是尝试从 `planning` 之类的
/// 子串推断更广的规划意图。斜杠和 shell 草稿在此仍会匹配，使
/// 调用方可以把词法匹配与呈现策略分开。
pub(super) fn contains_plan_keyword(text: &str) -> bool {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .any(|word| word.eq_ignore_ascii_case("plan"))
}
