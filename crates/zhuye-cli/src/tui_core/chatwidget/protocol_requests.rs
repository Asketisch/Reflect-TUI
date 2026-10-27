//! `ChatWidget` 的 App 服务端请求与通知分发。
//!
//! 本模块将协议请求转换为聚焦的聊天 widget 流程，
//! 用于渲染审批、权限、工具输入与守护审查。

use super::*;

impl ChatWidget {
    pub(crate) fn handle_server_request(
        &mut self,
        request: ServerRequest,
        replay_kind: Option<ReplayKind>,
    ) {
        let id = request.id().to_string();
        match request {
            ServerRequest::CommandExecutionRequestApproval { params, .. } => {
                let fallback_cwd = self.config.cwd.clone();
                self.on_exec_approval_request(
                    id,
                    exec_approval_request_from_params(params, &fallback_cwd),
                );
            }
            ServerRequest::FileChangeRequestApproval { params, .. } => {
                self.on_apply_patch_approval_request(
                    id,
                    patch_approval_request_from_params(params),
                );
            }
            ServerRequest::McpServerElicitationRequest { request_id, params } => {
                self.on_elicitation_request(request_id, params);
            }
            ServerRequest::PermissionsRequestApproval { params, .. } => {
                // TODO(anp): 当核心权限路径跨越 app 服务端边界后仍保持为 PathUri 时，
                // 移除这条本地路径定位错误路径。
                match request_permissions_from_params(params) {
                    Ok(event) => self.on_request_permissions(event),
                    Err(err) => {
                        self.add_error_message(format!(
                            "failed to localize requested filesystem paths: {err}"
                        ));
                    }
                }
            }
            ServerRequest::ToolRequestUserInput { params, .. } => {
                self.on_request_user_input(params);
            }
            ServerRequest::DynamicToolCall { .. }
            | ServerRequest::AttestationGenerate { .. }
            | ServerRequest::CurrentTimeRead { .. }
            | ServerRequest::ChatgptAuthTokensRefresh { .. }
            | ServerRequest::ApplyPatchApproval { .. }
            | ServerRequest::ExecCommandApproval { .. } => {
                if replay_kind.is_none() {
                    self.add_error_message(TUI_STUB_MESSAGE.to_string());
                }
            }
        }
    }

    pub(crate) fn handle_skills_list_response(&mut self, response: SkillsListResponse) {
        self.on_list_skills(response);
    }

    pub(super) fn on_patch_apply_output_delta(&mut self, _item_id: String, _delta: String) {}

    pub(super) fn on_guardian_review_notification(
        &mut self,
        id: String,
        turn_id: String,
        started_at_ms: i64,
        review: crate::app_server_protocol::GuardianApprovalReview,
        completion: Option<(i64, crate::app_server_protocol::AutoReviewDecisionSource)>,
        action: GuardianApprovalReviewAction,
    ) {
        // TODO(anp): 当核心权限路径跨越 app 服务端边界后仍保持为 PathUri 时，
        // 移除这条本地路径定位错误路径。
        let action = match action.try_into() {
            Ok(action) => action,
            Err(err) => {
                self.add_error_message(format!(
                    "failed to localize guardian filesystem paths: {err}"
                ));
                return;
            }
        };
        let (completed_at_ms, decision_source) = match completion {
            Some((completed_at_ms, decision_source)) => {
                (Some(completed_at_ms), Some(decision_source))
            }
            None => (None, None),
        };

        self.on_guardian_assessment(GuardianAssessmentEvent {
            id,
            target_item_id: None,
            turn_id,
            started_at_ms,
            completed_at_ms,
            status: match review.status {
                crate::app_server_protocol::GuardianApprovalReviewStatus::InProgress => {
                    GuardianAssessmentStatus::InProgress
                }
                crate::app_server_protocol::GuardianApprovalReviewStatus::Approved => {
                    GuardianAssessmentStatus::Approved
                }
                crate::app_server_protocol::GuardianApprovalReviewStatus::Denied => {
                    GuardianAssessmentStatus::Denied
                }
                crate::app_server_protocol::GuardianApprovalReviewStatus::TimedOut => {
                    GuardianAssessmentStatus::TimedOut
                }
                crate::app_server_protocol::GuardianApprovalReviewStatus::Aborted => {
                    GuardianAssessmentStatus::Aborted
                }
            },
            risk_level: review.risk_level.map(|risk_level| match risk_level {
                crate::app_server_protocol::GuardianRiskLevel::Low => {
                    crate::protocol_compat::approvals::GuardianRiskLevel::Low
                }
                crate::app_server_protocol::GuardianRiskLevel::Medium => {
                    crate::protocol_compat::approvals::GuardianRiskLevel::Medium
                }
                crate::app_server_protocol::GuardianRiskLevel::High => {
                    crate::protocol_compat::approvals::GuardianRiskLevel::High
                }
                crate::app_server_protocol::GuardianRiskLevel::Critical => {
                    crate::protocol_compat::approvals::GuardianRiskLevel::Critical
                }
            }),
            user_authorization: review.user_authorization.map(|user_authorization| {
                match user_authorization {
                    crate::app_server_protocol::GuardianUserAuthorization::Unknown => {
                        crate::protocol_compat::approvals::GuardianUserAuthorization::Unknown
                    }
                    crate::app_server_protocol::GuardianUserAuthorization::Low => {
                        crate::protocol_compat::approvals::GuardianUserAuthorization::Low
                    }
                    crate::app_server_protocol::GuardianUserAuthorization::Medium => {
                        crate::protocol_compat::approvals::GuardianUserAuthorization::Medium
                    }
                    crate::app_server_protocol::GuardianUserAuthorization::High => {
                        crate::protocol_compat::approvals::GuardianUserAuthorization::High
                    }
                }
            }),
            rationale: review.rationale,
            decision_source: decision_source.map(|source| match source {
                crate::app_server_protocol::AutoReviewDecisionSource::Agent => {
                    GuardianAssessmentDecisionSource::Agent
                }
            }),
            action,
        });
    }

    pub(super) fn on_shutdown_complete(&mut self) {
        self.request_immediate_exit();
    }

    pub(super) fn on_turn_diff(&mut self, unified_diff: String) {
        debug!("TurnDiffEvent: {unified_diff}");
        self.refresh_status_line();
    }

    pub(super) fn on_deprecation_notice(&mut self, summary: String, details: Option<String>) {
        self.add_to_history(history_cell::new_deprecation_notice(summary, details));
        self.request_redraw();
    }
}
