//! 引导（onboarding）屏幕的编排与顶层键盘路由。
//!
//! 引导流程是一个在可见步骤（welcome/auth/trust）上的小型状态机。
//! 本模块决定哪些步骤接收按键和粘贴事件，
//! 并强制执行跨越各个步骤 widget 的流程级安全规则。
//!
//! 特别是，引导的退出处理对 API key 输入有文本录入保护：
//! 当用户正在编辑非空的 API key 字段时，可打印的 `q` 退出键被当作文本输入处理，
//! 而 control/alt 组合键仍然作为显式的退出快捷键可用。

use crate::app_server_client::AppServerEvent;
use crate::app_server_client::AppServerRequestHandle;
use crate::app_server_protocol::ServerNotification;
use crate::git_utils::resolve_root_git_project_for_trust;
#[cfg(target_os = "windows")]
use crate::protocol_compat::config_types::WindowsSandboxLevel;
use crate::utils_absolute_path::AbsolutePathBuf;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Color;
use ratatui::widgets::Clear;
use ratatui::widgets::WidgetRef;

use crate::protocol_compat::config_types::ForcedLoginMethod;

use crate::login::LoginStatus;
use crate::tui_core::app_server_session::AppServerSession;
use crate::tui_core::config_update::format_config_error;
use crate::tui_core::config_update::write_trusted_project;
use crate::tui_core::key_hint::KeyBindingListExt;
use crate::tui_core::legacy_core::config::Config;
use crate::tui_core::onboarding::auth::AuthModeWidget;
use crate::tui_core::onboarding::auth::SignInOption;
use crate::tui_core::onboarding::auth::SignInState;
use crate::tui_core::onboarding::keys;
use crate::tui_core::onboarding::trust_directory::TrustDirectorySelection;
use crate::tui_core::onboarding::trust_directory::TrustDirectoryWidget;
use crate::tui_core::onboarding::welcome::WelcomeWidget;
use crate::tui_core::tui::FrameRequester;
use crate::tui_core::tui::Tui;
use crate::tui_core::tui::TuiEvent;
use color_eyre::eyre::Result;
use std::sync::Arc;
use std::sync::RwLock;

#[allow(clippy::large_enum_variant)]
enum Step {
    Welcome(WelcomeWidget),
    Auth(AuthModeWidget),
    TrustDirectory(TrustDirectoryWidget),
}

pub(crate) trait KeyboardHandler {
    fn handle_key_event(&mut self, key_event: KeyEvent);
    fn handle_paste(&mut self, _pasted: String) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepState {
    Hidden,
    InProgress,
    Complete,
}

pub(crate) trait StepStateProvider {
    fn get_step_state(&self) -> StepState;
}

pub(crate) struct OnboardingScreen {
    request_frame: FrameRequester,
    steps: Vec<Step>,
    is_done: bool,
    should_exit: bool,
}

pub(crate) struct OnboardingScreenArgs {
    pub show_trust_screen: bool,
    pub show_login_screen: bool,
    pub login_status: LoginStatus,
    pub app_server_request_handle: Option<AppServerRequestHandle>,
    pub config: Config,
}

pub(crate) struct OnboardingResult {
    pub directory_trust_persisted: bool,
    pub should_exit: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ApiKeyEntryContext {
    /// 当引导当前正在渲染 API key 输入状态时为 true。
    active: bool,
    /// 当 API key 输入字段当前包含用户文本时为 true。
    has_text: bool,
}

impl OnboardingScreen {
    pub(crate) async fn new(tui: &mut Tui, args: OnboardingScreenArgs) -> Self {
        let OnboardingScreenArgs {
            show_trust_screen,
            show_login_screen,
            login_status,
            app_server_request_handle,
            config,
        } = args;
        let cwd = config.cwd.to_path_buf();
        let forced_login_method = config.forced_login_method;
        let mut steps: Vec<Step> = Vec::new();
        steps.push(Step::Welcome(WelcomeWidget::new(
            !matches!(login_status, LoginStatus::NotAuthenticated),
            tui.frame_requester(),
            config.animations,
        )));
        if show_login_screen {
            let highlighted_mode = match forced_login_method {
                Some(ForcedLoginMethod::Api) => SignInOption::ApiKey,
                _ => SignInOption::ChatGpt,
            };
            if let Some(app_server_request_handle) = app_server_request_handle {
                steps.push(Step::Auth(AuthModeWidget {
                    request_frame: tui.frame_requester(),
                    highlighted_mode,
                    error: Arc::new(RwLock::new(None)),
                    sign_in_state: Arc::new(RwLock::new(SignInState::PickMode)),
                    login_status,
                    app_server_request_handle,
                    forced_login_method,
                    animations_enabled: config.animations,
                    animations_suppressed: std::cell::Cell::new(false),
                }));
            } else {
                tracing::warn!("skipping onboarding login step without app-server request handle");
            }
        }
        #[cfg(target_os = "windows")]
        let show_windows_create_sandbox_hint =
            crate::tui_core::windows_sandbox::level_from_config(&config)
                == WindowsSandboxLevel::Disabled;
        #[cfg(not(target_os = "windows"))]
        let show_windows_create_sandbox_hint = false;
        let highlighted = TrustDirectorySelection::Trust;
        if show_trust_screen {
            let trust_target = resolve_root_git_project_for_trust(&config.cwd)
                .map(Into::into)
                .unwrap_or_else(|| cwd.clone());
            steps.push(Step::TrustDirectory(TrustDirectoryWidget {
                cwd,
                trust_target,
                show_windows_create_sandbox_hint,
                should_quit: false,
                selection: None,
                highlighted,
                error: None,
            }))
        }
        Self {
            request_frame: tui.frame_requester(),
            steps,
            is_done: false,
            should_exit: false,
        }
    }

