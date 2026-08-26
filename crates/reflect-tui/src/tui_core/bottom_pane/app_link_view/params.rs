//! app link 视图参数结构与方法。从 app_link_view.rs 抽出。

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppLinkViewParams {
    pub(crate) app_id: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) instructions: String,
    pub(crate) url: String,
    pub(crate) is_installed: bool,
    pub(crate) is_enabled: bool,
    pub(crate) suggest_reason: Option<String>,
    pub(crate) suggestion_type: Option<AppLinkSuggestionType>,
    pub(crate) elicitation_target: Option<AppLinkElicitationTarget>,
}

impl AppLinkViewParams {
    pub(crate) fn from_url_app_server_request(
        thread_id: ThreadId,
        server_name: &str,
        request_id: AppServerRequestId,
        request: &crate::app_server_protocol::McpServerElicitationRequest,
    ) -> Option<Self> {
        let crate::app_server_protocol::McpServerElicitationRequest::Url {
            meta,
            message,
            url,
            elicitation_id,
        } = request
        else {
            return None;
        };
        if server_name == MCP_REFLECT_APPS_SERVER_NAME {
            let url = validate_external_url(url, /*require_chatgpt_host*/ true)?;
            return Self::from_reflect_apps_auth_url_parts(
                thread_id,
                server_name,
                request_id,
                meta.as_ref(),
                message,
                url.as_str(),
                elicitation_id,
            );
        }

        let url = validate_external_url(url, /*require_chatgpt_host*/ false)?;
        Some(Self::from_generic_url_parts(
            thread_id,
            server_name,
            request_id,
            message,
            url.as_str(),
            elicitation_id,
        ))
    }

    fn from_reflect_apps_auth_url_parts(
        thread_id: ThreadId,
        server_name: &str,
        request_id: AppServerRequestId,
        meta: Option<&serde_json::Value>,
        message: &str,
        url: &str,
        elicitation_id: &str,
    ) -> Option<Self> {
        let auth_failure = meta?
            .as_object()?
            .get(MCP_TOOL_REFLECT_APPS_META_KEY)?
            .as_object()?
            .get(CONNECTOR_AUTH_FAILURE_META_KEY)?
            .as_object()?;
        if auth_failure
            .get(CONNECTOR_AUTH_FAILURE_IS_AUTH_FAILURE_KEY)
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        {
            return None;
        }

        let app_id = auth_failure
            .get(CONNECTOR_AUTH_FAILURE_CONNECTOR_ID_KEY)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(elicitation_id)
            .to_string();
        let title = auth_failure
            .get(CONNECTOR_AUTH_FAILURE_CONNECTOR_NAME_KEY)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(app_id.as_str())
            .to_string();

        Some(Self {
            app_id,
            title,
            description: None,
            instructions: "Sign in to this app in your browser, then return here.".to_string(),
            url: url.to_string(),
            is_installed: true,
            is_enabled: true,
            suggest_reason: Some(message.to_string()),
            suggestion_type: Some(AppLinkSuggestionType::Auth),
            elicitation_target: Some(AppLinkElicitationTarget {
                thread_id,
                server_name: server_name.to_string(),
                request_id,
            }),
        })
    }

    fn from_generic_url_parts(
        thread_id: ThreadId,
        server_name: &str,
        request_id: AppServerRequestId,
        message: &str,
        url: &str,
        elicitation_id: &str,
    ) -> Self {
        Self {
            app_id: elicitation_id.to_string(),
            title: "Action required".to_string(),
            description: Some(format!("Server: {server_name}")),
            instructions: "Complete the requested action in your browser, then return here."
                .to_string(),
            url: url.to_string(),
            is_installed: true,
            is_enabled: true,
            suggest_reason: Some(message.to_string()),
            suggestion_type: Some(AppLinkSuggestionType::ExternalAction),
            elicitation_target: Some(AppLinkElicitationTarget {
                thread_id,
                server_name: server_name.to_string(),
                request_id,
            }),
        }
    }
}
