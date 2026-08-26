//! `ChatWidget` 的运行时设置状态以及模型/协作模式协调。

use super::*;
use crate::tui_core::app_event::AppEvent;
use crate::tui_core::chatwidget::rate_limits::RATE_LIMIT_SWITCH_PROMPT_VIEW_ID;

impl ChatWidget {
    /// 在 widget 的配置副本中设置审批策略。
    pub(crate) fn set_approval_policy(&mut self, policy: AskForApproval) {
        if let Err(err) = self
            .config
            .permissions
            .approval_policy
            .set(policy.to_core())
        {
            tracing::warn!(%err, "failed to set approval_policy on chat config");
        } else {
            self.refresh_status_surfaces();
        }
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn set_permission_profile_from_session_snapshot(
        &mut self,
        snapshot: PermissionProfileSnapshot,
    ) -> ConstraintResult<()> {
        self.config
            .permissions
            .set_permission_profile_from_session_snapshot(snapshot)?;
        self.refresh_status_surfaces();
        Ok(())
    }

    pub(crate) fn set_permission_profile_with_active_profile(
        &mut self,
        profile: PermissionProfile,
        active_permission_profile: Option<ActivePermissionProfile>,
    ) -> ConstraintResult<()> {
        self.config
            .permissions
            .set_permission_profile_from_session_snapshot(
                PermissionProfileSnapshot::from_session_snapshot(
                    profile,
                    active_permission_profile.map(Into::into),
                ),
            )?;
        self.refresh_status_surfaces();
        Ok(())
    }

    pub(crate) fn set_permission_network(
        &mut self,
        network: Option<crate::tui_core::legacy_core::config::NetworkProxySpec>,
    ) {
        self.config.permissions.network = network;
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn set_windows_sandbox_mode(&mut self, mode: Option<WindowsSandboxModeToml>) {
        self.config.permissions.windows_sandbox_mode =
            mode.map(|m| m.to_string()).unwrap_or_default();
        #[cfg(target_os = "windows")]
        self.bottom_pane
            .set_windows_degraded_sandbox_active(matches!(
                crate::tui_core::windows_sandbox::level_from_config(&self.config),
                WindowsSandboxLevel::RestrictedToken
            ));
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn set_feature_enabled(&mut self, feature: Feature, enabled: bool) -> bool {
        if let Err(err) = self.config.features.set_enabled(feature, enabled) {
            tracing::warn!(
                error = %err,
                feature = feature.key(),
                "failed to update constrained chat widget feature state"
            );
        }
        let enabled = self.config.features.enabled(feature);
        if feature == Feature::FastMode {
            self.refresh_effective_service_tier();
            self.sync_service_tier_commands();
        }
        if feature == Feature::Personality {
            self.sync_personality_command_enabled();
        }
        if feature == Feature::Plugins {
            self.sync_plugins_command_enabled();
            self.refresh_plugin_mentions();
        }
        if feature == Feature::Goals {
            self.sync_goal_command_enabled();
            if !enabled {
                self.current_goal_status_indicator = None;
                self.current_goal_status = None;
                self.turn_lifecycle.goal_status_active_turn_started_at = None;
                self.turn_lifecycle.budget_limited_turn_ids.clear();
                self.update_collaboration_mode_indicator();
            }
        }
        if feature == Feature::MentionsV2 {
            self.sync_mentions_v2_enabled();
        }
        if feature == Feature::PreventIdleSleep {
            self.turn_lifecycle.set_prevent_idle_sleep(enabled);
        }
        #[cfg(target_os = "windows")]
        if matches!(
            feature,
            Feature::WindowsSandbox | Feature::WindowsSandboxElevated
        ) {
            self.bottom_pane
                .set_windows_degraded_sandbox_active(matches!(
                    crate::tui_core::windows_sandbox::level_from_config(&self.config),
                    WindowsSandboxLevel::RestrictedToken
                ));
        }
        enabled
    }

    pub(crate) fn set_approvals_reviewer(&mut self, policy: ApprovalsReviewer) {
        self.config.approvals_reviewer = policy;
        self.refresh_status_surfaces();
    }

    pub(crate) fn set_world_writable_warning_acknowledged(&mut self, acknowledged: bool) {
        self.config.notices.hide_world_writable_warning = Some(acknowledged);
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn world_writable_warning_hidden(&self) -> bool {
        self.config
            .notices
            .hide_world_writable_warning
            .unwrap_or(false)
    }

    /// 覆盖在 Plan 模式激活时所使用的推理力度。
    ///
    /// 当当前激活的 mask 已经是 Plan 时，会立即应用该覆盖值，使底部状态栏无需等待下一次模式切换即可反映它。
    /// 传入 `None` 会重置为 Plan 模式预设的默认值。
    pub(crate) fn set_plan_mode_reasoning_effort(&mut self, effort: Option<ReasoningEffortConfig>) {
        self.config.plan_mode_reasoning_effort = effort.clone();
        if self.collaboration_modes_enabled()
            && let Some(mask) = self.active_collaboration_mask.as_mut()
            && mask.mode == Some(ModeKind::Plan)
        {
            if let Some(effort) = effort {
                mask.reasoning_effort = Some(Some(effort));
            } else if let Some(plan_mask) =
                collaboration_modes::plan_mask(self.model_catalog.as_ref())
            {
                mask.reasoning_effort = plan_mask.reasoning_effort;
            }
        }
        self.refresh_model_dependent_surfaces();
    }

    /// 为非 Plan 协作模式设置推理力度。
    ///
    /// 不会触及当前激活的 Plan mask —— Plan 推理完全由 Plan 预设和 `set_plan_mode_reasoning_effort` 控制。
    pub(crate) fn set_reasoning_effort(&mut self, effort: Option<ReasoningEffortConfig>) {
        self.current_collaboration_mode = self.current_collaboration_mode.with_updates(
            /*model*/ None,
            Some(effort.clone()),
            /*developer_instructions*/ None,
        );
        if self.collaboration_modes_enabled()
            && let Some(mask) = self.active_collaboration_mask.as_mut()
            && mask.mode != Some(ModeKind::Plan)
        {
            // 通用"全局默认值"的更新不应修改当前激活的 Plan mask。
            // Plan 推理由 Plan 预设和 Plan 专属的覆盖更新控制。
            mask.reasoning_effort = Some(effort);
        }
        self.refresh_model_dependent_surfaces();
    }

    /// 在 widget 的配置副本中设置人格。
    pub(crate) fn set_personality(&mut self, personality: Personality) {
        self.config.personality = Some(personality);
    }

    pub(crate) fn status_account_display(&self) -> Option<&StatusAccountDisplay> {
        self.status_account_display.as_ref()
    }

    pub(crate) fn runtime_model_provider_base_url(&self) -> Option<&str> {
        self.runtime_model_provider_base_url.as_deref()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn model_catalog(&self) -> Arc<ModelCatalog> {
        self.model_catalog.clone()
    }

    pub(crate) fn current_plan_type(&self) -> Option<PlanType> {
        self.plan_type
    }

    pub(crate) fn has_chatgpt_account(&self) -> bool {
        self.has_chatgpt_account
    }

    pub(crate) fn has_reflect_backend_auth(&self) -> bool {
        self.has_reflect_backend_auth
    }

    pub(crate) fn update_account_state(
        &mut self,
        status_account_display: Option<StatusAccountDisplay>,
        plan_type: Option<PlanType>,
        has_chatgpt_account: bool,
        has_reflect_backend_auth: bool,
    ) {
        // 账户更新通知是身份边界。不同账户之间可见的账户字段可能完全相同，因此始终使账户作用域的请求和数据失效。
        self.clear_pending_token_activity_refreshes();
        self.clear_pending_rate_limit_reset_requests();
        self.reflect_rate_limit_reached_type = None;
        self.reflect_spend_control_reached = None;
        self.rate_limit_warnings = RateLimitWarningState::default();
        self.rate_limit_switch_prompt = RateLimitSwitchPromptState::Idle;
        self.bottom_pane
            .dismiss_view_by_id(RATE_LIMIT_SWITCH_PROMPT_VIEW_ID);
        let had_refreshing_status_outputs = !self.refreshing_status_outputs.is_empty();
        let now = Local::now();
        for (_, handle) in self.refreshing_status_outputs.drain(..) {
            handle.finish_rate_limit_refresh(&[], now);
        }
        if had_refreshing_status_outputs {
            self.request_redraw();
        }
        self.status_line_workspace_headline = None;
        self.status_line_workspace_headline_pending_request_id = None;
        self.status_line_workspace_headline_last_requested_at = None;
        self.status_line_workspace_messages_disabled = false;
        self.status_account_display = status_account_display;
        self.plan_type = plan_type;
        self.has_chatgpt_account = has_chatgpt_account;
        self.has_reflect_backend_auth = has_reflect_backend_auth;
        self.bottom_pane
            .set_connectors_enabled(self.connectors_enabled());
        self.bottom_pane
            .set_token_activity_command_enabled(has_reflect_backend_auth);
        self.refresh_status_surfaces();
    }

    /// 在 widget 的配置副本中设置语法主题覆盖。
    pub(crate) fn set_tui_theme(&mut self, theme: Option<String>) {
        self.config.tui_theme = theme;
    }

    /// 在 widget 的配置副本以及已存储的协作模式中设置模型。
    pub(crate) fn set_model(&mut self, model: &str) {
        self.current_collaboration_mode = self.current_collaboration_mode.with_updates(
            Some(model.to_string()),
            /*effort*/ None,
            /*developer_instructions*/ None,
        );
        if self.collaboration_modes_enabled()
            && let Some(mask) = self.active_collaboration_mask.as_mut()
        {
            mask.model = Some(model.to_string());
        }
        self.refresh_effective_service_tier();
        self.refresh_model_dependent_surfaces();
    }

    pub(crate) fn current_model(&self) -> &str {
        if !self.collaboration_modes_enabled() {
            return self.current_collaboration_mode.model();
        }
        self.active_collaboration_mask
            .as_ref()
            .and_then(|mask| mask.model.as_deref())
            .unwrap_or_else(|| self.current_collaboration_mode.model())
    }

    pub(super) fn sync_personality_command_enabled(&mut self) {
        self.bottom_pane
            .set_personality_command_enabled(self.config.features.enabled(Feature::Personality));
    }

    pub(super) fn sync_plugins_command_enabled(&mut self) {
        self.bottom_pane
            .set_plugins_command_enabled(self.config.features.enabled(Feature::Plugins));
    }

    pub(super) fn sync_goal_command_enabled(&mut self) {
        self.bottom_pane
            .set_goal_command_enabled(self.config.features.enabled(Feature::Goals));
    }

    pub(super) fn sync_mentions_v2_enabled(&mut self) {
        self.bottom_pane
            .set_mentions_v2_enabled(self.config.features.enabled(Feature::MentionsV2));
    }

    pub(super) fn current_model_supports_personality(&self) -> bool {
        let model = self.current_model();
        self.model_catalog
            .try_list_models()
            .ok()
            .and_then(|models| {
                models
                    .into_iter()
                    .find(|preset| preset.model == model)
                    .map(|preset| preset.supports_personality)
            })
            .unwrap_or(false)
    }

    /// 返回当前生效模型是否声明支持图像输入。
    ///
    /// 当无法读取模型元数据时，我们刻意默认返回 `true`，以避免瞬态的目录读取失败在 UI 中硬性阻塞用户输入。
    pub(super) fn current_model_supports_images(&self) -> bool {
        let model = self.current_model();
        self.model_catalog
            .try_list_models()
            .ok()
            .and_then(|models| {
                models
                    .into_iter()
                    .find(|preset| preset.model == model)
                    .map(|preset| preset.input_modalities.contains(&InputModality::Image))
            })
            .unwrap_or(true)
    }

    pub(super) fn sync_image_paste_enabled(&mut self) {
        let enabled = self.current_model_supports_images();
        self.bottom_pane.set_image_paste_enabled(enabled);
    }

    pub(super) fn image_inputs_not_supported_message(&self) -> String {
        format!(
            "Model {} does not support image inputs. Remove images or switch models.",
            self.current_model()
        )
    }

    pub(crate) fn current_collaboration_mode(&self) -> &CollaborationMode {
        &self.current_collaboration_mode
    }

    pub(crate) fn current_reasoning_effort(&self) -> Option<ReasoningEffortConfig> {
        self.effective_reasoning_effort()
    }

    pub(crate) fn on_thread_settings_updated(
        &mut self,
        notification: ThreadSettingsUpdatedNotification,
    ) {
        let Ok(thread_id) = ThreadId::from_string(&notification.thread_id) else {
            tracing::warn!(
                thread_id = notification.thread_id,
                "ignoring app-server ThreadSettingsUpdated with invalid thread_id"
            );
            return;
        };
        if self.thread_id != Some(thread_id) {
            return;
        }

        self.apply_thread_settings(notification.thread_settings);
    }

    #[cfg(test)]
    pub(crate) fn active_collaboration_mode_kind(&self) -> ModeKind {
        self.active_mode_kind()
    }

    pub(super) fn is_session_configured(&self) -> bool {
        self.thread_id.is_some()
    }

    pub(super) fn collaboration_modes_enabled(&self) -> bool {
        true
    }

    /// 返回适用于当前可见草稿的关闭作用域。
    fn plan_mode_nudge_scope(&self) -> PlanModeNudgeScope {
        self.thread_id
            .map_or(PlanModeNudgeScope::NewThread, PlanModeNudgeScope::Thread)
    }

    /// 返回当前草稿是否应当用 Plan 模式提示替换正常的底部状态栏。
    ///
    /// `ChatWidget` 拥有该策略，因为它能够将基于词法的草稿匹配与模式可用性、交互状态以及线程作用域的关闭结合起来。
    /// `ChatComposer` 仅渲染最终得到的可见性位。在此处把斜杠命令和 shell 草稿排除在外，可以避免在用户刻意编写其他本地命令时还提示切换模式。
    pub(super) fn should_show_plan_mode_nudge(&self) -> bool {
        let text = self.bottom_pane.composer_text();
        let trimmed = text.trim_start();
        self.collaboration_modes_enabled()
            && collaboration_modes::plan_mask(self.model_catalog.as_ref()).is_some()
            && self.active_mode_kind() != ModeKind::Plan
            && self.bottom_pane.composer_input_enabled()
            && !self.bottom_pane.is_task_running()
            && self.bottom_pane.no_modal_or_popup_active()
            && !trimmed.starts_with('/')
            && !trimmed.starts_with('!')
            && contains_plan_keyword(&text)
            && !self
                .dismissed_plan_mode_nudge_scopes
                .contains(&self.plan_mode_nudge_scope())
    }

    /// 根据当前 Plan 模式提示策略同步底部状态栏的呈现。
    pub(super) fn refresh_plan_mode_nudge(&mut self) {
        self.bottom_pane
            .set_plan_mode_nudge_visible(self.should_show_plan_mode_nudge());
    }

    /// 在当前线程作用域内隐藏该提示，直至用户切换会话上下文。
    pub(super) fn dismiss_plan_mode_nudge(&mut self) {
        self.dismissed_plan_mode_nudge_scopes
            .insert(self.plan_mode_nudge_scope());
        self.refresh_plan_mode_nudge();
    }

    pub(super) fn initial_collaboration_mask(
        _config: &Config,
        model_catalog: &ModelCatalog,
        model_override: Option<&str>,
    ) -> Option<CollaborationModeMask> {
        let mut mask = collaboration_modes::default_mask(model_catalog)?;
        if let Some(model_override) = model_override {
            mask.model = Some(model_override.to_string());
        }
        Some(mask)
    }

    pub(super) fn active_mode_kind(&self) -> ModeKind {
        self.active_collaboration_mask
            .as_ref()
            .and_then(|mask| mask.mode)
            .unwrap_or(ModeKind::Default)
    }

    pub(super) fn effective_reasoning_effort(&self) -> Option<ReasoningEffortConfig> {
        if !self.collaboration_modes_enabled() {
            return self.current_collaboration_mode.reasoning_effort();
        }
        let current_effort = self.current_collaboration_mode.reasoning_effort();
        self.active_collaboration_mask
            .as_ref()
            .and_then(|mask| mask.reasoning_effort.clone())
            .unwrap_or(current_effort)
    }

    pub(crate) fn effective_collaboration_mode(&self) -> CollaborationMode {
        if !self.collaboration_modes_enabled() {
            return self.current_collaboration_mode.clone();
        }
        self.active_collaboration_mask.as_ref().map_or_else(
            || self.current_collaboration_mode.clone(),
            |mask| self.current_collaboration_mode.apply_mask(mask),
        )
    }

    pub(super) fn refresh_model_display(&mut self) {
        let effective = self.effective_collaboration_mode();
        self.session_header.set_model(effective.model());
        // 使 composer 的粘贴能力与当前生效的模型保持一致。
        self.sync_image_paste_enabled();
        self.sync_service_tier_commands();
        self.refresh_terminal_title();
        let effort = self.effective_reasoning_effort();
        self.bottom_pane
            .set_active_reasoning_effort(effort.as_ref());
    }

    /// 刷新每个依赖生效模型、推理
    /// 力度或协作模式的 UI 表面。
    ///
    /// 在任何变更 `current_collaboration_mode`、`active_collaboration_mask`
    /// 或按模式的推理力度覆盖项的 setter 末尾调用此方法。
    /// 在此合并两次刷新，可防止调用方更新
    /// 头部/标题（`refresh_model_display`）却忘记页脚状态行
    /// （`refresh_status_line`）的缺陷。
    pub(super) fn refresh_model_dependent_surfaces(&mut self) {
        self.refresh_model_display();
        self.refresh_status_line();
    }

    fn apply_thread_settings(&mut self, mut settings: ThreadSettings) {
        let cwd_changed = self.config.cwd != settings.cwd.clone().into();
        self.apply_thread_settings_cwd(settings.cwd.clone().into());
        self.config.model_provider_id = settings.model_provider.clone();
        self.set_service_tier(settings.service_tier.clone());
        self.set_approval_policy(settings.approval_policy);
        self.set_approvals_reviewer(settings.approvals_reviewer.to_core());
        self.config.personality = settings.personality.clone().map(|s| {
            use crate::protocol_compat::config_types::Personality;
            match s.to_ascii_lowercase().as_str() {
                "friendly" => Personality::Friendly,
                "pragmatic" => Personality::Pragmatic,
                _ => Personality::None,
            }
        });

        let permission_profile = PermissionProfile::from_legacy_sandbox_policy_for_cwd(
            "default",
            settings.cwd.as_path(),
        );
        let permission_snapshot = PermissionProfileSnapshot::from_session_snapshot(
            permission_profile,
            settings.active_permission_profile.take().map(Into::into),
        );
        if let Err(err) = self
            .config
            .permissions
            .set_permission_profile_from_session_snapshot(permission_snapshot.clone())
        {
            tracing::warn!(%err, "failed to sync permissions from ThreadSettingsUpdated");
            if let Err(replace_err) = self
                .config
                .permissions
                .replace_permission_profile_from_session_snapshot(permission_snapshot)
            {
                tracing::error!(
                    %replace_err,
                    "failed to replace permissions from ThreadSettingsUpdated after constraint fallback"
                );
            }
        }

        if let Some(ref mut mode) = settings.collaboration_mode {
            mode.settings.model = settings.model.clone();
            mode.settings.reasoning_effort = settings
                .effort
                .clone()
                .map(crate::protocol_compat::openai_models::ReasoningEffort::from);
        }
        self.set_effective_collaboration_mode(settings.collaboration_mode.unwrap_or_default());
        self.refresh_effective_service_tier();
        self.refresh_status_surfaces();
        self.sync_service_tier_commands();
        self.sync_personality_command_enabled();
        if cwd_changed {
            self.refresh_skills_for_current_cwd(/*force_reload*/ true);
        }
        self.refresh_plugin_mentions();
        self.request_redraw();
    }

    fn apply_thread_settings_cwd(&mut self, cwd: AbsolutePathBuf) {
        let previous_cwd = std::mem::replace(&mut self.config.cwd, cwd.clone());
        self.current_cwd = Some(cwd.to_path_buf());
        self.status_line_project_root_name_cache = None;

        if !self.config.workspace_roots.contains(&previous_cwd) {
            return;
        }

        let previous_roots = std::mem::take(&mut self.config.workspace_roots);
        self.config.workspace_roots.push(cwd);
        for root in previous_roots {
            if root != previous_cwd && !self.config.workspace_roots.contains(&root) {
                self.config.workspace_roots.push(root);
            }
        }
        self.config
            .permissions
            .set_workspace_roots(self.config.workspace_roots.clone());
    }

    pub(super) fn set_effective_collaboration_mode(&mut self, mode: CollaborationMode) {
        let mode_kind = mode.mode;
        let settings = mode.settings;
        if mode_kind == ModeKind::Default {
            self.current_collaboration_mode = CollaborationMode {
                mode: ModeKind::Default,
                settings: settings.clone(),
            };
        }
        self.active_collaboration_mask = Some(CollaborationModeMask {
            name: mode_kind.display_name().to_string(),
            mode: Some(mode_kind),
            model: Some(settings.model.clone()),
            reasoning_effort: Some(settings.reasoning_effort.clone()),
            developer_instructions: Some(settings.developer_instructions),
        });
        self.update_collaboration_mode_indicator();
        self.refresh_plan_mode_nudge();
        self.refresh_model_dependent_surfaces();
    }

    pub(super) fn model_display_name(&self) -> &str {
        let model = self.current_model();
        if model.is_empty() {
            DEFAULT_MODEL_DISPLAY_NAME
        } else {
            model
        }
    }

    /// 获取当前协作模式的标签。
    pub(super) fn collaboration_mode_label(&self) -> Option<&'static str> {
        if !self.collaboration_modes_enabled() {
            return None;
        }
        let active_mode = self.active_mode_kind();
        active_mode
            .is_tui_visible()
            .then_some(active_mode.display_name())
    }

    fn collaboration_mode_indicator(&self) -> Option<CollaborationModeIndicator> {
        if !self.collaboration_modes_enabled() {
            return None;
        }
        match self.active_mode_kind() {
            ModeKind::Plan => Some(CollaborationModeIndicator::Plan),
            ModeKind::Default | ModeKind::PairProgramming | ModeKind::Execute => None,
        }
    }

    pub(super) fn update_collaboration_mode_indicator(&mut self) {
        let indicator = self.collaboration_mode_indicator();
        let goal_indicator = if indicator.is_none() {
            self.goal_status_indicator(Instant::now())
        } else {
            None
        };
        self.current_goal_status_indicator = goal_indicator.clone();
        self.bottom_pane.set_collaboration_mode_indicator(indicator);
        self.bottom_pane.set_goal_status_indicator(goal_indicator);
    }

    pub(super) fn refresh_goal_status_indicator_for_time_tick(&mut self) {
        if self.collaboration_mode_indicator().is_some() {
            return;
        }
        let goal_indicator = self.goal_status_indicator(Instant::now());
        if goal_indicator != self.current_goal_status_indicator {
            self.current_goal_status_indicator = goal_indicator.clone();
            self.bottom_pane.set_goal_status_indicator(goal_indicator);
        }
    }

    fn goal_status_indicator(&self, now: Instant) -> Option<GoalStatusIndicator> {
        if !self.config.features.enabled(Feature::Goals) {
            return None;
        }
        self.current_goal_status.as_ref().and_then(|state| {
            state.indicator(now, self.turn_lifecycle.goal_status_active_turn_started_at)
        })
    }

    pub(super) fn on_thread_goal_updated(&mut self, goal: AppThreadGoal, turn_id: Option<String>) {
        if let Some(active_thread_id) = self.thread_id
            && active_thread_id.to_string() != goal.thread_id
        {
            return;
        }
        if !self.config.features.enabled(Feature::Goals) {
            self.current_goal_status_indicator = None;
            self.current_goal_status = None;
            self.update_collaboration_mode_indicator();
            return;
        }
        if goal.status == AppThreadGoalStatus::BudgetLimited
            && let Some(turn_id) = turn_id
        {
            self.turn_lifecycle.mark_budget_limited(turn_id);
        }
        self.current_goal_status = Some(GoalStatusState::new(goal, Instant::now()));
        self.update_collaboration_mode_indicator();
    }

    /// 循环切换到下一个协作模式变体（Plan -> Default -> Plan）。
    pub(super) fn cycle_collaboration_mode(&mut self) {
        if !self.collaboration_modes_enabled() {
            return;
        }

        if let Some(next_mask) = collaboration_modes::next_mask(
            self.model_catalog.as_ref(),
            self.active_collaboration_mask.as_ref(),
        ) {
            self.set_collaboration_mask_from_user_action(next_mask);
        }
    }

    pub(crate) fn set_collaboration_mask_from_user_action(&mut self, mask: CollaborationModeMask) {
        self.set_collaboration_mask(mask);
        self.submit_collaboration_mode_settings_update();
    }

    /// 更新活动协作掩码。
    ///
    /// 协作模式启用且选中预置时，当前模式会以
    /// `Op::UserTurn { collaboration_mode: Some(...) }` 附加到提交中。
    pub(crate) fn set_collaboration_mask(&mut self, mut mask: CollaborationModeMask) {
        if !self.collaboration_modes_enabled() {
            return;
        }
        let previous_mode = self.active_mode_kind();
        let previous_model = self.current_model().to_string();
        let previous_effort = self.effective_reasoning_effort();
        if mask.mode == Some(ModeKind::Plan)
            && let Some(effort) = self.config.plan_mode_reasoning_effort.clone()
        {
            mask.reasoning_effort = Some(Some(effort));
        }
        if mask.mode == Some(ModeKind::Plan) {
            self.dismissed_plan_mode_nudge_scopes
                .insert(self.plan_mode_nudge_scope());
        }
        self.active_collaboration_mask = Some(mask);
        self.update_collaboration_mode_indicator();
        self.refresh_plan_mode_nudge();
        self.refresh_model_dependent_surfaces();
        let next_mode = self.active_mode_kind();
        let next_model = self.current_model();
        let next_effort = self.effective_reasoning_effort();
        if previous_mode != next_mode
            && (previous_model != next_model || previous_effort != next_effort)
        {
            let mut message = format!("Model changed to {next_model}");
            if !next_model.starts_with("reflect-auto-") {
                let reasoning_label = match next_effort.as_ref() {
                    None | Some(ReasoningEffortConfig::None) => "default",
                    Some(effort) => effort.as_str(),
                };
                message.push(' ');
                message.push_str(reasoning_label);
            }
            message.push_str(" for ");
            message.push_str(next_mode.display_name());
            message.push_str(" mode.");
            self.add_info_message(message, /*hint*/ None);
        }
        self.request_redraw();
    }

    fn submit_collaboration_mode_settings_update(&self) {
        let Some(thread_id) = self.thread_id else {
            return;
        };
        self.app_event_tx.send(AppEvent::SubmitThreadOp {
            thread_id,
            op: AppCommand::override_turn_context(
                /*cwd*/ None,
                /*approval_policy*/ None,
                /*approvals_reviewer*/ None,
                /*permission_profile*/ None,
                /*active_permission_profile*/ None,
                /*windows_sandbox_level*/ None,
                /*model*/ None,
                /*effort*/ None,
                /*summary*/ None,
                /*service_tier*/ None,
                Some(self.effective_collaboration_mode()),
                /*personality*/ None,
            ),
        });
    }
}
