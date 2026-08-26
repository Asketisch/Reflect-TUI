//! 审批浮层的 options/header 构建辅助函数。从 approval_overlay.rs 抽出。

use super::*;

pub(super) fn approval_footer_hint(
    request: &ApprovalRequest,
    approval_keymap: &ApprovalKeymap,
    list_keymap: &ListKeymap,
) -> Line<'static> {
    let mut spans = accept_cancel_hint_line(
        primary_binding(&list_keymap.accept),
        "to confirm",
        primary_binding(&list_keymap.cancel),
        "to cancel",
    )
    .spans;
    if request.thread_label().is_some()
        && let Some(open_thread) = primary_binding(&approval_keymap.open_thread)
    {
        if !spans.is_empty() {
            spans.push(" or ".into());
        } else {
            spans.push("Press ".into());
        }
        spans.extend([open_thread.into(), " to open thread".into()]);
    }
    Line::from(spans)
}

pub(super) fn network_approval_target(
    network_approval_context: &NetworkApprovalContext,
    command: &[String],
) -> String {
    if let Some(target) = network_approval_command_target(command) {
        return target.to_string();
    }

    let scheme = match network_approval_context.protocol {
        NetworkApprovalProtocol::Http => "http",
        NetworkApprovalProtocol::Https => "https",
        NetworkApprovalProtocol::Socks5Tcp => "socks5-tcp",
        NetworkApprovalProtocol::Socks5Udp => "socks5-udp",
    };
    format!("{scheme}://{}", network_approval_context.host)
}

pub(super) fn network_approval_command_target(command: &[String]) -> Option<&str> {
    match command {
        [program, target] if program == "network-access" && !target.is_empty() => {
            Some(target.as_str())
        }
        [command] => command
            .strip_prefix("network-access ")
            .filter(|target| !target.is_empty()),
        _ => None,
    }
}

pub(super) fn build_header(request: &ApprovalRequest) -> Box<dyn Renderable> {
    match request {
        ApprovalRequest::Exec(request) => {
            let mut header: Vec<Line<'static>> = Vec::new();
            if let Some(thread_label) = &request.thread_label {
                header.push(Line::from(vec![
                    "Thread: ".into(),
                    thread_label.clone().bold(),
                ]));
                header.push(Line::from(""));
            }
            if let Some(environment_id) = &request.environment_id {
                header.push(Line::from(vec![
                    "Environment: ".into(),
                    environment_id.clone().bold(),
                ]));
                header.push(Line::from(""));
            }
            if let Some(reason) = &request.reason {
                header.push(Line::from(vec!["Reason: ".into(), reason.clone().italic()]));
                header.push(Line::from(""));
            }
            if let Some(additional_permissions) = &request.additional_permissions
                && let Some(rule_line) = format_additional_permissions_rule(additional_permissions)
            {
                header.push(Line::from(vec![
                    "Permission rule: ".into(),
                    rule_line.cyan(),
                ]));
                header.push(Line::from(""));
            }
            let full_cmd = strip_bash_lc_and_escape(&request.command);
            let mut full_cmd_lines = highlight_bash_to_lines(&full_cmd);
            if let Some(first) = full_cmd_lines.first_mut() {
                first.spans.insert(0, Span::from("$ "));
            }
            if request.network_approval_context.is_none() {
                header.extend(full_cmd_lines);
            }
            Box::new(Paragraph::new(header).wrap(Wrap { trim: false }))
        }
        ApprovalRequest::Permissions(request) => {
            let mut header: Vec<Line<'static>> = Vec::new();
            if let Some(thread_label) = &request.thread_label {
                header.push(Line::from(vec![
                    "Thread: ".into(),
                    thread_label.clone().bold(),
                ]));
                header.push(Line::from(""));
            }
            if let Some(environment_id) = &request.environment_id {
                header.push(Line::from(vec![
                    "Environment: ".into(),
                    environment_id.clone().bold(),
                ]));
                header.push(Line::from(""));
            }
            if let Some(reason) = &request.reason {
                header.push(Line::from(vec!["Reason: ".into(), reason.clone().italic()]));
                header.push(Line::from(""));
            }
            if let Some(rule_line) = format_requested_permissions_rule(&request.permissions) {
                header.push(Line::from(vec![
                    "Permission rule: ".into(),
                    rule_line.cyan(),
                ]));
            }
            Box::new(Paragraph::new(header).wrap(Wrap { trim: false }))
        }
        ApprovalRequest::ApplyPatch(request) => {
            let mut header: Vec<Box<dyn Renderable>> = Vec::new();
            if let Some(thread_label) = &request.thread_label {
                header.push(Box::new(Line::from(vec![
                    "Thread: ".into(),
                    thread_label.clone().bold(),
                ])));
            }
            if let Some(reason) = &request.reason
                && !reason.is_empty()
            {
                if !header.is_empty() {
                    header.push(Box::new(Line::from("")));
                }
                header.push(Box::new(
                    Paragraph::new(Line::from_iter([
                        "Reason: ".into(),
                        reason.clone().italic(),
                    ]))
                    .wrap(Wrap { trim: false }),
                ));
            }
            Box::new(ColumnRenderable::with(header))
        }
        ApprovalRequest::McpElicitation(request) => {
            let mut lines = Vec::new();
            if let Some(thread_label) = &request.thread_label {
                lines.push(Line::from(vec![
                    "Thread: ".into(),
                    thread_label.clone().bold(),
                ]));
                lines.push(Line::from(""));
            }
            lines.extend([
                Line::from(vec!["Server: ".into(), request.server_name.clone().bold()]),
                Line::from(""),
                Line::from(request.message.clone()),
            ]);
            let header = Paragraph::new(lines).wrap(Wrap { trim: false });
            Box::new(header)
        }
    }
}

