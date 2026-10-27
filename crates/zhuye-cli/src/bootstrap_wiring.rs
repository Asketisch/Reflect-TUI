//! TUI 侧运行时装配:对齐 headless(`zhuye-exec`)的 M4/M5/M6/LSP 能力。
//!
//! 不能直接复用 `zhuye-exec` 的 `bootstrap_m4` —— 它依赖该 bin crate 的
//! thread-local `TOOLS` 句柄。本模块按同一逻辑移植为纯参数式函数,直接调用
//! 相同的 submodule 库 crate,让 TUI 进程获得与无头模式一致的能力面:
//! agent 定义、skills、session notes、post-compact 恢复、LLM 摘要压缩、
//! coordinator、MCP、LSP、telemetry、quota、sanitizer。

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use zhuye_agent_def::AgentDefinition;
use zhuye_compact::{Compactor, LlmSummarizer, Summarizer, SummarizerError};
use zhuye_llm::{ChatMessage, SharedModelRegistry};
use zhuye_lsp::{LspConnectionManager, LspLifecycleEvent, LspTool};
use zhuye_mcp::{McpConnectionManager, McpLifecycleEvent, McpServerConfig, McpToolAdapter};
use zhuye_memory::{InMemoryStore, MemoryStore};
use zhuye_prompt::PromptBuilder;
use zhuye_protocol::{EVENT_ID_NONE, Event, ThreadId};
use zhuye_skills::SkillsCatalog;
use zhuye_subagent::SubAgentFactory;
use zhuye_task::coordinator::{CoordinatorConfig, build_scratchpad_path, ensure_scratchpad};
use zhuye_tools::{SanitizeConfig, Sanitizer, ToolRegistry, ToolSource};

// v1.6 起 `DEFAULT_SYSTEM_PROMPT` 本地副本删除:默认 agent 不再携带固定
// 文案,`SystemPromptSource::BuiltinDefault` 标记让 pre_loop 按当前模型
// spec 从提示词库(`zhuye_prompt::library()`)解析 `system.main`,
// 与 headless(`zhuye-exec` bootstrap_m4)同款。

// ── 无 LLM 客户端时的 fallback 摘要器 ──────────────────────────────────

/// 始终报错,让 compactor 退回 smart_prune(与 exec 的 `NoopSummarizer` 同语义)。
pub(crate) struct NoopSummarizer;

#[async_trait::async_trait]
impl Summarizer for NoopSummarizer {
    async fn summarize_full(&self, _: &[ChatMessage]) -> Result<String, SummarizerError> {
        Err(SummarizerError::Cancelled)
    }
    async fn summarize_recent(
        &self,
        _: &[ChatMessage],
        _: Option<&str>,
    ) -> Result<String, SummarizerError> {
        Err(SummarizerError::Cancelled)
    }
}

// ── 配置整形构建器(移植自 exec/runtime_config.rs)───────────────────────

/// 把 `~/.zhuye/config.toml [sanitize]` 段接到 queue 的脱敏 pass。
/// 用户写错 `extra_patterns[i]` 时 warn 后 fallback 默认 10-pattern,不阻塞启动。
pub(crate) fn build_sanitizer(section: Option<&zhuye_config::SanitizeSection>) -> Arc<Sanitizer> {
    let cfg = SanitizeConfig {
        enabled: section.and_then(|s| s.enabled),
        marker: section.and_then(|s| s.marker.clone()),
        disable_default_patterns: section.and_then(|s| s.disable_default_patterns),
        extra_patterns: section.and_then(|s| s.extra_patterns.clone()),
    };
    match Sanitizer::from_config(&cfg) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::warn!(error = %e, "[sanitize] 配置解析失败,fallback 到默认 10-pattern");
            Arc::new(Sanitizer::with_defaults())
        }
    }
}

