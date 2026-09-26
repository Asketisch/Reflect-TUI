//! TUI 侧运行时装配:对齐 headless(`reflect-exec`)的 M4/M5/M6/LSP 能力。
//!
//! 不能直接复用 `reflect-exec` 的 `bootstrap_m4` —— 它依赖该 bin crate 的
//! thread-local `TOOLS` 句柄。本模块按同一逻辑移植为纯参数式函数,直接调用
//! 相同的 submodule 库 crate,让 TUI 进程获得与无头模式一致的能力面:
//! agent 定义、skills、session notes、post-compact 恢复、LLM 摘要压缩、
//! coordinator、MCP、LSP、telemetry、quota、sanitizer。

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use reflect_agent_def::AgentDefinition;
use reflect_compact::{Compactor, LlmSummarizer, Summarizer, SummarizerError};
use reflect_llm::{ChatMessage, SharedModelRegistry};
use reflect_lsp::{LspConnectionManager, LspLifecycleEvent, LspTool};
use reflect_mcp::{McpConnectionManager, McpLifecycleEvent, McpServerConfig, McpToolAdapter};
use reflect_memory::{InMemoryStore, MemoryStore};
use reflect_prompt::PromptBuilder;
use reflect_protocol::{EVENT_ID_NONE, Event, ThreadId};
use reflect_skills::SkillsCatalog;
use reflect_subagent::SubAgentFactory;
use reflect_task::coordinator::{CoordinatorConfig, build_scratchpad_path, ensure_scratchpad};
use reflect_tools::{SanitizeConfig, Sanitizer, ToolRegistry, ToolSource};
use tokio_util::sync::CancellationToken;

/// 默认 agent 系统提示(用户未提供 `.reflect/agents/<name>.md` 时的 fallback)。
///
/// 与 `reflect-exec` 的 `DEFAULT_SYSTEM_PROMPT` 保持同一份文本:只覆盖 5 条
/// 最通用的编码 agent 行为约束,具体能力由工具和 skills 在运行时叠加。
pub(crate) const DEFAULT_SYSTEM_PROMPT: &str = "你是一名编码助手。在回答用户请求时遵守以下约定:\n\
\n\
1. 先读后改:编辑文件前先用 `read` 读目标文件;做有针对性的修改,而非整文件重写。\n\
2. 先约定后实现:不要假设库或工具可用。从 README、package manifest(如 \
Cargo.toml / package.json)、邻近文件确认约定与风格,模仿现有代码的命名与惯用法。\n\
3. 不过度设计:只做被要求的事,不主动增加范围外的功能、重构或抽象。三行重复 \
优于过早抽象。任务完成后不要顺手创建未要求的 README / 测试 / 文档。\n\
4. 注释克制:除非用户要求,不要写注释;需要写注释时,只解释「为什么」, \
不要复述代码「是什么」。\n\
5. 破坏性操作前确认:对难以撤销或对外可见的操作,先说明再执行。典型例子包括 \
`rm -rf`、`git reset --hard`、删除分支、`git push`(含 `--force`)、 \
发送外部请求或提交等。\n\
\n\
语言:用与用户提问相同的语言回答。";

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

