//! 登录鉴权（onboarding/auth） 的测试集。
//!
//! 从 onboarding/auth.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::app_server_client::AppServerRequestHandle;
use crate::app_server_client::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use crate::app_server_client::InProcessAppServerClient;
use crate::app_server_client::InProcessClientStartArgs;
use crate::cloud_config::cloud_config_bundle_loader_for_storage;
use crate::config_compat::types::AuthCredentialsStoreMode;
use crate::login::AuthKeyringBackendKind;
use crate::tui_core::legacy_core::config::ConfigBuilder;
use arg0::Arg0DispatchPaths;

use pretty_assertions::assert_eq;
use std::sync::Arc;
use tempfile::TempDir;

async fn widget_forced_chatgpt() -> (AuthModeWidget, TempDir) {
    let reflect_home = TempDir::new().unwrap();
    let reflect_home_path = reflect_home.path().to_path_buf();
    let config = ConfigBuilder::default()
        .reflect_home(reflect_home_path.clone())
        .build()
        .await
        .unwrap();
    let client = InProcessAppServerClient::start(InProcessClientStartArgs {
        arg0_paths: Arg0DispatchPaths::default(),
        config: Arc::new(config),
        cli_overrides: Vec::new(),
        loader_overrides: Default::default(),
        strict_config: false,
        cloud_config_bundle: cloud_config_bundle_loader_for_storage(
            reflect_home_path.clone(),
            /*enable_reflect_api_key_env*/ false,
            AuthCredentialsStoreMode::File,
            AuthKeyringBackendKind::default(),
            "https://chatgpt.com/backend-api/".to_string(),
            /*auth_route_config*/ None,
        )
        .await,
        feedback: crate::feedback::ReflectFeedback::new(),
        log_db: None,
        state_db: None,
        environment_manager: Arc::new(
            crate::app_server_client::EnvironmentManager::default_for_tests(),
        ),
        config_warnings: Vec::new(),
        session_source: serde_json::from_value(serde_json::json!("cli"))
            .expect("cli session source should deserialize"),
        enable_reflect_api_key_env: false,
        client_name: "test".to_string(),
        client_version: "test".to_string(),
        experimental_api: true,
        mcp_server_openai_form_elicitation: false,
        opt_out_notification_methods: Vec::new(),
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
    .await
    .unwrap();
    let widget = AuthModeWidget {
        request_frame: FrameRequester::test_dummy(),
        highlighted_mode: SignInOption::ChatGpt,
        error: Arc::new(RwLock::new(None)),
        sign_in_state: Arc::new(RwLock::new(SignInState::PickMode)),
        login_status: LoginStatus::NotAuthenticated,
        app_server_request_handle: AppServerRequestHandle::InProcess(client.request_handle()),
        forced_login_method: Some(ForcedLoginMethod::Chatgpt),
        animations_enabled: true,
        animations_suppressed: std::cell::Cell::new(false),
    };
    (widget, reflect_home)
}

#[tokio::test]
async fn api_key_flow_disabled_when_reflect_forced() {
    let (mut widget, _tmp) = widget_forced_chatgpt().await;

    widget.start_api_key_entry();

    assert_eq!(
        widget.error_message().as_deref(),
        Some(API_KEY_DISABLED_MESSAGE)
    );
    assert!(matches!(
        &*widget.sign_in_state.read().unwrap(),
        SignInState::PickMode
    ));
}

#[tokio::test]
async fn saving_api_key_is_blocked_when_reflect_forced() {
    let (mut widget, _tmp) = widget_forced_chatgpt().await;

    widget.save_api_key("sk-test".to_string());

    assert_eq!(
        widget.error_message().as_deref(),
        Some(API_KEY_DISABLED_MESSAGE)
    );
    assert!(matches!(
        &*widget.sign_in_state.read().unwrap(),
        SignInState::PickMode
    ));
    assert_eq!(widget.login_status, LoginStatus::NotAuthenticated);
}

#[tokio::test]
async fn existing_non_oauth_reflect_login_counts_as_signed_in() {
    for auth_mode in [AuthMode::ChatgptAuthTokens, AuthMode::PersonalAccessToken] {
        let (mut widget, _tmp) = widget_forced_chatgpt().await;
        widget.login_status = LoginStatus::AuthMode(auth_mode);

        let handled = widget.handle_existing_reflect_login();

        assert_eq!(handled, true);
        assert!(matches!(
            &*widget.sign_in_state.read().unwrap(),
            SignInState::ChatGptSuccess
        ));
    }
}

#[tokio::test]
async fn cancel_active_attempt_resets_browser_login_state() {
    let (widget, _tmp) = widget_forced_chatgpt().await;
    *widget.error.write().unwrap() = Some("still logging in".to_string());
    *widget.sign_in_state.write().unwrap() =
        SignInState::ChatGptContinueInBrowser(ContinueInBrowserState {
            login_id: "login-1".to_string(),
            auth_url: "https://auth.example.com".to_string(),
        });

    widget.cancel_active_attempt();

    assert_eq!(widget.error_message(), None);
    assert!(matches!(
        &*widget.sign_in_state.read().unwrap(),
        SignInState::PickMode
    ));
}

#[tokio::test]
async fn cancel_active_attempt_notifies_device_code_login() {
    let (widget, _tmp) = widget_forced_chatgpt().await;
    *widget.error.write().unwrap() = Some("still logging in".to_string());
    *widget.sign_in_state.write().unwrap() =
        SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::ready(
            "request-1".to_string(),
            "login-1".to_string(),
            "https://chatgpt.com/device".to_string(),
            "ABCD-EFGH".to_string(),
        ));

    widget.cancel_active_attempt();

    assert_eq!(widget.error_message(), None);
    assert!(matches!(
        &*widget.sign_in_state.read().unwrap(),
        SignInState::PickMode
    ));
}

/// 收集包含给定 URL 的 OSC 8 开启序列的
/// 所有缓冲区单元格符号。返回拼接后的“内部”字符。
fn collect_osc8_chars(buf: &Buffer, area: Rect, url: &str) -> String {
    let open = format!("\x1B]8;;{url}\x07");
    let close = "\x1B]8;;\x07";
    let mut chars = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let sym = buf[(x, y)].symbol();
            if let Some(rest) = sym.strip_prefix(open.as_str())
                && let Some(ch) = rest.strip_suffix(close)
            {
                chars.push_str(ch);
            }
        }
    }
    chars
}