/// 从 `[telemetry]` 配置构造本地 trace sink(P2 `langfuse` 云端并行投递)。
/// `enabled = false`(显式)→ `None`;缺省 → 启用,目录默认 `~/.zhuye/traces`。
pub(crate) fn build_telemetry_sink(
    cfg: &zhuye_config::ReflectConfig,
) -> Option<Arc<zhuye_telemetry::TelemetrySink>> {
    let default_section = zhuye_config::TelemetrySection::default();
    let section = cfg.telemetry.as_ref().unwrap_or(&default_section);
    if !section.is_enabled() {
        return None;
    }
    let base_dir = section
        .dir
        .clone()
        .unwrap_or_else(|| zhuye_telemetry::resolve_traces_dir(None));
    let session_id = uuid::Uuid::new_v4().to_string();
    // v1.x:`redact` / `max_bytes` / `record_tool_schemas` 三个配置项此前在
    // schema 里定义了但从未被读取 —— 现在经 `new_with_config` 真正生效
    // (与 headless `zhuye-exec` 的 `runtime_config::build_telemetry_sink`
    // 同款逻辑,避免 TUI 与 headless 行为漂移)。
    let write_cfg = zhuye_telemetry::TelemetryWriteConfig {
        redact: section.should_redact(),
        max_bytes: section
            .max_bytes
            .map(|n| n as usize)
            .unwrap_or(zhuye_telemetry::model::MAX_FIELD_CHARS),
    };
    let sink = zhuye_telemetry::TelemetrySink::new_with_config(
        base_dir,
        session_id,
        write_cfg,
        section.should_record_tool_schemas(),
    );

    if let Some(lf) = cfg.hooks.langfuse_tracker.as_ref() {
        if lf.enabled.unwrap_or(false) {
            if let (Some(endpoint), Some(pk), Some(sk)) = (
                lf.endpoint.as_ref(),
                lf.public_key.as_ref(),
                lf.secret_key.as_ref(),
            ) {
                if !endpoint.is_empty() && !pk.is_empty() && !sk.is_empty() {
                    let lf_cfg = zhuye_telemetry::LangfuseConfig {
                        endpoint: endpoint.clone(),
                        public_key: pk.clone(),
                        secret_key: sk.clone(),
                        batch_size: 64,
                    };
                    let exporter = zhuye_telemetry::LangfuseExporter::start(Some(lf_cfg));
                    sink.set_langfuse(exporter);
                    tracing::info!(endpoint = %endpoint, "langfuse cloud exporter 已启用");
                }
            }
        }
    }

    Some(sink)
}

/// 遍历 config 所有 provider 的 credentials,收集声明了 `quota` 的条目,
/// 注册到 `QuotaTracker`。任一 provider 有 quota 声明 → `Some(tracker)`;
/// 全无 → `None`(向后兼容)。
pub(crate) fn build_quota_tracker(
    cfg: &zhuye_config::ReflectConfig,
) -> Option<zhuye_llm::SharedQuotaTracker> {
    use std::sync::Arc as StdArc;
    use zhuye_config::QuotaSource;
    use zhuye_llm::{
        KimiQuotaProvider, MinimaxQuotaProvider, ZenmuxQuotaProvider, ZhipuQuotaProvider,
    };

    let tracker = Arc::new(zhuye_llm::QuotaTracker::new());
    let mut any = false;

    fn make_provider(src: &QuotaSource) -> Option<Arc<dyn zhuye_llm::QuotaProvider>> {
        match src {
            QuotaSource::Kimi => {
                Some(StdArc::new(KimiQuotaProvider) as Arc<dyn zhuye_llm::QuotaProvider>)
            }
            QuotaSource::Zhipu => {
                Some(StdArc::new(ZhipuQuotaProvider) as Arc<dyn zhuye_llm::QuotaProvider>)
            }
            QuotaSource::Minimax => {
                Some(StdArc::new(MinimaxQuotaProvider) as Arc<dyn zhuye_llm::QuotaProvider>)
            }
            QuotaSource::Zenmux => {
                Some(StdArc::new(ZenmuxQuotaProvider) as Arc<dyn zhuye_llm::QuotaProvider>)
            }
            QuotaSource::Volcengine | QuotaSource::AnthropicUsage | QuotaSource::OpenAIUsage => {
                None
            }
        }
    }

    let mut register_provider = |provider: &str, creds: &[zhuye_config::CredentialConfig]| {
        for c in creds {
            if let Some(q) = &c.quota {
                let rt_source = q.check_via.as_ref().map(|s| match s {
                    QuotaSource::Kimi => zhuye_llm::QuotaSource::Kimi,
                    QuotaSource::Zhipu => zhuye_llm::QuotaSource::Zhipu,
                    QuotaSource::Minimax => zhuye_llm::QuotaSource::Minimax,
                    QuotaSource::Zenmux => zhuye_llm::QuotaSource::Zenmux,
                    QuotaSource::Volcengine => zhuye_llm::QuotaSource::Volcengine,
                    QuotaSource::AnthropicUsage => zhuye_llm::QuotaSource::AnthropicUsage,
                    QuotaSource::OpenAIUsage => zhuye_llm::QuotaSource::OpenAIUsage,
                });
                let rt = zhuye_llm::QuotaConfig {
                    window_secs: q.window_secs,
                    max_tokens: q.max_tokens,
                    check_via: rt_source,
                };
                tracker.register(provider, &c.label, rt);
                let base_url = c
                    .base_url
                    .clone()
                    .unwrap_or_else(|| format!("https://{provider}"));
                tracker.register_credential(provider, &c.label, &base_url, &c.api_key);
                if let Some(src) = &q.check_via {
                    if let Some(p) = make_provider(src) {
                        tracker.register_provider(provider, &c.label, p);
                    }
                }
                tracing::info!(
                    provider,
                    label = %c.label,
                    window_secs = q.window_secs,
                    max_tokens = q.max_tokens,
                    has_api = q.check_via.is_some(),
                    "registered token plan quota for credential"
                );
                any = true;
            }
        }
    };
    if let Some(s) = &cfg.anthropic {
        register_provider("anthropic", &s.credentials);
    }
    if let Some(s) = &cfg.openai {
        register_provider("openai", &s.credentials);
    }
    if let Some(s) = &cfg.ollama {
        register_provider("ollama", &s.credentials);
    }
    if any { Some(tracker) } else { None }
}

