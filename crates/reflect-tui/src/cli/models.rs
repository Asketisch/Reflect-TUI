//! `reflect-tui models` —— 列出 / 挑选 provider 当前可用的模型。
//!
//! ## 为什么要有这个命令
//!
//! 免费模型池的成员集合由**上游**决定,随时可能增删 —— 模型 id 写死在
//! 配置里必然腐烂。本命令把「有哪些模型可用」变成**从接口实时获取**的事实:
//!
//! ```bash
//! reflect-tui models list --free-only          # 人看
//! reflect-tui models pick --free-only          # 脚本用:只打印一个 model id
//! ```
//!
//! 「免费」的判定走规则(见 `zhuye_llm::FreeModelRules`):provider 显式
//! 标记 → 白名单 → id 模式。**判定规则不硬编码模型清单** —— 模型本身
//! 永远来自 provider 的清单端点。
//!
//! 实现与 submodule 的 `reflect models` 同源(`reflect-agent/crates/
//! runtime/reflect-cli/src/models.rs`),但只保留 TUI 需要的两个动作
//! (`list` / `pick`):本仓是表现层,不提供 headless 推理入口。

use std::sync::Arc;

use zhuye_config::{ReflectConfig, fetch_catalog, pick_free_model};
use zhuye_llm::ModelRegistry;

/// `reflect-tui models` 子命令的退出码约定。
///
/// 0 = 成功;2 = provider 不支持模型清单 / 未配置;3 = 没有匹配的模型。
pub const EXIT_NO_LISTING: i32 = 2;
pub const EXIT_NO_MATCH: i32 = 3;

/// 表格行:(模型 id, 是否免费, 上下文窗口, 归属方)。
///
/// 用别名拆开嵌套泛型(`clippy::type_complexity`)。
type ModelRow = (String, Option<bool>, Option<u32>, Option<String>);

/// `reflect-tui models list` 的参数。
#[derive(Debug, Clone)]
pub struct ListArgs {
    /// provider 名(默认:配置里的 active provider)。
    pub provider: Option<String>,
    /// 只列免费模型。
    pub free_only: bool,
    /// 以 JSON 输出(供脚本消费)。
    pub json: bool,
    /// 清单接口超时(秒)。
    pub timeout_secs: Option<u64>,
}

/// `reflect-tui models pick` 的参数。
#[derive(Debug, Clone)]
pub struct PickArgs {
    /// provider 名(默认:配置里的 active provider)。
    pub provider: Option<String>,
    /// 只在免费模型里挑。
    pub free_only: bool,
    /// id 子串偏好(优先挑含该子串的模型)。
    pub prefer: Option<String>,
    /// 清单接口超时(秒)。
    pub timeout_secs: Option<u64>,
}

/// 加载配置并注册 provider(与其他子命令一致的口径)。
fn load_registry() -> anyhow::Result<(ReflectConfig, Arc<ModelRegistry>)> {
    let cfg = zhuye_config::load_default();
    let registry = Arc::new(ModelRegistry::new());
    cfg.apply_to_registry(&registry)
        .map_err(|e| anyhow::anyhow!("failed to build provider from config: {e}"))?;
    Ok((cfg, registry))
}

/// 解析目标 provider:显式参数 > 配置里的 active provider。
fn target_provider(cfg: &ReflectConfig, explicit: Option<&str>) -> anyhow::Result<String> {
    if let Some(p) = explicit.map(str::trim).filter(|p| !p.is_empty()) {
        return Ok(p.to_string());
    }
    cfg.active_provider().map(str::to_string).ok_or_else(|| {
        anyhow::anyhow!("no active provider: set [active] provider in config, or pass --provider")
    })
}

/// 清单接口超时:显式参数 > 配置的 `[model_discovery].timeout_secs` > 内置默认。
fn resolve_timeout(cfg: &ReflectConfig, explicit: Option<u64>) -> u64 {
    explicit.unwrap_or(
        cfg.model_discovery
            .as_ref()
            .and_then(|d| d.timeout_secs)
            .unwrap_or(zhuye_config::DEFAULT_DISCOVERY_TIMEOUT_SECS),
    )
}