#[test]
fn continue_in_browser_renders_osc8_hyperlink() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
    let url = "https://auth.example.com/login?state=abc123";
    *widget.sign_in_state.write().unwrap() =
        SignInState::ChatGptContinueInBrowser(ContinueInBrowserState {
            login_id: "login-1".to_string(),
            auth_url: url.to_string(),
        });

    // 渲染进窄缓冲区，使 URL 跨多行折行。
    let area = Rect::new(0, 0, 30, 20);
    let mut buf = Buffer::empty(area);
    widget.render_continue_in_browser(area, &mut buf);

    // URL 的每个字符都应作为 OSC 8 单元格存在。
    let found = collect_osc8_chars(&buf, area, url);
    assert_eq!(found, url, "OSC 8 hyperlink should cover the full URL");
}

#[test]
fn auth_widget_suppresses_animations_when_device_code_is_visible() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
    *widget.sign_in_state.write().unwrap() =
        SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::ready(
            "request-1".to_string(),
            "login-1".to_string(),
            "https://chatgpt.com/device".to_string(),
            "ABCD-EFGH".to_string(),
        ));

    assert_eq!(widget.should_suppress_animations(), true);
}

#[test]
fn auth_widget_suppresses_animations_while_requesting_device_code() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (widget, _tmp) = runtime.block_on(widget_forced_chatgpt());
    *widget.sign_in_state.write().unwrap() = SignInState::ChatGptDeviceCode(
        ContinueWithDeviceCodeState::pending("request-1".to_string()),
    );

    assert_eq!(widget.should_suppress_animations(), true);
}

#[tokio::test]
async fn device_code_login_completion_advances_to_success_message() {
    let (mut widget, _tmp) = widget_forced_chatgpt().await;
    *widget.sign_in_state.write().unwrap() =
        SignInState::ChatGptDeviceCode(ContinueWithDeviceCodeState::ready(
            "request-1".to_string(),
            "login-1".to_string(),
            "https://chatgpt.com/device".to_string(),
            "ABCD-EFGH".to_string(),
        ));

    widget.on_account_login_completed(AccountLoginCompletedNotification {
        login_id: Some("login-1".to_string()),
        success: true,
        error: None,
    });

    assert!(matches!(
        &*widget.sign_in_state.read().unwrap(),
        SignInState::ChatGptSuccessMessage
    ));
}

#[test]
fn mark_url_hyperlink_wraps_cyan_underlined_cells() {
    let url = "https://example.com";
    let area = Rect::new(0, 0, 20, 1);
    let mut buf = Buffer::empty(area);

    // 手动写入一些青色+下划线字符，模拟已渲染的 URL。
    for (i, ch) in "example".chars().enumerate() {
        let cell = &mut buf[(i as u16, 0)];
        cell.set_symbol(&ch.to_string());
        cell.fg = Color::Cyan;
        cell.modifier = Modifier::UNDERLINED;
    }
    // 留下一个不应被标记的普通单元格。
    buf[(7, 0)].set_symbol("X");

    mark_url_hyperlink(&mut buf, area, url);

    // 每个青色+下划线单元格现在都应带有 OSC 8 包装。
    let found = collect_osc8_chars(&buf, area, url);
    assert_eq!(found, "example");

    // 普通 "X" 单元格应保持原样。
    assert_eq!(buf[(7, 0)].symbol(), "X");
}

#[test]
fn mark_url_hyperlink_sanitizes_control_chars() {
    let area = Rect::new(0, 0, 10, 1);
    let mut buf = Buffer::empty(area);

    // 标记一个青色+下划线单元格。
    let cell = &mut buf[(0, 0)];
    cell.set_symbol("a");
    cell.fg = Color::Cyan;
    cell.modifier = Modifier::UNDERLINED;

    // URL 含有可能破坏 OSC 8 序列的 ESC 和 BEL。
    let malicious_url = "https://evil.com/\x1B]8;;\x07injected";
    mark_url_hyperlink(&mut buf, area, malicious_url);

    let sym = buf[(0, 0)].symbol().to_string();
    // 净化后的 URL 保留 `]`（可打印），但去除 ESC 和 BEL。
    let sanitized = "https://evil.com/]8;;injected";
    assert!(
        sym.contains(sanitized),
        "symbol should contain sanitized URL, got: {sym:?}"
    );
    // 注入的关闭序列不应残留：\x1B 与 \x07 已被清除。
    assert!(
        !sym.contains("\x1B]8;;\x07injected"),
        "symbol must not contain raw control chars from URL"
    );
}