// ── M4:agent 定义 / skills / notes / recovery / 压缩器 ─────────────────

/// `$ZHUYE_HOME/session-notes/<thread_id>.jsonl` 上的 note store。
/// 任何一步失败返回 `None`,caller fallback 纯内存 store,不阻塞启动。
fn build_note_store(thread_id: ThreadId) -> Option<Arc<dyn zhuye_notes::NoteStore>> {
    let home = zhuye_notes::resolve_notes_home()?;
    let dir = home.join("session-notes");
    let path = dir.join(format!("{}.jsonl", thread_id));
    match zhuye_notes::FileBackedNoteStore::open_or_create_default(&path) {
        Ok(s) => {
            tracing::info!(
                thread_id = %thread_id,
                path = %path.display(),
                "session notes 落盘已启用"
            );
            Some(Arc::new(s))
        }
        Err(e) => {
            tracing::warn!(
                thread_id = %thread_id,
                path = %path.display(),
                error = %e,
                "session notes JSONL 初始化失败;fallback 纯 RAM"
            );
            None
        }
    }
}

/// 为当前 session 构造 M4 依赖项:agent 定义、skills、压缩器(LLM 摘要)、
/// 记忆、prompt 构建器、session notes、post-compact 恢复、子代理调用注册表。
///
/// 与 exec 的 `bootstrap_m4` 同逻辑;差异仅两点:`recorder` 由 caller 注入
/// (TUI 在更早处创建,需与 SubAgentFactory 共享同一 Arc),`tools` 走参数
/// (exec 用 bin crate 的 thread_local 句柄)。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_m4(
    workspace: &Path,
    agent_name: &str,
    model: &str,
    registry: &SharedModelRegistry,
    thread_id: ThreadId,
    toml_trigger_tokens: Option<u32>,
    cfg: &zhuye_config::ReflectConfig,
    telemetry: Option<Arc<zhuye_telemetry::TelemetrySink>>,
    recorder: Arc<dyn zhuye_protocol::RolloutRecorder>,
    tools: &Arc<ToolRegistry>,
) -> zhuye_core::config::M4Deps {
    // Agent 定义:workspace + home 两个目录合并,同名时 workspace 覆盖 home。
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = std::path::Path::new(&home);

    // v1.6:按模型匹配的外置提示词库(对齐 exec bootstrap_m4)。home 在前、
    // workspace 在后(workspace 同名层优先),另兼容旧版 `~/.zhuye/prompts.md`
    // 降级为 default 层。进程级全局句柄,pre_loop 每轮 resolve。
    let ws_prompts = workspace.join(".zhuye").join("prompts");
    let home_prompts = home_path.join(".zhuye").join("prompts");
    let legacy_prompts_md = home_path.join(".zhuye").join("prompts.md");
    let legacy = std::fs::read_to_string(&legacy_prompts_md).ok();
    if legacy.is_some() {
        tracing::warn!(
            path = %legacy_prompts_md.display(),
            "~/.zhuye/prompts.md is deprecated; migrate to ~/.zhuye/prompts/default.md"
        );
    }
    zhuye_prompt::init_library(&[&home_prompts, &ws_prompts], legacy.as_deref());

    let ws_agents = workspace.join(".zhuye").join("agents");
    let home_agents = home_path.join(".zhuye").join("agents");
    let mut agents = zhuye_agent_def::load_agents_dir(&ws_agents).unwrap_or_default();
    agents.extend(zhuye_agent_def::load_agents_dir(&home_agents).unwrap_or_default());
    #[allow(clippy::field_reassign_with_default)] // 与 exec 同款,耐字段增减
    let active_def = agents.remove(agent_name).unwrap_or_else(|| {
        // v1.6:默认 agent 不再携带固定文案 —— `BuiltinDefault` 来源标记让
        // pre_loop 按当前模型 spec 从提示词库解析 `system.main`(用户自定义
        // `.zhuye/agents/<name>.md` 的 `Custom` 来源则原文不动)。
        let mut d = AgentDefinition::default();
        d.name = agent_name.to_string();
        d.description = "default agent".into();
        d.system_prompt = String::new();
        d.source = zhuye_agent_def::SystemPromptSource::BuiltinDefault;
        d
    });
    let active_def = Arc::new(active_def);

    // Skills:扫描 workspace + home skill 目录,合并编译期内置技能包
    // (code-review / commit-helper 等)。同名 skill 以文件目录扫描结果优先。
    let ws_skills = workspace.join(".zhuye").join("skills");
    let home_skills = home_path.join(".zhuye").join("skills");
    let skill_dirs: Vec<&std::path::Path> = vec![&ws_skills, &home_skills];
    let skills_catalog = SkillsCatalog::new();
    skills_catalog.scan(&skill_dirs);
    zhuye_skills::merge_bundled(&skills_catalog);
    let skills_catalog = Arc::new(skills_catalog);

    // 注册 `load_skill` + `read_skill_resource`(渐进披露第三级)。
    // 走 `register_runtime_tool` 应用 Runtime 源的 `Prompt` 安全 floor。
    let load_skill = Arc::new(zhuye_skills::LoadSkillTool::new(skills_catalog.clone()));
    tools.register_runtime_tool(load_skill);
    let read_skill_resource = Arc::new(zhuye_skills::ReadSkillResourceTool::new(
        skills_catalog.clone(),
    ));
    tools.register_runtime_tool(read_skill_resource);

    // Memory:project + user 走文件,session scope 走内存(进程退出即丢,
    // 对话历史已在 rollout JSONL)。
    let file_store = Arc::new(zhuye_memory::FileMemoryStore::new(workspace, home_path));
    let memory: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::with_fallback(file_store));

    // Session notes:FIFO 30 条 + JSONL 落盘,工具写入与 pre_loop 注入读同一 Arc。
    let note_store: Arc<dyn zhuye_notes::NoteStore> = match build_note_store(thread_id) {
        Some(store) => store,
        None => Arc::new(zhuye_notes::InMemoryNoteStore::new()),
    };
    let note_tool: Arc<dyn zhuye_tools::Tool> =
        Arc::new(zhuye_notes::AddSessionNoteTool::new(note_store.clone()));
    tools.register_runtime_tool(note_tool);

    // post-compact 活跃文件恢复 + 已完成子代理调用注册表(防 LLM 重复 spawn)。
    let file_recovery = Arc::new(zhuye_recovery::ActiveFileRecovery::new(Arc::from(
        workspace.to_path_buf(),
    )));
    let subagent_registry = zhuye_recovery::SubagentRegistry::shared();

    // 压缩器:接 LLM 摘要器(Router::Compact slot,失败跨 credential failover),
    // 无可用客户端时退回 Noop(compactor 降级 smart_prune)。
    let summarizer: Arc<dyn Summarizer> = {
        // v1.5:用**已解析**的 spec 构建 routing(compact slot 的 primary
        // 必须等于真实模型)。`routing_policy()` 走同步解析,在
        // `model = "free"` 场景下拿到的是占位串,会盖掉真实 id。
        let policy = cfg.routing_policy_with(model);
        let spec = {
            let p = policy.resolve(zhuye_llm::Role::Compact).primary.clone();
            if p.is_empty() { model.to_string() } else { p }
        };
        if registry.next_for(&spec, &[]).is_some() {
            Arc::new(
                LlmSummarizer::new(registry.clone(), Arc::new(policy), spec)
                    .with_telemetry(telemetry.clone()),
            )
        } else {
            tracing::info!("compact slot 无可用模型,LLM 摘要压缩不可用(smart_prune 仍生效)");
            Arc::new(NoopSummarizer)
        }
    };
    // 触发阈值:env `ZHUYE_AUTO_COMPACT_INPUT_TOKENS` > TOML > 默认值。
    let compactor_cfg = zhuye_core::config::compactor_config_from_env_and_toml(toml_trigger_tokens);
    // v1.4 D2 记忆固化门控(ZHUYE_MEMORY_CONSOLIDATION):默认关闭;
    // LLM 摘要器在位时才接线(Noop 提取不出内容)。
    let consolidation_scope = match std::env::var("ZHUYE_MEMORY_CONSOLIDATION")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "session" => Some(zhuye_memory::MemoryScope::Session),
        "project" => Some(zhuye_memory::MemoryScope::Project),
        "user" => Some(zhuye_memory::MemoryScope::User),
        _ => None,
    };
    let compactor_base = Compactor::new(compactor_cfg, summarizer);
    let compactor = match consolidation_scope {
        Some(scope) if registry.next_for(model, &[]).is_some() => {
            tracing::info!(?scope, "memory consolidation enabled");
            Arc::new(compactor_base.with_memory_consolidation(
                memory.clone(),
                scope,
                agent_name.to_string(),
            ))
        }
        _ => Arc::new(compactor_base),
    };

    // Prompt 构建器:coordinator 启用时注入命名 section(build_request 拼到 core 末尾)。
    let prompt_builder = Arc::new(Mutex::new(PromptBuilder::new()));
    let coord_cfg = CoordinatorConfig::from_env_or_config(
        cfg.coordinator
            .as_ref()
            .unwrap_or(&zhuye_config::CoordinatorSection::default()),
    );
    if coord_cfg.enabled {
        prompt_builder
            .lock()
            .upsert_section("Coordinator", coord_cfg.system_prompt.clone());
        tracing::info!(
            max_workers = coord_cfg.max_workers,
            "coordinator mode enabled; system prompt 注入到 prompt_builder"
        );
    }

    zhuye_core::config::M4Deps {
        compactor,
        memory,
        skills: skills_catalog,
        prompt_builder,
        active_agent_def: active_def,
        recorder: Some(recorder),
        note_store,
        file_recovery,
        subagent_registry,
        // v1.6.1:`/prompt` Switch 重扫用的目录序(home 在前、workspace 在后,
        // 与启动时 init_library 同序)。
        prompt_dirs: vec![home_prompts, ws_prompts],
    }
}