    fn current_steps_mut(&mut self) -> Vec<&mut Step> {
        let mut out: Vec<&mut Step> = Vec::new();
        for step in self.steps.iter_mut() {
            match step.get_step_state() {
                StepState::Hidden => continue,
                StepState::Complete => out.push(step),
                StepState::InProgress => {
                    out.push(step);
                    break;
                }
            }
        }
        out
    }

    fn current_steps(&self) -> Vec<&Step> {
        let mut out: Vec<&Step> = Vec::new();
        for step in self.steps.iter() {
            match step.get_step_state() {
                StepState::Hidden => continue,
                StepState::Complete => out.push(step),
                StepState::InProgress => {
                    out.push(step);
                    break;
                }
            }
        }
        out
    }

    fn should_suppress_animations(&self) -> bool {
        // 当认证显示可复制的登录材料时，冻结整个引导屏幕，
        // 这样终端选区就不会被重绘打断。
        self.current_steps().into_iter().any(|step| match step {
            Step::Auth(widget) => widget.should_suppress_animations(),
            Step::Welcome(_) | Step::TrustDirectory(_) => false,
        })
    }

    fn is_auth_in_progress(&self) -> bool {
        self.steps.iter().any(|step| {
            matches!(step, Step::Auth(_)) && matches!(step.get_step_state(), StepState::InProgress)
        })
    }

    pub(crate) fn is_done(&self) -> bool {
        self.is_done
            || !self
                .steps
                .iter()
                .any(|step| matches!(step.get_step_state(), StepState::InProgress))
    }

    pub fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn cancel_auth_if_active(&self) {
        for step in &self.steps {
            if let Step::Auth(widget) = step {
                widget.cancel_active_attempt();
            }
        }
    }

    fn auth_widget_mut(&mut self) -> Option<&mut AuthModeWidget> {
        self.steps.iter_mut().find_map(|step| match step {
            Step::Auth(widget) => Some(widget),
            Step::Welcome(_) | Step::TrustDirectory(_) => None,
        })
    }

    fn handle_app_server_notification(&mut self, notification: ServerNotification) {
        match notification {
            ServerNotification::AccountLoginCompleted(notification) => {
                if let Some(widget) = self.auth_widget_mut() {
                    widget.on_account_login_completed(notification);
                }
            }
            ServerNotification::AccountUpdated(notification) => {
                if let Some(widget) = self.auth_widget_mut() {
                    widget.on_account_updated(notification);
                }
            }
            _ => {}
        }
    }