/// 列出模型清单。
pub async fn list(args: ListArgs) -> anyhow::Result<i32> {
    let (cfg, registry) = load_registry()?;
    let provider = target_provider(&cfg, args.provider.as_deref())?;
    let timeout_secs = resolve_timeout(&cfg, args.timeout_secs);

    let catalog = match fetch_catalog(&registry, &provider, timeout_secs).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot fetch model list for provider '{provider}': {e}");
            return Ok(EXIT_NO_LISTING);
        }
    };

    let rules = cfg
        .model_discovery
        .as_ref()
        .map(|d| d.rules())
        .unwrap_or_default();

    // 过滤:只要免费的 → 用规则判定;全要 → 全列。
    let rows: Vec<ModelRow> = catalog
        .models
        .iter()
        .map(|m| {
            (
                m.id.clone(),
                rules.classify(m),
                m.context_window,
                m.owned_by.clone(),
            )
        })
        .filter(|(_, free, _, _)| !args.free_only || *free == Some(true))
        .collect();

    if args.json {
        let items: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, free, ctx, owner)| {
                serde_json::json!({
                    "id": id,
                    "free": free,
                    "context_window": ctx,
                    "owned_by": owner,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "provider": provider,
                "total": catalog.models.len(),
                "matched": items.len(),
                "models": items,
            }))
            .unwrap_or_default()
        );
        return Ok(0);
    }

    if rows.is_empty() {
        println!("no models matched for provider '{provider}'");
        return Ok(EXIT_NO_MATCH);
    }

    println!("provider: {provider}");
    println!(
        "total:    {} model(s), matched: {}",
        catalog.models.len(),
        rows.len()
    );
    println!();
    println!("{:<52} {:<6} {:>10}  OWNER", "ID", "FREE", "CONTEXT");
    for (id, free, ctx, owner) in &rows {
        let free_cell = match free {
            Some(true) => "yes",
            Some(false) => "no",
            None => "?",
        };
        println!(
            "{:<52} {:<6} {:>10}  {}",
            id,
            free_cell,
            ctx.map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string()),
            owner.as_deref().unwrap_or("-")
        );
    }
    Ok(0)
}

/// 挑一个模型并**只打印它的 id**(供 `$(...)` 消费)。
pub async fn pick(args: PickArgs) -> anyhow::Result<i32> {
    let (cfg, registry) = load_registry()?;
    let provider = target_provider(&cfg, args.provider.as_deref())?;
    let timeout_secs = resolve_timeout(&cfg, args.timeout_secs);

    let catalog = match fetch_catalog(&registry, &provider, timeout_secs).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot fetch model list for provider '{provider}': {e}");
            return Ok(EXIT_NO_LISTING);
        }
    };

    let picked = if args.free_only {
        // 复用配置里的规则段(`always_free` / `free_id_patterns`)。
        let section = cfg.model_discovery.clone().unwrap_or_default();
        let section = zhuye_config::ModelDiscoverySection {
            prefer: args.prefer.clone().or(section.prefer),
            ..section
        };
        pick_free_model(&catalog, &section)
    } else {
        // 不限免费 → 按 id 字典序取第一个(确定性)。
        let mut sorted = catalog.models.clone();
        sorted.sort_by(|a, b| a.id.cmp(&b.id));
        match sorted.into_iter().next() {
            Some(info) => Ok(info),
            // 清单为空 → 与「无匹配」同义。
            None => Err(zhuye_config::ConfigError::Build(format!(
                "provider '{}' 的模型清单为空",
                provider
            ))),
        }
    };

    match picked {
        Ok(info) => {
            println!("{}", info.id);
            Ok(0)
        }
        Err(e) => {
            eprintln!("{e}");
            Ok(EXIT_NO_MATCH)
        }
    }
}