// ── M5:子代理工厂(call_<role>)─────────────────────────────────────────

/// 在 M4 之上构建 M5:`SubAgentFactory` + 为每个声明的 subagent spec 注册
/// `call_<role>` 工具,并补入 always-on 可见集(否则模型看不到 schema,
/// 永远无法委派)。返回 factory 供 caller 注入 runtime registry / cancel /
/// telemetry。与 exec 的 `bootstrap_m5` 同逻辑,`tools` 走参数。
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_m5(
    workspace: &Path,
    agent_name: &str,
    model: &str,
    registry: &SharedModelRegistry,
    thread_id: ThreadId,
    parent_recorder: Arc<dyn zhuye_protocol::RolloutRecorder>,
    m4: &zhuye_core::config::M4Deps,
    initial_cfg: &zhuye_config::ReflectConfig,
    tools: &Arc<ToolRegistry>,
) -> Arc<SubAgentFactory> {
    let _ = (workspace, agent_name);
    // v1.x 功能 1:`[subagent_providers]` 独立 child registry(独立
    // base_url + api_key + model);`None` = 父子共享。
    let child_registry: Option<SharedModelRegistry> = initial_cfg.to_child_registry().map(Arc::new);
    let factory = Arc::new(SubAgentFactory::new(
        thread_id,
        model.to_string(),
        registry.clone(),
        child_registry,
        tools.clone(),
        CancellationToken::new(),
        Some(parent_recorder),
    ));
    factory.set_subagent_registry(m4.subagent_registry.clone());
    // 注入父 skills catalog,让 subagent 也能 LoadSkill。
    factory.set_parent_skills(m4.skills.clone());

    // 合并 subagent spec:优先级 TOML `[[subagents]]` > workspace
    // `.zhuye/subagents/*.md` > home `~/.zhuye/subagents/*.md` >
    // 硬编码 explorer(仅当全部为空时)。
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = std::path::Path::new(&home);
    let home_md = zhuye_subagent::load_subagents_dir(&home_path.join(".zhuye").join("subagents"))
        .unwrap_or_default();
    let ws_md = zhuye_subagent::load_subagents_dir(&workspace.join(".zhuye").join("subagents"))
        .unwrap_or_default();
    let toml_specs = initial_cfg.subagents.clone();
    let mut configs = zhuye_subagent::merge_by_priority(vec![home_md, ws_md, toml_specs]);
    if configs.is_empty() {
        configs.push(zhuye_config::SubagentSpecConfig {
            name: "Explorer".into(),
            role: "explorer".into(),
            model: None,
            system_prompt:
                "You are an explorer subagent. Inspect the codebase and return a concise summary."
                    .into(),
            allowed_tools: vec!["bash".into(), "read".into(), "grep".into(), "glob".into()],
            allowed_skills: vec![],
            max_turns: None,
        });
    }

    let call_tool_names: Vec<String> = configs
        .iter()
        .map(|sc| format!("call_{}", sc.role))
        .collect();

    for sc in configs {
        let spec = zhuye_subagent::SubAgentSpec {
            name: sc.name,
            role: sc.role.clone(),
            model: sc.model,
            system_prompt: sc.system_prompt,
            allowed_tools: sc.allowed_tools,
            data_transfer: Default::default(),
            max_turns: sc.max_turns,
            allowed_skills: sc.allowed_skills,
        };
        let role = sc.role;
        let tool: Arc<dyn zhuye_tools::Tool> =
            Arc::new(zhuye_subagent::CallSubAgentTool::new(factory.clone(), spec));
        // 走 `register_runtime_tool` 应用 Runtime 源安全 floor。
        tools.register_runtime_tool(tool);
        tracing::debug!(role = %role, "registered subagent spec");
    }

    // 补入 always-on 可见集:`call_<role>` 是运行时动态注册工具,不在静态
    // ALWAYS_ON_TOOLS 里;pre_loop 按 skills 计算 effective_tools,不补则
    // 模型永远看不到子代理工具 schema。
    m4.skills.add_always_on_tools(call_tool_names);

    factory
}