#[derive(Clone)]
pub(super) enum ApprovalDecision {
    Command(CommandExecutionApprovalDecision),
    FileChange(FileChangeApprovalDecision),
    Permissions(PermissionsDecision),
    McpElicitation(McpServerElicitationAction),
}

#[derive(Clone, Copy)]
pub(super) enum PermissionsDecision {
    GrantForTurn,
    GrantForTurnWithStrictAutoReview,
    GrantForSession,
    Deny,
}

#[derive(Clone)]
pub(super) struct ApprovalOption {
    pub(super) label: String,
    pub(super) decision: ApprovalDecision,
    pub(super) shortcuts: Vec<KeyBinding>,
}

pub(super) fn command_decision_to_review_decision(
    decision: &CommandExecutionApprovalDecision,
) -> ReviewDecision {
    match decision {
        CommandExecutionApprovalDecision::Accept => ReviewDecision::Approved,
        CommandExecutionApprovalDecision::AcceptForSession => ReviewDecision::ApprovedForSession,
        CommandExecutionApprovalDecision::AcceptWithExecpolicyAmendment {
            execpolicy_amendment,
        } => ReviewDecision::ApprovedExecpolicyAmendment {
            proposed_execpolicy_amendment: execpolicy_amendment.clone().into_core(),
        },
        CommandExecutionApprovalDecision::ApplyNetworkPolicyAmendment {
            network_policy_amendment,
        } => ReviewDecision::NetworkPolicyAmendment {
            network_policy_amendment: network_policy_amendment.clone().into_core(),
        },
        CommandExecutionApprovalDecision::Decline => ReviewDecision::Denied,
        CommandExecutionApprovalDecision::Cancel => ReviewDecision::Abort,
    }
}