    fn api_key_entry_context(&self) -> ApiKeyEntryContext {
        self.steps
            .iter()
            .find_map(|step| {
                if let Step::Auth(widget) = step {
                    Some(ApiKeyEntryContext {
                        active: widget.is_api_key_entry_active(),
                        has_text: widget.api_key_entry_has_text(),
                    })
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }
}

impl KeyboardHandler for OnboardingScreen {
    /// 将按键事件路由到引导步骤，同时保持文本录入安全性。
    ///
    /// 在 API key 输入模式下，只有当用户开始向 API key 字段输入文本后，
    /// 才会屏蔽可打印的退出绑定。这保证在空字段上可打印的 `q` 退出键仍然可用，
    /// 同时保护正在进行的文本录入避免意外退出。
    /// Control/alt 退出的组合键仍然作为紧急退出来使用。
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        let api_key_entry_context = self.api_key_entry_context();
        let should_quit = key_event.kind == KeyEventKind::Press
            && keys::QUIT.is_pressed(key_event)
            && !suppress_quit_while_typing_api_key(key_event, api_key_entry_context);
        if should_quit {
            if self.is_auth_in_progress() {
                self.cancel_auth_if_active();
                // 如果用户取消了认证菜单，则退出应用，
                // 而不是让用户处于未登录的提示界面。
                self.should_exit = true;
            }
            self.is_done = true;
        } else {
            if let Some(Step::Welcome(widget)) = self
                .steps
                .iter_mut()
                .find(|step| matches!(step, Step::Welcome(_)))
            {
                widget.handle_key_event(key_event);
            }
            if let Some(active_step) = self.current_steps_mut().into_iter().last() {
                active_step.handle_key_event(key_event);
            }
            if self.steps.iter().any(|step| {
                if let Step::TrustDirectory(widget) = step {
                    widget.should_quit()
                } else {
                    false
                }
            }) {
                self.should_exit = true;
                self.is_done = true;
            }
        }
        self.request_frame.schedule_frame();
    }

    fn handle_paste(&mut self, pasted: String) {
        if pasted.is_empty() {
            return;
        }

        if let Some(active_step) = self.current_steps_mut().into_iter().last() {
            active_step.handle_paste(pasted);
        }
        self.request_frame.schedule_frame();
    }
}

/// 当退出快捷键应被当作文本输入而忽略时返回 `true`。
///
/// 此规则仅在 API key 输入处于激活状态、按键是不带 control/alt 修饰键
/// 的可打印字符，且输入字段中已有文本时才适用。
/// 空输入场景下有意不触发屏蔽，以保证可打印的 `q` 退出键仍能退出引导。
fn suppress_quit_while_typing_api_key(
    key_event: KeyEvent,
    api_key_entry_context: ApiKeyEntryContext,
) -> bool {
    api_key_entry_context.active
        && api_key_entry_context.has_text
        && matches!(key_event.code, KeyCode::Char(_))
        && !key_event
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

impl WidgetRef for &OnboardingScreen {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let suppress_animations = self.should_suppress_animations();
        for step in self.current_steps() {
            match step {
                Step::Welcome(widget) => widget.set_animations_suppressed(suppress_animations),
                Step::Auth(widget) => widget.set_animations_suppressed(suppress_animations),
                Step::TrustDirectory(_) => {}
            }
        }

        Clear.render(area, buf);
        // 按从上到下的顺序渲染各步骤，并动态测量每个步骤的高度。
        let mut y = area.y;
        let bottom = area.y.saturating_add(area.height);
        let width = area.width;

        // 辅助函数：扫描临时缓冲区，返回已使用的行数。
        fn used_rows(tmp: &Buffer, width: u16, height: u16) -> u16 {
            if width == 0 || height == 0 {
                return 0;
            }
            let mut last_non_empty: Option<u16> = None;
            for yy in 0..height {
                let mut any = false;
                for xx in 0..width {
                    let cell = &tmp[(xx, yy)];
                    let has_symbol = !cell.symbol().trim().is_empty();
                    let has_style = cell.fg != Color::Reset
                        || cell.bg != Color::Reset
                        || !cell.modifier.is_empty();
                    if has_symbol || has_style {
                        any = true;
                        break;
                    }
                }
                if any {
                    last_non_empty = Some(yy);
                }
            }
            last_non_empty.map(|v| v + 2).unwrap_or(0)
        }

        let mut i = 0usize;
        let current_steps = self.current_steps();

        while i < current_steps.len() && y < bottom {
            let step = &current_steps[i];
            let max_h = bottom.saturating_sub(y);
            if max_h == 0 || width == 0 {
                break;
            }
            let scratch_area = Rect::new(0, 0, width, max_h);
            let mut scratch = Buffer::empty(scratch_area);
            if let Step::Welcome(widget) = step {
                widget.update_layout_area(scratch_area);
            }
            step.render_ref(scratch_area, &mut scratch);
            let h = used_rows(&scratch, width, max_h).min(max_h);
            if h > 0 {
                let target = Rect {
                    x: area.x,
                    y,
                    width,
                    height: h,
                };
                Clear.render(target, buf);
                step.render_ref(target, buf);
                y = y.saturating_add(h);
            }
            i += 1;
        }
    }
}

impl KeyboardHandler for Step {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match self {
            Step::Welcome(widget) => widget.handle_key_event(key_event),
            Step::Auth(widget) => widget.handle_key_event(key_event),
            Step::TrustDirectory(widget) => widget.handle_key_event(key_event),
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        match self {
            Step::Welcome(_) => {}
            Step::Auth(widget) => widget.handle_paste(pasted),
            Step::TrustDirectory(widget) => widget.handle_paste(pasted),
        }
    }
}

impl StepStateProvider for Step {
    fn get_step_state(&self) -> StepState {
        match self {
            Step::Welcome(w) => w.get_step_state(),
            Step::Auth(w) => w.get_step_state(),
            Step::TrustDirectory(w) => w.get_step_state(),
        }
    }
}

impl WidgetRef for Step {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        match self {
            Step::Welcome(widget) => {
                widget.render_ref(area, buf);
            }
            Step::Auth(widget) => {
                widget.render_ref(area, buf);
            }
            Step::TrustDirectory(widget) => {
                widget.render_ref(area, buf);
            }
        }
    }
}

pub(crate) async fn run_onboarding_app(
    args: OnboardingScreenArgs,
    mut app_server: Option<&mut AppServerSession>,
    tui: &mut Tui,
) -> Result<OnboardingResult> {
    use tokio_stream::StreamExt;

    let app_server_request_handle = args.app_server_request_handle.clone();
    let mut onboarding_screen = OnboardingScreen::new(tui, args).await;
    let mut directory_trust_persisted = false;
    // 一次性守卫：在显示 ChatGPT 登录成功消息后，彻底清屏一次
    let mut did_full_clear_after_success = false;

    tui.draw(u16::MAX, |frame| {
        frame.render_widget_ref(&onboarding_screen, frame.area());
    })?;

    let tui_events = tui.event_stream();
    tokio::pin!(tui_events);

    while !onboarding_screen.is_done() {
        tokio::select! {
            event = tui_events.next() => {
                if let Some(event) = event {
                    match event {
                        TuiEvent::FocusGained | TuiEvent::FocusLost | TuiEvent::Tick | TuiEvent::Frame => {}
                        TuiEvent::Key(key_event) => {
                            onboarding_screen.handle_key_event(key_event);
                            if !directory_trust_persisted {
                                directory_trust_persisted = persist_selected_trust(
                                    &mut onboarding_screen,
                                    app_server_request_handle.clone(),
                                )
                                .await;
                            }
                        }
                        TuiEvent::Paste(text) => {
                            onboarding_screen.handle_paste(text);
                        }
                        TuiEvent::Draw | TuiEvent::Resize(_, _) => {
                            if !did_full_clear_after_success
                                && onboarding_screen.steps.iter().any(|step| {
                                    if let Step::Auth(w) = step {
                                        w.sign_in_state.read().is_ok_and(|g| {
                                            matches!(&*g, super::auth::SignInState::ChatGptSuccessMessage)
                                        })
                                    } else {
                                        false
                                    }
                                })
                            {
                                // 在清屏之前重置任何残留的 SGR（下划线/颜色）
                                let _ = ratatui::crossterm::execute!(
                                    std::io::stdout(),
                                    ratatui::crossterm::style::SetAttribute(
                                        ratatui::crossterm::style::Attribute::Reset
                                    ),
                                    ratatui::crossterm::style::SetAttribute(
                                        ratatui::crossterm::style::Attribute::NoUnderline
                                    ),
                                    ratatui::crossterm::style::SetForegroundColor(
                                        ratatui::crossterm::style::Color::Reset
                                    ),
                                    ratatui::crossterm::style::SetBackgroundColor(
                                        ratatui::crossterm::style::Color::Reset
                                    )
                                );
                                let _ = tui.terminal.clear();
                                did_full_clear_after_success = true;
                            }
                            let _ = tui.draw(u16::MAX, |frame| {
                                frame.render_widget_ref(&onboarding_screen, frame.area());
                            });
                        }
                    }
                }
            }
            app_event = async {
                match app_server.as_mut() {
                    Some(app_server) => app_server.next_event().await,
                    None => None,
                }
            }, if app_server.is_some() => {
                if let Some(event) = app_event {
                    match event {
                        AppServerEvent::Empty => {}
                        AppServerEvent::ServerNotification(notification) => {
                            onboarding_screen.handle_app_server_notification(notification);
                        }
                        AppServerEvent::Disconnected { message } => {
                            return Err(color_eyre::eyre::eyre!(message));
                        }
                        AppServerEvent::Lagged { .. }
                        | AppServerEvent::ServerRequest(_) => {}
                    }
                }
            }
        }
    }
    Ok(OnboardingResult {
        directory_trust_persisted,
        should_exit: onboarding_screen.should_exit(),
    })
}

async fn persist_selected_trust(
    onboarding_screen: &mut OnboardingScreen,
    request_handle: Option<AppServerRequestHandle>,
) -> bool {
    let Some((trust_step_index, trust_target)) = onboarding_screen
        .steps
        .iter()
        .enumerate()
        .find_map(|(index, step)| {
            if let Step::TrustDirectory(widget) = step
                && widget.selection == Some(TrustDirectorySelection::Trust)
            {
                return Some((index, widget.trust_target.clone()));
            }
            None
        })
    else {
        return false;
    };

    let result = match request_handle {
        Some(request_handle) => {
            write_trusted_project(request_handle, &AbsolutePathBuf::from(trust_target.clone()))
                .await
                .map(|_| ())
        }
        None => Err(color_eyre::eyre::eyre!("app server unavailable")),
    };

    match result {
        Ok(()) => true,
        Err(error) => {
            let error = format_config_error(&error);
            tracing::error!(
                "failed to persist trusted project state for {}: {error}",
                trust_target.display()
            );
            if let Step::TrustDirectory(widget) = &mut onboarding_screen.steps[trust_step_index] {
                widget.selection = None;
                widget.error = Some(format!(
                    "Failed to set trust for {}: {error}",
                    trust_target.display()
                ));
            }
            false
        }
    }
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests {
    use super::ApiKeyEntryContext;
    use super::OnboardingScreen;
    use super::Step;
    use super::StepStateProvider;
    use super::persist_selected_trust;
    use super::suppress_quit_while_typing_api_key;
    use crate::tui_core::onboarding::trust_directory::TrustDirectorySelection;
    use crate::tui_core::onboarding::trust_directory::TrustDirectoryWidget;
    use crate::tui_core::tui::FrameRequester;
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn suppresses_printable_quit_key_during_api_key_entry() {
        let suppressed = suppress_quit_while_typing_api_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            ApiKeyEntryContext {
                active: true,
                has_text: true,
            },
        );
        assert!(suppressed);
    }

    #[test]
    fn does_not_suppress_printable_quit_key_when_api_key_input_is_empty() {
        let suppressed = suppress_quit_while_typing_api_key(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            ApiKeyEntryContext {
                active: true,
                has_text: false,
            },
        );
        assert!(!suppressed);
    }

    #[test]
    fn does_not_suppress_control_quit_key_during_api_key_entry() {
        let suppressed = suppress_quit_while_typing_api_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            ApiKeyEntryContext {
                active: true,
                has_text: true,
            },
        );
        assert!(!suppressed);
    }

