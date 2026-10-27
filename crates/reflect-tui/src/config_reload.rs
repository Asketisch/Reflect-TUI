//! 配置热重载接线(v1.x `ConfigWatcher`)。
//!
//! 背景:submodule 的 `zhuye_config::ConfigWatcher` 会监听
//! `~/.zhuye/config.toml` 的变更并把新配置推给订阅者;headless
//! (`reflect-exec`)据此重建 provider client、切换 model、推
//! `EventMsg::ConfigReloaded`。TUI 此前**完全没有**这个 watcher ——
//! 改配置后必须重启 TUI 才生效,`[routing] fallbacks`、`[model]` 之类
//! 的改动统统不认。
//!
//! 本模块负责:
//! - `diff_sections` —— 纯函数,算出本轮变更了哪些配置段;
//! - `spawn_config_reload_task` —— 后台任务:应用新 provider client →
//!   必要时切 model → 推一条 Notice 回界面。

use std::sync::Arc;

use zhuye_config::ReflectConfig;
use zhuye_core::AgentConfig;
use zhuye_llm::ModelRegistry;

/// 算出两份配置之间变更了哪些段(段名列表)。
///
/// 段名与 `reflect-exec` 的 `diff_sections` 对齐,便于两边行为一致。
pub fn diff_sections(old: &ReflectConfig, new: &ReflectConfig) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    macro_rules! cmp_section {
        ($field:ident, $name:literal) => {
            if old.$field != new.$field {
                out.push($name.to_string());
            }
        };
    }
    cmp_section!(active, "active");
    cmp_section!(anthropic, "anthropic");
    cmp_section!(openai, "openai");
    cmp_section!(ollama, "ollama");
    cmp_section!(compact, "compact");
    cmp_section!(token_budget, "token_budget");
    cmp_section!(sandbox, "sandbox");
    cmp_section!(hooks, "hooks");
    cmp_section!(mcp_servers, "mcp_servers");
    cmp_section!(lsp_servers, "lsp_servers");
    cmp_section!(plugins, "plugins");
    cmp_section!(routing, "routing");
    cmp_section!(subagent_providers, "subagent_providers");
    cmp_section!(coordinator, "coordinator");
    cmp_section!(sanitize, "sanitize");
    cmp_section!(model, "model");
    cmp_section!(model_discovery, "model_discovery");
    cmp_section!(context_windows, "context_windows");
    cmp_section!(goal, "goal");
    cmp_section!(telemetry, "telemetry");
    cmp_section!(analytics, "analytics");
    cmp_section!(notifications, "notifications");
    cmp_section!(web_search, "web_search");
    cmp_section!(permissions, "permissions");
    out
}

/// 启动配置热重载后台任务。
///
/// 每次配置变更:
/// 1. `apply_to_registry` 替换 provider client(失败 → 保留旧 provider,
///    只 warn,不推事件 —— 与 headless 同款语义);
/// 2. model spec 变了 → `agent_cfg.set_model(...)` 同步给运行中的 agent;
/// 3. 推一条 Notice 回界面,告诉用户「配置已重载,哪些段变了」。
///
/// `ui_tx` 是 TUI 的 `UiEvent` 通道 —— 直接推 `UiEventKind::Notice`,
/// 免得绕协议事件再转一次。
pub fn spawn_config_reload_task(
    watcher: zhuye_config::ConfigWatcher,
    registry: Arc<ModelRegistry>,
    agent_cfg: AgentConfig,
    path: std::path::PathBuf,
    previous_cfg: ReflectConfig,
    ui_tx: tokio::sync::mpsc::UnboundedSender<crate::adapter::UiEvent>,
) {
    tokio::spawn(async move {
        let mut rx = watcher.subscribe();
        // 不要在循环前预调 `rx.changed()`「跳过初始值」—— watch::Receiver
        // 新订阅时已把当前值视为已见,那次预调会吞掉启动后第一次真实变更。
        let mut prev = previous_cfg;
        while rx.changed().await.is_ok() {
            let new_cfg = rx.borrow().clone();
            match handle_reload(&prev, &new_cfg, &registry, &agent_cfg, &path, &ui_tx) {
                Ok(()) => {}
                Err(e) => tracing::warn!(error = %e, "config reload failed"),
            }
            prev = new_cfg;
        }
        tracing::debug!("config reload task exiting");
    });
}

/// 应用一次配置变更。返回 `Ok(())` 表示已生效并已推事件。
fn handle_reload(
    old_cfg: &ReflectConfig,
    new_cfg: &ReflectConfig,
    registry: &Arc<ModelRegistry>,
    agent_cfg: &AgentConfig,
    path: &std::path::Path,
    ui_tx: &tokio::sync::mpsc::UnboundedSender<crate::adapter::UiEvent>,
) -> Result<(), String> {
    // provider client 重建失败 → 保留旧 provider,不推事件(与 headless 一致)。
    new_cfg
        .apply_to_registry(registry)
        .map_err(|e| format!("apply_to_registry failed: {e}"))?;
    let sections_changed = diff_sections(old_cfg, new_cfg);
    // model spec 变了 → 同步给运行中的 agent(下一轮生效)。
    if let Some(spec) = new_cfg.resolved_model_spec()
        && old_cfg.resolved_model_spec().as_deref() != Some(spec.as_str())
    {
        agent_cfg.set_model(spec.clone());
        tracing::info!(model = %spec, "config reload: model switched");
    }
    tracing::info!(path = %path.display(), "config reload applied");
    // 推给界面(通道关闭时忽略 —— TUI 已退出)。事件携带新配置,消费方
    // 刷新 `state.zhuye_config` 并渲染变更提示。
    let _ = ui_tx.send(crate::adapter::UiEvent {
        kind: crate::adapter::UiEventKind::ConfigReloaded {
            config: Box::new(new_cfg.clone()),
            sections_changed,
        },
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_model(model: &str) -> ReflectConfig {
        ReflectConfig {
            active: zhuye_config::ActiveSection {
                provider: Some("openai".into()),
                ..Default::default()
            },
            openai: Some(zhuye_config::OpenAISection {
                api_key: Some("k".into()),
                model: Some(model.into()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn no_change_yields_empty_diff() {
        let a = cfg_with_model("gpt-4o");
        let b = cfg_with_model("gpt-4o");
        assert!(diff_sections(&a, &b).is_empty());
    }

    #[test]
    fn model_change_is_reported() {
        let a = cfg_with_model("gpt-4o");
        let b = cfg_with_model("gpt-4o-mini");
        let d = diff_sections(&a, &b);
        // model 挂在 provider 段(`[<provider>].model`),`[active]` 段只有
        // provider / credential / max_iterations —— provider 没变则 active 不变。
        assert!(d.iter().any(|s| s == "openai"), "{d:?}");
        assert!(!d.iter().any(|s| s == "active"), "{d:?}");
    }

    #[test]
    fn routing_change_is_reported() {
        let a = ReflectConfig::default();
        let mut b = ReflectConfig::default();
        b.routing = Some(zhuye_config::RoutingSection::default());
        let d = diff_sections(&a, &b);
        assert!(d.iter().any(|s| s == "routing"), "{d:?}");
    }

    #[test]
    fn model_discovery_change_is_reported() {
        let a = ReflectConfig::default();
        let mut b = ReflectConfig::default();
        b.model_discovery = Some(zhuye_config::ModelDiscoverySection::default());
        let d = diff_sections(&a, &b);
        assert!(d.iter().any(|s| s == "model_discovery"), "{d:?}");
    }
}