pub(super) fn exec_options(
    available_decisions: &[CommandExecutionApprovalDecision],
    network_approval_context: Option<&NetworkApprovalContext>,
    additional_permissions: Option<&AdditionalPermissionProfile>,
    keymap: &ApprovalKeymap,
) -> Vec<ApprovalOption> {
    available_decisions
        .iter()
        .filter_map(|decision| match decision {
            CommandExecutionApprovalDecision::Accept => Some(ApprovalOption {
                label: if network_approval_context.is_some() {
                    "Yes, just this once".to_string()
                } else {
                    "Yes, proceed".to_string()
                },
                decision: ApprovalDecision::Command(CommandExecutionApprovalDecision::Accept),
                shortcuts: keymap.approve.clone(),
            }),
            CommandExecutionApprovalDecision::AcceptWithExecpolicyAmendment {
                execpolicy_amendment,
            } => {
                let rendered_prefix = strip_bash_lc_and_escape(&execpolicy_amendment.command);
                if rendered_prefix.contains('\n') || rendered_prefix.contains('\r') {
                    return None;
                }

                Some(ApprovalOption {
                    label: format!(
                        "Yes, and don't ask again for commands that start with `{rendered_prefix}`"
                    ),
                    decision: ApprovalDecision::Command(
                        CommandExecutionApprovalDecision::AcceptWithExecpolicyAmendment {
                            execpolicy_amendment: execpolicy_amendment.clone(),
                        },
                    ),
                    shortcuts: keymap.approve_for_prefix.clone(),
                })
            }
            CommandExecutionApprovalDecision::AcceptForSession => Some(ApprovalOption {
                label: if network_approval_context.is_some() {
                    "Yes, and allow this host for this conversation".to_string()
                } else if additional_permissions.is_some() {
                    "Yes, and allow these permissions for this session".to_string()
                } else {
                    "Yes, and don't ask again for this command in this session".to_string()
                },
                decision: ApprovalDecision::Command(
                    CommandExecutionApprovalDecision::AcceptForSession,
                ),
                shortcuts: keymap.approve_for_session.clone(),
            }),
            CommandExecutionApprovalDecision::ApplyNetworkPolicyAmendment {
                network_policy_amendment,
            } => {
                let (label, shortcuts) = match network_policy_amendment.action {
                    NetworkPolicyRuleAction::Allow => (
                        "Yes, and allow this host in the future".to_string(),
                        keymap.approve_for_prefix.clone(),
                    ),
                    NetworkPolicyRuleAction::Deny => (
                        "No, and block this host in the future".to_string(),
                        keymap.deny.clone(),
                    ),
                };
                Some(ApprovalOption {
                    label,
                    decision: ApprovalDecision::Command(
                        CommandExecutionApprovalDecision::ApplyNetworkPolicyAmendment {
                            network_policy_amendment: network_policy_amendment.clone(),
                        },
                    ),
                    shortcuts,
                })
            }
            CommandExecutionApprovalDecision::Decline => Some(ApprovalOption {
                label: "No, continue without running it".to_string(),
                decision: ApprovalDecision::Command(CommandExecutionApprovalDecision::Decline),
                shortcuts: keymap.deny.clone(),
            }),
            CommandExecutionApprovalDecision::Cancel => Some(ApprovalOption {
                label: "No, and tell Reflect what to do differently".to_string(),
                decision: ApprovalDecision::Command(CommandExecutionApprovalDecision::Cancel),
                shortcuts: keymap.decline.clone(),
            }),
        })
        .collect()
}