    #[test]
    fn does_not_suppress_when_not_in_api_key_entry() {
        let suppressed = suppress_quit_while_typing_api_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            ApiKeyEntryContext {
                active: false,
                has_text: true,
            },
        );
        assert!(!suppressed);
    }

    #[tokio::test]
    async fn trust_persistence_failure_keeps_trust_step_in_progress() {
        let mut onboarding_screen = OnboardingScreen {
            request_frame: FrameRequester::test_dummy(),
            steps: vec![Step::TrustDirectory(TrustDirectoryWidget {
                cwd: PathBuf::from("/workspace/project"),
                trust_target: PathBuf::from("/workspace/project"),
                show_windows_create_sandbox_hint: false,
                should_quit: false,
                selection: Some(TrustDirectorySelection::Trust),
                highlighted: TrustDirectorySelection::Trust,
                error: None,
            })],
            is_done: false,
            should_exit: false,
        };

        let persisted =
            persist_selected_trust(&mut onboarding_screen, /*request_handle*/ None).await;

        assert!(!persisted);
        let Step::TrustDirectory(widget) = &onboarding_screen.steps[0] else {
            panic!("trust step should remain present");
        };
        assert_eq!(widget.selection, None);
        assert_eq!(widget.get_step_state(), super::StepState::InProgress);
        assert!(
            widget
                .error
                .as_deref()
                .is_some_and(|error| error.contains("app server unavailable"))
        );
    }
}