// ── M6:MCP ─────────────────────────────────────────────────────────────

/// 启动 `[mcp_servers]` 配置的 MCP server 集合(与 exec 的 `bootstrap_m6`
/// 同逻辑;差异:`tools` 走参数,生命周期事件发到 caller 给的 channel)。
///
/// 单 server 启动失败仅 warn,不阻塞其它 server 与 agent 启动。
/// 返回 `Arc<McpConnectionManager>` 供插件层复用(资源读取工具)。
pub(crate) async fn bootstrap_mcp(
    initial_cfg: &zhuye_config::ReflectConfig,
    event_tx: tokio::sync::mpsc::Sender<Event>,
    tools: Arc<ToolRegistry>,
) -> Option<Arc<McpConnectionManager>> {
    let configs = match initial_cfg.mcp_server_configs() {
        Ok(c) if c.is_empty() => return None,
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "MCP config invalid; skipping all MCP servers");
            return None;
        }
    };
    let (internal_tx, mut internal_rx) = tokio::sync::mpsc::channel::<McpLifecycleEvent>(32);
    let manager = Arc::new(McpConnectionManager::new(internal_tx));

    // 后台 task:MCP 生命周期事件 → protocol Event → TUI 事件循环。
    let event_tx_clone = event_tx.clone();
    tokio::spawn(async move {
        while let Some(evt) = internal_rx.recv().await {
            let msg = match evt {
                McpLifecycleEvent::Started {
                    server,
                    tools,
                    tool_names,
                    transport,
                } => zhuye_protocol::event_msg::EventMsg::McpServerStarted(
                    zhuye_protocol::McpServerStartedEvent {
                        server,
                        tool_count: tools,
                        tool_names,
                        transport: transport_mirror_from_runtime(transport),
                    },
                ),
                McpLifecycleEvent::Failed {
                    server,
                    error,
                    will_retry,
                } => zhuye_protocol::event_msg::EventMsg::McpServerFailed(
                    zhuye_protocol::McpServerFailedEvent {
                        server,
                        error,
                        will_retry,
                    },
                ),
                McpLifecycleEvent::Stopped { server: _ } => continue,
            };
            if event_tx_clone
                .send(Event::new(EVENT_ID_NONE, msg))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // 并发启动每个 server;拿到 handle 后把 adapter 逐个注册进工具表。
    for cfg_shape in &configs {
        let cfg: McpServerConfig = McpServerConfig::from(cfg_shape.clone());
        let manager_clone = manager.clone();
        let tools_clone = tools.clone();
        // adapter 调用完成后 emit `McpToolInvoked`(TUI 侧转换层当前不渲染,
        // 通道保留给后续 /mcp 详情视图)。
        let invoked_tx = event_tx.clone();
        tokio::spawn(async move {
            match manager_clone.start_server(cfg.clone()).await {
                Ok(handle) => {
                    for desc in &handle.tools {
                        let adapter = McpToolAdapter::from_descriptor(
                            handle.inner.clone(),
                            desc,
                            &cfg.name,
                            cfg.timeout,
                            Some(invoked_tx.clone()),
                        );
                        let arc: Arc<dyn zhuye_tools::Tool> = Arc::new(adapter);
                        // MCP 工具走 `ToolSource::Mcp` + 安全 floor;重名跳过。
                        if !tools_clone.register_if_absent_with_floor(ToolSource::Mcp, arc) {
                            tracing::warn!(
                                tool = %desc.full_name,
                                "MCP tool name collision, skipped"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(server = %cfg.name, error = %e, "MCP server failed to start");
                }
            }
        });
    }
    tracing::info!(
        mcp_servers = configs.len(),
        "MCP bootstrap: spawning start tasks"
    );
    Some(manager)
}

/// `zhuye_mcp::McpTransport` → `zhuye_protocol::McpTransportMirror`。
fn transport_mirror_from_runtime(t: zhuye_mcp::McpTransport) -> zhuye_protocol::McpTransportMirror {
    match t {
        zhuye_mcp::McpTransport::Stdio => zhuye_protocol::McpTransportMirror::Stdio,
        zhuye_mcp::McpTransport::Http => zhuye_protocol::McpTransportMirror::Http,
        zhuye_mcp::McpTransport::Sse => zhuye_protocol::McpTransportMirror::Sse,
    }
}

// ── LSP ────────────────────────────────────────────────────────────────

/// 启动 `[lsp_servers]` 配置的 LSP server 集合并注册单例 `lsp` 工具
/// (与 exec 的 `bootstrap_lsp` 同逻辑)。单 server 失败仅 warn。
pub(crate) async fn bootstrap_lsp(
    initial_cfg: &zhuye_config::ReflectConfig,
    event_tx: tokio::sync::mpsc::Sender<Event>,
    tools: Arc<ToolRegistry>,
) -> Option<Arc<LspConnectionManager>> {
    let configs = match initial_cfg.lsp_server_configs() {
        Ok(c) if c.is_empty() => return None,
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "LSP config invalid; skipping all LSP servers");
            return None;
        }
    };
    let (internal_tx, mut internal_rx) = tokio::sync::mpsc::channel::<LspLifecycleEvent>(32);
    let manager = Arc::new(LspConnectionManager::new(internal_tx));

    let event_tx_clone = event_tx.clone();
    tokio::spawn(async move {
        while let Some(evt) = internal_rx.recv().await {
            let msg = match evt {
                LspLifecycleEvent::Started {
                    server,
                    methods,
                    language_ids,
                } => zhuye_protocol::event_msg::EventMsg::LspServerStarted(
                    zhuye_protocol::LspServerStartedEvent {
                        server,
                        methods,
                        language_ids,
                    },
                ),
                LspLifecycleEvent::Failed {
                    server,
                    error,
                    will_retry,
                } => zhuye_protocol::event_msg::EventMsg::LspServerFailed(
                    zhuye_protocol::LspServerFailedEvent {
                        server,
                        error,
                        will_retry,
                    },
                ),
                LspLifecycleEvent::Stopped { server: _ } => continue,
            };
            if event_tx_clone
                .send(Event::new(EVENT_ID_NONE, msg))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // 单例 LspTool(action enum 分派,不需要 per-method 注册)。
    let tool = Arc::new(LspTool::new(manager.clone()));
    tools.register_runtime_tool(tool);

    for cfg_shape in &configs {
        let cfg: zhuye_lsp::LspServerConfig = match cfg_shape.clone().try_into() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "LSP config shape→strong-type conversion failed");
                continue;
            }
        };
        let mgr = manager.clone();
        tokio::spawn(async move {
            if let Err(e) = mgr.start_server(cfg).await {
                tracing::warn!(error = %e, "LSP server failed to start");
            }
        });
    }
    tracing::info!(
        lsp_servers = configs.len(),
        "LSP bootstrap: spawning start tasks"
    );
    Some(manager)
}

// ── coordinator 启停 ───────────────────────────────────────────────────

/// 根据 `[coordinator]` 配置启用/关闭 coordinator 模式(与 exec 的
/// `apply_coordinator_from_config` 同逻辑)。
pub(crate) fn apply_coordinator_from_config(
    cfg: &zhuye_config::ReflectConfig,
    workspace: &Path,
    m4: Option<&zhuye_core::config::M4Deps>,
    factory: &SubAgentFactory,
    tools: &ToolRegistry,
) {
    let default_section = zhuye_config::CoordinatorSection::default();
    let section = cfg.coordinator.as_ref().unwrap_or(&default_section);
    let coord_cfg = CoordinatorConfig::from_env_or_config(section);

    if let Some(m4) = m4 {
        let mut pb = m4.prompt_builder.lock();
        if coord_cfg.enabled {
            pb.upsert_section("Coordinator", coord_cfg.system_prompt.clone());
        } else {
            pb.remove_section("Coordinator");
        }
    }

    if coord_cfg.enabled {
        let thread_id = factory.parent_session_id();
        ensure_scratchpad(&coord_cfg, workspace, &thread_id.to_string());
        let scratchpad = build_scratchpad_path(workspace, &thread_id.to_string());
        let footer: String = coordinator_footer(&coord_cfg.system_prompt);
        factory.set_coordinator_mode(true, Some(footer));
        // P2 `git-worktree-auto`:coordinator 启用时,worker spawn 自动隔离到
        // 独立 worktree;非 git 仓库仅 warn 跳过,不阻断 coordinator。
        match zhuye_tools::git_root(std::path::Path::new(workspace)) {
            Ok(root) => {
                let coord = Arc::new(zhuye_tools::WorktreeCoordinator::new(root));
                factory.set_worktree_coordinator(Some(coord));
                tracing::info!("coordinator worktree 隔离已启用");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "coordinator worktree 隔离跳过(非 git 仓库或 git_root 解析失败)"
                );
            }
        }
        tools.register_with_source(
            ToolSource::Builtin,
            Arc::new(zhuye_task::tools::WriteNoteTool::new(Arc::new(
                scratchpad.clone(),
            ))),
        );
        tools.register_with_source(
            ToolSource::Builtin,
            Arc::new(zhuye_task::tools::ReadNotesTool::new(Arc::new(
                scratchpad.clone(),
            ))),
        );
        tracing::info!(
            scratchpad = %scratchpad.display(),
            max_workers = coord_cfg.max_workers,
            "coordinator mode active"
        );
    } else {
        factory.set_coordinator_mode(false, None);
        tools.unregister("WriteNote");
        tools.unregister("ReadNotes");
        tracing::info!("coordinator mode disabled");
    }
}

/// 取 coordinator system_prompt 的末尾 200 个**字符**作为 footer。
/// 按字符边界截取,避免多字节中文 prompt 触发 `byte index is not a char
/// boundary` panic(exec 侧同款回归)。
fn coordinator_footer(prompt: &str) -> String {
    prompt
        .chars()
        .rev()
        .take(200)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_footer_handles_multibyte_without_panic() {
        // 回归:中文 prompt 长度 > 200 字节,字节切片实现会 panic。
        let prompt: String = "中".repeat(300);
        let footer = coordinator_footer(&prompt);
        assert_eq!(footer.chars().count(), 200);
        assert!(footer.chars().all(|c| c == '中'));
    }

    #[test]
    fn coordinator_footer_short_prompt_returned_verbatim() {
        let prompt = "你好,这是一个很短的 coordinator prompt。";
        let footer = coordinator_footer(prompt);
        assert_eq!(footer, prompt);
    }

    #[test]
    fn coordinator_footer_preserves_order() {
        let mut prompt = String::new();
        for i in 0..250u8 {
            prompt.push((b'a' + (i % 26)) as char);
            prompt.push(char::from_digit((i / 26) as u32, 10).unwrap_or('0'));
        }
        let footer = coordinator_footer(&prompt);
        assert!(footer.chars().count() == 200);
        assert!(
            prompt.ends_with(&footer),
            "footer must be an ordered suffix"
        );
    }

    #[test]
    fn noop_summarizer_always_errors() {
        // NoopSummarizer 语义:恒报错,让 compactor 退回 smart_prune。
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let s = NoopSummarizer;
            assert!(s.summarize_full(&[]).await.is_err());
            assert!(s.summarize_recent(&[], None).await.is_err());
        });
    }

    #[test]
    fn build_sanitizer_defaults_on_bad_patterns() {
        // 非法 regex 的 extra_patterns 应 warn + fallback 默认,不 panic。
        let section = zhuye_config::SanitizeSection {
            enabled: Some(true),
            marker: None,
            disable_default_patterns: None,
            extra_patterns: Some(vec!["(unclosed".to_string()]),
        };
        let _ = build_sanitizer(Some(&section));
    }
}