pub(super) fn format_additional_permissions_rule(
    additional_permissions: &AdditionalPermissionProfile,
) -> Option<String> {
    let mut parts = Vec::new();
    if additional_permissions
        .network
        .as_ref()
        .and_then(|network| network.enabled)
        .unwrap_or(false)
    {
        parts.push("network".to_string());
    }
    if let Some(file_system) = additional_permissions.file_system.as_ref() {
        let reads = format_file_system_entry_paths(
            file_system
                .entries
                .iter()
                .flatten()
                .filter(|entry| entry.access == FileSystemAccessMode::Read),
        );
        if !reads.is_empty() {
            parts.push(format!("read {reads}"));
        }
        let writes = format_file_system_entry_paths(
            file_system
                .entries
                .iter()
                .flatten()
                .filter(|entry| entry.access == FileSystemAccessMode::Write),
        );
        if !writes.is_empty() {
            parts.push(format!("write {writes}"));
        }
        let denied_reads = format_file_system_entry_paths(
            file_system
                .entries
                .iter()
                .flatten()
                .filter(|entry| entry.access == FileSystemAccessMode::Deny),
        );
        if !denied_reads.is_empty() {
            parts.push(format!("deny read {denied_reads}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

pub(super) fn format_requested_permissions_rule(
    permissions: &RequestPermissionProfile,
) -> Option<String> {
    let permissions =
        crate::tui_core::app_server_approval_conversions::granted_permission_profile_from_request(
            permissions.clone(),
        );
    format_additional_permissions_rule(&AdditionalPermissionProfile {
        network: permissions.network,
        file_system: permissions.file_system,
    })
}

pub(super) fn format_file_system_entry_paths<'a>(
    entries: impl Iterator<Item = &'a FileSystemSandboxEntry>,
) -> String {
    entries
        .map(|entry| match &entry.path {
            FileSystemPath::Path { path } => format!("`{path}`"),
            FileSystemPath::GlobPattern { pattern } => format!("glob `{pattern}`"),
            FileSystemPath::Special { value } => format!("`{}`", special_path_label(value)),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn special_path_label(value: &FileSystemSpecialPath) -> String {
    match value {
        FileSystemSpecialPath::Root => ":root".to_string(),
        FileSystemSpecialPath::Minimal => ":minimal".to_string(),
        FileSystemSpecialPath::ProjectRoots { subpath } => path_label(":workspace_roots", subpath),
        FileSystemSpecialPath::Tmpdir => ":tmpdir".to_string(),
        FileSystemSpecialPath::SlashTmp => "/tmp".to_string(),
        FileSystemSpecialPath::Unknown { path, subpath } => path_label(path, subpath),
    }
}

pub(super) fn path_label(base: &str, subpath: &Option<String>) -> String {
    match subpath {
        Some(subpath) => format!("{base}/{subpath}"),
        None => base.to_string(),
    }
}

pub(super) fn patch_options(keymap: &ApprovalKeymap) -> Vec<ApprovalOption> {
    vec![
        ApprovalOption {
            label: "Yes, proceed".to_string(),
            decision: ApprovalDecision::FileChange(FileChangeApprovalDecision::Accept),
            shortcuts: keymap.approve.clone(),
        },
        ApprovalOption {
            label: "Yes, and don't ask again for these files".to_string(),
            decision: ApprovalDecision::FileChange(FileChangeApprovalDecision::AcceptForSession),
            shortcuts: keymap.approve_for_session.clone(),
        },
        ApprovalOption {
            label: "No, and tell Reflect what to do differently".to_string(),
            decision: ApprovalDecision::FileChange(FileChangeApprovalDecision::Cancel),
            shortcuts: keymap.decline.clone(),
        },
    ]
}

pub(super) fn permissions_options(keymap: &ApprovalKeymap) -> Vec<ApprovalOption> {
    let deny_shortcuts = keymap
        .deny
        .iter()
        .copied()
        .filter(|shortcut| shortcut.parts() != (KeyCode::Esc, KeyModifiers::NONE))
        .collect();

    vec![
        ApprovalOption {
            label: "Yes, grant these permissions for this turn".to_string(),
            decision: ApprovalDecision::Permissions(PermissionsDecision::GrantForTurn),
            shortcuts: keymap.approve.clone(),
        },
        ApprovalOption {
            label: "Yes, grant for this turn with strict auto review".to_string(),
            decision: ApprovalDecision::Permissions(
                PermissionsDecision::GrantForTurnWithStrictAutoReview,
            ),
            shortcuts: vec![key_hint::plain(KeyCode::Char('r'))],
        },
        ApprovalOption {
            label: "Yes, grant these permissions for this session".to_string(),
            decision: ApprovalDecision::Permissions(PermissionsDecision::GrantForSession),
            shortcuts: keymap.approve_for_session.clone(),
        },
        ApprovalOption {
            label: "No, continue without permissions".to_string(),
            decision: ApprovalDecision::Permissions(PermissionsDecision::Deny),
            shortcuts: deny_shortcuts,
        },
    ]
}

/// 构建具有稳定取消语义的 MCP elicitation 选项。
///
/// 对于 elicitation 提示，即使自定义了 `decline`/`cancel` 绑定，`Esc` 始终被
/// 视为取消。我们将其作为硬性约定保留下来，确保关闭始终是一条安全的终止路径，
/// 绝不会静默变成“无请求信息继续”。为保持该不变式，elicitation 模式下
/// 会从 decline 选项中移除与 cancel 重叠的绑定。
pub(super) fn elicitation_options(keymap: &ApprovalKeymap) -> Vec<ApprovalOption> {
    let mut cancel_shortcuts = vec![key_hint::plain(KeyCode::Esc)];
    for shortcut in &keymap.cancel {
        if !cancel_shortcuts.contains(shortcut) {
            cancel_shortcuts.push(*shortcut);
        }
    }

    let decline_shortcuts: Vec<KeyBinding> = keymap
        .decline
        .iter()
        .copied()
        .filter(|shortcut| !cancel_shortcuts.contains(shortcut))
        .collect();

    vec![
        ApprovalOption {
            label: "Yes, provide the requested info".to_string(),
            decision: ApprovalDecision::McpElicitation(McpServerElicitationAction::Accept),
            shortcuts: keymap.approve.clone(),
        },
        ApprovalOption {
            label: "No, but continue without it".to_string(),
            decision: ApprovalDecision::McpElicitation(McpServerElicitationAction::Decline),
            shortcuts: decline_shortcuts,
        },
        ApprovalOption {
            label: "Cancel this request".to_string(),
            decision: ApprovalDecision::McpElicitation(McpServerElicitationAction::Cancel),
            shortcuts: cancel_shortcuts,
        },
    ]
}