/// 把 `~/.reflect/config.toml [sanitize]` 段接到 queue 的脱敏 pass。
/// 用户写错 `extra_patterns[i]` 时 warn 后 fallback 默认 10-pattern,不阻塞启动。
pub(crate) fn build_sanitizer(section: Option<&reflect_config::SanitizeSection>) -> Arc<Sanitizer> {
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
/// `enabled = false`(显式)→ `None`;缺省 → 启用,目录默认 `~/.reflect/traces`。
pub(crate) fn build_telemetry_sink(
    cfg: &reflect_config::ReflectConfig,
) -> Option<Arc<reflect_telemetry::TelemetrySink>> {
    let default_section = reflect_config::TelemetrySection::default();
    let section = cfg.telemetry.as_ref().unwrap_or(&default_section);
    if !section.is_enabled() {
        return None;
    }
    let base_dir = section
        .dir
        .clone()
        .unwrap_or_else(|| reflect_telemetry::resolve_traces_dir(None));
    let session_id = uuid::Uuid::new_v4().to_string();
    let sink = reflect_telemetry::TelemetrySink::new(base_dir, session_id);

    if let Some(lf) = cfg.hooks.langfuse_tracker.as_ref() {
        if lf.enabled.unwrap_or(false) {
            if let (Some(endpoint), Some(pk), Some(sk)) = (
                lf.endpoint.as_ref(),
                lf.public_key.as_ref(),
                lf.secret_key.as_ref(),
            ) {
                if !endpoint.is_empty() && !pk.is_empty() && !sk.is_empty() {
                    let lf_cfg = reflect_telemetry::LangfuseConfig {
                        endpoint: endpoint.clone(),
                        public_key: pk.clone(),
                        secret_key: sk.clone(),
                        batch_size: 64,
                    };
                    let exporter = reflect_telemetry::LangfuseExporter::start(Some(lf_cfg));
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
    cfg: &reflect_config::ReflectConfig,
) -> Option<reflect_llm::SharedQuotaTracker> {
    use reflect_config::QuotaSource;
    use reflect_llm::{
        KimiQuotaProvider, MinimaxQuotaProvider, ZenmuxQuotaProvider, ZhipuQuotaProvider,
    };
    use std::sync::Arc as StdArc;

    let tracker = Arc::new(reflect_llm::QuotaTracker::new());
    let mut any = false;

    fn make_provider(src: &QuotaSource) -> Option<Arc<dyn reflect_llm::QuotaProvider>> {
        match src {
            QuotaSource::Kimi => {
                Some(StdArc::new(KimiQuotaProvider) as Arc<dyn reflect_llm::QuotaProvider>)
            }
            QuotaSource::Zhipu => {
                Some(StdArc::new(ZhipuQuotaProvider) as Arc<dyn reflect_llm::QuotaProvider>)
            }
            QuotaSource::Minimax => {
                Some(StdArc::new(MinimaxQuotaProvider) as Arc<dyn reflect_llm::QuotaProvider>)
            }
            QuotaSource::Zenmux => {
                Some(StdArc::new(ZenmuxQuotaProvider) as Arc<dyn reflect_llm::QuotaProvider>)
            }
            QuotaSource::Volcengine | QuotaSource::AnthropicUsage | QuotaSource::OpenAIUsage => {
                None
            }
        }
    }

    let mut register_provider = |provider: &str, creds: &[reflect_config::CredentialConfig]| {
        for c in creds {
            if let Some(q) = &c.quota {
                let rt_source = q.check_via.as_ref().map(|s| match s {
                    QuotaSource::Kimi => reflect_llm::QuotaSource::Kimi,
                    QuotaSource::Zhipu => reflect_llm::QuotaSource::Zhipu,
                    QuotaSource::Minimax => reflect_llm::QuotaSource::Minimax,
                    QuotaSource::Zenmux => reflect_llm::QuotaSource::Zenmux,
                    QuotaSource::Volcengine => reflect_llm::QuotaSource::Volcengine,
                    QuotaSource::AnthropicUsage => reflect_llm::QuotaSource::AnthropicUsage,
                    QuotaSource::OpenAIUsage => reflect_llm::QuotaSource::OpenAIUsage,
                });
                let rt = reflect_llm::QuotaConfig {
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

/// `$REFLECT_HOME/session-notes/<thread_id>.jsonl` 上的 note store。
/// 任何一步失败返回 `None`,caller fallback 纯内存 store,不阻塞启动。
fn build_note_store(thread_id: ThreadId) -> Option<Arc<dyn reflect_notes::NoteStore>> {
    let home = reflect_notes::resolve_notes_home()?;
    let dir = home.join("session-notes");
    let path = dir.join(format!("{}.jsonl", thread_id));
    match reflect_notes::FileBackedNoteStore::open_or_create_default(&path) {
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
    cfg: &reflect_config::ReflectConfig,
    telemetry: Option<Arc<reflect_telemetry::TelemetrySink>>,
    recorder: Arc<dyn reflect_protocol::RolloutRecorder>,
    tools: &Arc<ToolRegistry>,
) -> reflect_core::config::M4Deps {
    // Agent 定义:workspace + home 两个目录合并,同名时 workspace 覆盖 home。
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = std::path::Path::new(&home);
    let ws_agents = workspace.join(".reflect").join("agents");
    let home_agents = home_path.join(".reflect").join("agents");
    let mut agents = reflect_agent_def::load_agents_dir(&ws_agents).unwrap_or_default();
    agents.extend(reflect_agent_def::load_agents_dir(&home_agents).unwrap_or_default());
    #[allow(clippy::field_reassign_with_default)] // 与 exec 同款,耐字段增减
    let active_def = agents.remove(agent_name).unwrap_or_else(|| {
        let mut d = AgentDefinition::default();
        d.name = agent_name.to_string();
        d.description = "default agent".into();
        d.system_prompt = DEFAULT_SYSTEM_PROMPT.into();
        d
    });
    let active_def = Arc::new(active_def);

    // Skills:扫描 workspace + home skill 目录,合并编译期内置技能包
    // (code-review / commit-helper 等)。同名 skill 以文件目录扫描结果优先。
    let ws_skills = workspace.join(".reflect").join("skills");
    let home_skills = home_path.join(".reflect").join("skills");
    let skill_dirs: Vec<&std::path::Path> = vec![&ws_skills, &home_skills];
    let skills_catalog = SkillsCatalog::new();
    skills_catalog.scan(&skill_dirs);
    reflect_skills::merge_bundled(&skills_catalog);
    let skills_catalog = Arc::new(skills_catalog);

    // 注册 `load_skill` + `read_skill_resource`(渐进披露第三级)。
    // 走 `register_runtime_tool` 应用 Runtime 源的 `Prompt` 安全 floor。
    let load_skill = Arc::new(reflect_skills::LoadSkillTool::new(skills_catalog.clone()));
    tools.register_runtime_tool(load_skill);
    let read_skill_resource = Arc::new(reflect_skills::ReadSkillResourceTool::new(
        skills_catalog.clone(),
    ));
    tools.register_runtime_tool(read_skill_resource);

    // Memory:project + user 走文件,session scope 走内存(进程退出即丢,
    // 对话历史已在 rollout JSONL)。
    let file_store = Arc::new(reflect_memory::FileMemoryStore::new(workspace, home_path));
    let memory: Arc<dyn MemoryStore> = Arc::new(InMemoryStore::with_fallback(file_store));

    // Session notes:FIFO 30 条 + JSONL 落盘,工具写入与 pre_loop 注入读同一 Arc。
    let note_store: Arc<dyn reflect_notes::NoteStore> = match build_note_store(thread_id) {
        Some(store) => store,
        None => Arc::new(reflect_notes::InMemoryNoteStore::new()),
    };
    let note_tool: Arc<dyn reflect_tools::Tool> =
        Arc::new(reflect_notes::AddSessionNoteTool::new(note_store.clone()));
    tools.register_runtime_tool(note_tool);

    // post-compact 活跃文件恢复 + 已完成子代理调用注册表(防 LLM 重复 spawn)。
    let file_recovery = Arc::new(reflect_recovery::ActiveFileRecovery::new(Arc::from(
        workspace.to_path_buf(),
    )));
    let subagent_registry = reflect_recovery::SubagentRegistry::shared();

    // 压缩器:接 LLM 摘要器(Router::Compact slot,失败跨 credential failover),
    // 无可用客户端时退回 Noop(compactor 降级 smart_prune)。
    let summarizer: Arc<dyn Summarizer> = {
        let policy = cfg.routing_policy();
        let spec = {
            let p = policy.resolve(reflect_llm::Role::Compact).primary.clone();
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
    // 触发阈值:env `REFLECT_AUTO_COMPACT_INPUT_TOKENS` > TOML > 默认值。
    let compactor_cfg =
        reflect_core::config::compactor_config_from_env_and_toml(toml_trigger_tokens);
    // v1.4 D2 记忆固化门控(REFLECT_MEMORY_CONSOLIDATION):默认关闭;
    // LLM 摘要器在位时才接线(Noop 提取不出内容)。
    let consolidation_scope = match std::env::var("REFLECT_MEMORY_CONSOLIDATION")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "session" => Some(reflect_memory::MemoryScope::Session),
        "project" => Some(reflect_memory::MemoryScope::Project),
        "user" => Some(reflect_memory::MemoryScope::User),
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
            .unwrap_or(&reflect_config::CoordinatorSection::default()),
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

    reflect_core::config::M4Deps {
        compactor,
        memory,
        skills: skills_catalog,
        prompt_builder,
        active_agent_def: active_def,
        recorder: Some(recorder),
        note_store,
        file_recovery,
        subagent_registry,
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
    parent_recorder: Arc<dyn reflect_protocol::RolloutRecorder>,
    m4: &reflect_core::config::M4Deps,
    initial_cfg: &reflect_config::ReflectConfig,
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
    // `.reflect/subagents/*.md` > home `~/.reflect/subagents/*.md` >
    // 硬编码 explorer(仅当全部为空时)。
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = std::path::Path::new(&home);
    let home_md =
        reflect_subagent::load_subagents_dir(&home_path.join(".reflect").join("subagents"))
            .unwrap_or_default();
    let ws_md = reflect_subagent::load_subagents_dir(&workspace.join(".reflect").join("subagents"))
        .unwrap_or_default();
    let toml_specs = initial_cfg.subagents.clone();
    let mut configs = reflect_subagent::merge_by_priority(vec![home_md, ws_md, toml_specs]);
    if configs.is_empty() {
        configs.push(reflect_config::SubagentSpecConfig {
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
        let spec = reflect_subagent::SubAgentSpec {
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
        let tool: Arc<dyn reflect_tools::Tool> = Arc::new(reflect_subagent::CallSubAgentTool::new(
            factory.clone(),
            spec,
        ));
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
    initial_cfg: &reflect_config::ReflectConfig,
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
                } => reflect_protocol::event_msg::EventMsg::McpServerStarted(
                    reflect_protocol::McpServerStartedEvent {
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
                } => reflect_protocol::event_msg::EventMsg::McpServerFailed(
                    reflect_protocol::McpServerFailedEvent {
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
                        let arc: Arc<dyn reflect_tools::Tool> = Arc::new(adapter);
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

/// `reflect_mcp::McpTransport` → `reflect_protocol::McpTransportMirror`。
fn transport_mirror_from_runtime(
    t: reflect_mcp::McpTransport,
) -> reflect_protocol::McpTransportMirror {
    match t {
        reflect_mcp::McpTransport::Stdio => reflect_protocol::McpTransportMirror::Stdio,
        reflect_mcp::McpTransport::Http => reflect_protocol::McpTransportMirror::Http,
        reflect_mcp::McpTransport::Sse => reflect_protocol::McpTransportMirror::Sse,
    }
}

// ── LSP ────────────────────────────────────────────────────────────────

/// 启动 `[lsp_servers]` 配置的 LSP server 集合并注册单例 `lsp` 工具
/// (与 exec 的 `bootstrap_lsp` 同逻辑)。单 server 失败仅 warn。
pub(crate) async fn bootstrap_lsp(
    initial_cfg: &reflect_config::ReflectConfig,
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
                } => reflect_protocol::event_msg::EventMsg::LspServerStarted(
                    reflect_protocol::LspServerStartedEvent {
                        server,
                        methods,
                        language_ids,
                    },
                ),
                LspLifecycleEvent::Failed {
                    server,
                    error,
                    will_retry,
                } => reflect_protocol::event_msg::EventMsg::LspServerFailed(
                    reflect_protocol::LspServerFailedEvent {
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
        let cfg: reflect_lsp::LspServerConfig = match cfg_shape.clone().try_into() {
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
    cfg: &reflect_config::ReflectConfig,
    workspace: &Path,
    m4: Option<&reflect_core::config::M4Deps>,
    factory: &SubAgentFactory,
    tools: &ToolRegistry,
) {
    let default_section = reflect_config::CoordinatorSection::default();
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
        match reflect_tools::git_root(std::path::Path::new(workspace)) {
            Ok(root) => {
                let coord = Arc::new(reflect_tools::WorktreeCoordinator::new(root));
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
            Arc::new(reflect_task::tools::WriteNoteTool::new(Arc::new(
                scratchpad.clone(),
            ))),
        );
        tools.register_with_source(
            ToolSource::Builtin,
            Arc::new(reflect_task::tools::ReadNotesTool::new(Arc::new(
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
        let section = reflect_config::SanitizeSection {
            enabled: Some(true),
            marker: None,
            disable_default_patterns: None,
            extra_patterns: Some(vec!["(unclosed".to_string()]),
        };
        let _ = build_sanitizer(Some(&section));
    }
}
