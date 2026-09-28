//! 稳定 Reflect 风格 TUI 的 Reflect runtime 引导启动。
//!
//! 加载 config,构建 model registry / provider / tools / AgentThread,
//! 然后在 `tui::run_async` 中进入 Reflect 风格的聊天循环。
//!
//! 能力面对齐 headless(`reflect-exec`):M4(agent 定义 / skills / notes /
//! 恢复 / LLM 摘要压缩)、M5(子代理工厂 + runtime registry)、M6(MCP)、
//! LSP、telemetry、quota、sanitizer、hook engine 的装配细节在
//! [`crate::bootstrap_wiring`],本文件只做编排。

use crate::TuiArgs;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use reflect_core::{AgentConfig, AgentThread};
use reflect_hooks::builtins::{PlanModeGate, build_read_before_edit};
use reflect_llm::ModelRegistry;
use reflect_permissions::{FilePermissionStore, StorePermissionResolver};
use reflect_tools::{ToolRegistry, builtins};

/// bootstrap 交给 TUI 主循环的运行时句柄:主循环需要的、但不属于
/// `AgentThread` 公开面的装配产物。
pub struct TuiRuntimeHandles {
    /// MCP / LSP 生命周期事件通道接收端(bootstrap 期起的服务把
    /// `EventMsg::McpServerStarted/Failed` 等发进来,主循环转 UiEvent)。
    pub lifecycle_rx: tokio::sync::mpsc::Receiver<reflect_protocol::Event>,
    /// 会话取消令牌:主循环退出时级联取消在飞子代理(factory 与 engine 共享)。
    pub cancel: tokio_util::sync::CancellationToken,
    /// 文件权限存储(approvals 选 always-allow 时写规则,跨重启生效)。
    pub persistent_permissions: Option<Arc<dyn reflect_permissions::PermissionStore>>,
    /// 模型注册表(`/model` 切换前的可用性校验)。
    pub registry: Arc<ModelRegistry>,
    /// 启动期完整 config(`/model` 解析 provider/model、状态展示)。
    pub config: reflect_config::ReflectConfig,
    /// M2:resume 回放出的历史消息(非 resume 启动为空)。主循环据此把
    /// 既往对话回填进 scrollback,让 `-c` / `--resume` 有连续的视觉上下文。
    pub prior_messages: Vec<reflect_llm::ChatMessage>,
    /// 运行时插件状态(bootstrap 期挂载的 skills / agents / MCP /
    /// shell hooks / slash 命令)。主循环在提交用户输入前用它展开
    /// `/plugin:ns:name` 命令;未装插件时为空句柄。
    pub plugin_runtime: reflect_plugin::runtime::SharedPluginRuntime,
}

/// 加载 config + 构建 Reflect runtime,然后进入 TUI 主循环。
pub fn run(args: TuiArgs) -> Result<()> {
    crate::logging::init();

    // M4:v1.4 D1 真实分词器(tokenizer feature)—— 压缩触发阈值与上下文
    // 估算用 tiktoken 替代 len/3.5 启发式。set-once;失败仅 warn,回退
    // 启发式,不阻塞启动(与 headless 同款注册)。
    match reflect_compact::global_tiktoken_estimator() {
        Ok(est) => {
            if reflect_compact::set_global_estimator(est) {
                tracing::info!("tiktoken estimator registered (tokenizer feature)");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "tiktoken estimator init failed; using heuristic");
        }
    }

    // 使用 current_thread runtime，使 TUI 主循环在主线程上运行。
    // 这点很关键：fork 后的 crossterm 的 event::poll/read 使用了一个
    // 绑定到调用线程的内部锁。在 multi_thread runtime 下，async 任务
    // 会被工作线程抢走，导致 poll() 始终返回 false（与另一个线程
    // 产生锁竞争）。使用 current_thread 可以让一切保持在同一线程上。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move { bootstrap_and_run(args).await })
}

async fn bootstrap_and_run(args: TuiArgs) -> Result<()> {
    use crate::bootstrap_wiring::{
        apply_coordinator_from_config, bootstrap_lsp, bootstrap_mcp, build_m4, build_m5,
        build_quota_tracker, build_sanitizer, build_telemetry_sink,
    };

    // 1. 加载 config + provider/model。
    let reflect_cfg = reflect_config::load_default();
    // SAFETY: 使用 current_thread runtime，此时保证所有代码在单线程中执行，
    // 无并发读取环境变量的可能。set_var 仅在 bootstrap 阶段调用一次。
    if reflect_cfg.sandbox.os_level && std::env::var_os("REFLECT_SANDBOX_OS_LEVEL").is_none() {
        #[allow(unsafe_op_in_unsafe_fn)]
        unsafe {
            std::env::set_var("REFLECT_SANDBOX_OS_LEVEL", "1");
        }
    }
    let registry = Arc::new(ModelRegistry::new());
    reflect_cfg
        .apply_to_registry(&registry)
        .map_err(|e| anyhow::anyhow!("provider 构造失败: {e}"))?;
    let provider = reflect_cfg.active_provider().ok_or_else(|| {
        anyhow::anyhow!(
            "未配置任何 provider:请在 ~/.reflect/config.toml 设置 [active]/[anthropic]/[openai],\
             或设置 OPENAI_API_KEY / ANTHROPIC_API_KEY 环境变量"
        )
    })?;
    // v1.3 model 解析诚实化:不再回落内置默认,缺配置时如实报错。
    let model_name = reflect_cfg.resolve_model(provider).ok_or_else(|| {
        anyhow::anyhow!(
            "provider `{provider}` 未配置 model:请在 ~/.reflect/config.toml \
             设置 [active].model 或 [{provider}].model,或设置 REFLECT_MODEL 环境变量"
        )
    })?;
    let model = format!("{}/{}", provider, model_name);

    // 1.5 `[sanitize]` 脱敏器 + `[hooks]` hook engine(含 builtin + 插件
    // hook),AgentThread 构造时注入 —— 此前 TUI 传 None,两段配置是死信。
    let sanitizer = build_sanitizer(reflect_cfg.sanitize.as_ref());
    let hook_engine: Arc<reflect_hooks::HookEngine> = Arc::new(
        reflect_hooks::config::HooksConfig::from_reflect_section(&reflect_cfg.hooks).build_engine(),
    );

    // 2. 注册内置工具的 Tool registry（与 reflect-exec 同集合）。
    let tools = Arc::new(ToolRegistry::default());
    tools.register(Arc::new(builtins::EchoTool));
    tools.register(Arc::new(builtins::BashTool));
    tools.register(Arc::new(builtins::ReadTool));
    tools.register(Arc::new(builtins::WriteTool));
    tools.register(Arc::new(builtins::EditTool));
    tools.register(Arc::new(builtins::DeleteTool));
    tools.register(Arc::new(builtins::GrepTool));
    tools.register(Arc::new(builtins::GlobTool));
    tools.register(Arc::new(reflect_ast::AstTool::new()));
    tools.register(Arc::new(builtins::WebFetchTool::new()));
    tools.register(Arc::new(builtins::WebSearchTool::new()));
    tools.register(Arc::new(builtins::EnterPlanModeTool));
    tools.register(Arc::new(builtins::ExitPlanModeTool));
    // v1.x Plan mode 写盘工具 —— Plan 阶段把 plan markdown 落到
    // `<workspace>/.reflect/plan/<name>.md`,供 ExitPlanMode 读取。
    // required_permission = Auto,Plan mode 下免审批直写。必须与
    // EnterPlanMode / ExitPlanMode 一起注册,否则 LLM 看到 schema
    // (因 ALWAYS_ON_TOOLS 含 PlanWrite)却找不到实现 → "tool not found"。
    tools.register(Arc::new(builtins::PlanWriteTool));
    // v1.x:AskUserQuestionTool 现需 max_questions 字段(此前死信,现从
    // config 注入)。TUI 走 Default(protocol 默认上限),与 CLI 默认一致。
    tools.register(Arc::new(builtins::AskUserQuestionTool::default()));
    tools.register(Arc::new(builtins::AskUserTool));
    tools.register(Arc::new(builtins::ImageViewTool));

    // 3. Task / team 存储。
    let task_store: Arc<dyn reflect_task::TaskStore> = if args.ephemeral_tasks {
        Arc::new(reflect_task::InMemoryTaskStore::default())
    } else {
        match reflect_task::FileTaskStore::with_default_home() {
            Ok(s) => Arc::new(s),
            Err(e) => {
                tracing::warn!(error = %e, "FileTaskStore 失败,降级 InMemory");
                Arc::new(reflect_task::InMemoryTaskStore::default())
            }
        }
    };
    let team_store: Arc<dyn reflect_task::TeamStore> = if args.ephemeral_teams {
        Arc::new(reflect_task::InMemoryTeamStore::default())
    } else {
        match reflect_task::FileTeamStore::with_default_home() {
            Ok(s) => Arc::new(s),
            Err(e) => {
                tracing::warn!(error = %e, "FileTeamStore 失败,降级 InMemory");
                Arc::new(reflect_task::InMemoryTeamStore::default())
            }
        }
    };
    let task_manager = Arc::new(reflect_task::TaskManager::new(task_store, team_store));
    reflect_task::register_task_tools(&tools, task_manager.clone());

    // 4. Workspace + permissions。
    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let workspace = if args.auto_root {
        reflect_core::detect_project_root(&workspace)
    } else {
        workspace
    };

    // 权限链:config.toml 展平规则(内存)+ 文件 store(持久化,含运行期
    // always-allow 写入)。file_store Arc 保留给 TuiRuntimeHandles,M3 交互
    // 补全用;chained 喂给 resolver。
    let (persistent_permissions, permission_resolver) = match FilePermissionStore::with_default_home(
    ) {
        Ok(file_store) => {
            let file_store: Arc<dyn reflect_permissions::PermissionStore> = Arc::new(file_store);
            let cfg_store: Arc<dyn reflect_permissions::PermissionStore> =
                Arc::new(reflect_permissions::InMemoryPermissionStore::new());
            if let Some(sec) = &reflect_cfg.permissions {
                // 展平 deny + allow 紧凑数组 + 显式 rules(与 headless 路径对齐)。
                // 必须 `.await` —— `PermissionStore::add` 是 async trait 方法,
                // 不 await 得到的是未 poll 的 Future,规则不会写入 store,
                // 导致 resolver 查到空规则列表 → NoMatch → 审批框误弹。
                // 单条落库失败仅 warn,不阻塞启动(与 reflect-exec 路径一致)。
                for rule in &sec.expanded_rules() {
                    if let Err(e) = cfg_store.add(rule.clone()).await {
                        tracing::warn!(error = %e, "permission rule from config.toml 落库失败,跳过");
                    }
                }
            }
            let chained: Arc<dyn reflect_permissions::PermissionStore> =
                Arc::new(reflect_permissions::ChainedPermissionStore::new(vec![
                    cfg_store,
                    Arc::clone(&file_store),
                ]));
            let resolver: Arc<dyn reflect_permissions::PermissionResolver> =
                Arc::new(StorePermissionResolver::new(chained));
            (Some(file_store), Some(resolver))
        }
        Err(e) => {
            tracing::warn!(error = %e, "FilePermissionStore 未挂载,/permissions 引导");
            (None, None)
        }
    };

    // 5. Session recorder + M4 + M5。
    // M2 resume 闭环:三选一(`--resume <id>` / `-c` / `-r N`)命中时把
    // session_id 换成目标线程 —— recorder 续写原 JSONL,M4 note store 等
    // per-thread 路径全部随之对齐;历史消息 replay 后经 preload 注入引擎,
    // 并交给 TUI 回填渲染。
    let resume_target = resolve_resume_target(&args)?;
    let prior_messages = match resume_target {
        Some(tid) => {
            let records =
                reflect_rollout::reader::replay(&reflect_rollout::path::default_base(), tid)
                    .await?;
            let msgs = reflect_core::resume::records_to_preload(&records);
            tracing::info!(
                thread_id = %tid,
                messages = msgs.len(),
                "resuming session(--resume/-c/-r)"
            );
            msgs
        }
        None => Vec::new(),
    };
    let session_id = resume_target.unwrap_or_else(reflect_protocol::ThreadId::new);
    // recorder 提前创建：既要进 m4（父 agent 落库），又要 clone 给
    // SubAgentFactory（子 agent 的 Fork 记录 + per-child JSONL）。
    let recorder: Arc<dyn reflect_protocol::RolloutRecorder> = Arc::new(
        reflect_rollout::JsonlRolloutWriter::new(reflect_rollout::path::default_base(), session_id),
    );

    let telemetry = build_telemetry_sink(&reflect_cfg);
    // M4:agent 定义 / skills / notes / 恢复 / LLM 摘要压缩 / prompt 构建器。
    // memory(Composite:session 内存 + project/user 文件)、skills 工具、
    // note 工具均在此装配进 tools。
    let m4 = build_m4(
        &workspace,
        "default",
        &model,
        &registry,
        session_id,
        reflect_cfg.compact.trigger_tokens,
        &reflect_cfg,
        telemetry.clone(),
        recorder.clone(),
        &tools,
    );
    // 插件 skills 能力的落点(m4 在下方移入 AgentConfig,先留 clone;
    // 与 exec 的 `skills_for_plugins` 同款)。
    let skills_for_plugins = m4.skills.clone();

    // 会话取消令牌:engine(Interrupt 级联)与 factory(child_token 挂载)
    // 共享同一份;主循环退出时 cancel,TUI 关窗不留下飞子代理。
    let cancel = tokio_util::sync::CancellationToken::new();

    // M5:子代理工厂 + call_<role> 工具(child registry / 父 skills /
    // always-on 可见集在 build_m5 内对齐 exec)。
    let factory = build_m5(
        &workspace,
        "default",
        &model,
        &registry,
        session_id,
        recorder.clone(),
        &m4,
        &reflect_cfg,
        &tools,
    );
    factory.set_telemetry(telemetry.clone());
    // v1.4 A1:子代理运行注册表 + 会话令牌 —— 同一 Arc 双侧共享
    // (Op::Interrupt { child_id } 路由侧 / 工厂 spawn 登记侧)。
    let subagent_runtime = Arc::new(reflect_core::SubagentRuntimeRegistry::new());
    factory.set_runtime_registry(subagent_runtime.clone());
    factory.set_cancel(cancel.clone());

    // Coordinator 模式:team spec 注入 factory + prompt section + scratchpad。
    let coord_enabled = reflect_task::coordinator::CoordinatorConfig::from_env_or_config(
        reflect_cfg
            .coordinator
            .as_ref()
            .unwrap_or(&reflect_config::CoordinatorSection::default()),
    )
    .enabled;
    if coord_enabled {
        if let Err(e) = task_manager.sync_team_specs(&factory).await {
            tracing::warn!(error = %e, "coordinator: sync_team_specs 失败");
        }
    }
    apply_coordinator_from_config(&reflect_cfg, &workspace, Some(&m4), &factory, &tools);

    // 回合级装配:token 预算 / 最大迭代 / 配额跟踪 / 工具级 env。
    let token_budget = reflect_core::config::token_budget_from_env(
        reflect_cfg
            .token_budget
            .as_ref()
            .and_then(|s| s.session_total_tokens),
    );
    let max_iterations =
        reflect_core::config::max_iterations_from_env(reflect_cfg.active.max_iterations);
    let quota_tracker = build_quota_tracker(&reflect_cfg);
    // v1.5 R4:恢复上次的配额窗口(重启不静默重置;文件缺失 no-op)。
    if let Some(t) = &quota_tracker {
        t.load_state();
    }
    // web_search 工具级 env:`[web_search].api_key` → BRAVE_API_KEY。
    let mut tool_env = std::collections::HashMap::new();
    if let Some(ws) = &reflect_cfg.web_search {
        if let Some(key) = &ws.api_key {
            if !key.is_empty() {
                tool_env.insert("BRAVE_API_KEY".to_string(), key.clone());
            }
        }
    }

    // 6. AgentConfig + AgentThread(注入 sanitizer + hook engine)。
    let cfg = {
        let mut c = AgentConfig::new(model.clone(), workspace.clone())
            .with_approvals(true)
            .with_m4(m4)
            .with_policy(Arc::new(reflect_cfg.routing_policy()))
            .with_token_budget(token_budget)
            .with_max_iterations(max_iterations)
            .with_quota_tracker(quota_tracker)
            .with_tool_env(tool_env)
            .with_telemetry(telemetry)
            .with_cancel(cancel.clone())
            .with_subagent_runtime(subagent_runtime)
            .with_session_id(session_id)
            .with_context_window_overrides(
                reflect_cfg
                    .context_windows
                    .as_ref()
                    .map(|c| c.entries.clone())
                    .unwrap_or_default(),
            );
        // M2:resume 路径的历史预载(非 resume 时为空 Vec,no-op)。
        if !prior_messages.is_empty() {
            c = c.with_preload_messages(prior_messages.clone());
        }
        if let Some(resolver) = permission_resolver {
            c = c.with_permission_resolver(resolver);
        }
        // YOLO 分类器:与 headless 一致,PermissionMode::Bubble/Bypass 下
        // 由启发式判定哪些调用可静默放行。
        c.yolo_classifier = Some(Arc::new(reflect_permissions::HeuristicYoloClassifier));
        c
    };
    // registry / tools clone：AgentThread 持有 Arc clone。
    let thread = Arc::new(AgentThread::new(
        cfg,
        registry.clone(),
        tools.clone(),
        Some(sanitizer),
        Some(hook_engine),
    ));

    // 7. Plan mode hooks:PlanModeGate + read_before_edit。
    thread.register_hook(PlanModeGate::default_mode());
    let rbe_section = reflect_cfg.hooks.read_before_edit.as_ref();
    let (_rbe_state, rbe_hook) = build_read_before_edit(
        rbe_section.and_then(|s| s.enabled),
        rbe_section.and_then(|s| s.mtime_drift_tolerance_ms),
    );
    thread.register_hook(rbe_hook);

    // 8. --plan-mode CLI flag。
    if args.plan_mode {
        thread
            .config()
            .set_permission_mode(reflect_protocol::PermissionMode::Plan);
        tracing::info!("tui started with --plan-mode");
    }

    // 9. MCP / LSP bootstrap(失败仅 warn,不阻塞主流程);生命周期事件经
    // lifecycle channel 进 TUI 事件循环(/mcp overlay 与 Notice 行)。
    let (lifecycle_tx, lifecycle_rx) = tokio::sync::mpsc::channel::<reflect_protocol::Event>(64);
    let mcp_manager = bootstrap_mcp(&reflect_cfg, lifecycle_tx.clone(), tools.clone()).await;
    let _lsp_manager = bootstrap_lsp(&reflect_cfg, lifecycle_tx.clone(), tools.clone()).await;

    // MCP 资源 / Prompts 读取工具(与 exec 对齐;MCP 未启用时挂空
    // manager,工具自身对空列表返回友好结果)。v1.6 补 Prompts 原语。
    let mcp_for_resources = mcp_manager.clone().unwrap_or_else(|| {
        let (tx, _rx) = tokio::sync::mpsc::channel::<reflect_mcp::McpLifecycleEvent>(16);
        Arc::new(reflect_mcp::McpConnectionManager::new(tx))
    });
    tools.register(Arc::new(reflect_mcp::ListMcpResourcesTool::new(
        mcp_for_resources.clone(),
    )));
    tools.register(Arc::new(reflect_mcp::ReadMcpResourceTool::new(
        mcp_for_resources.clone(),
    )));
    tools.register(Arc::new(reflect_mcp::ListMcpPromptsTool::new(
        mcp_for_resources.clone(),
    )));
    tools.register(Arc::new(reflect_mcp::GetMcpPromptTool::new(
        mcp_for_resources,
    )));

    // 9.5 插件挂载(对齐 headless 的 `bootstrap_plugins`):把启用插件的
    // skills / agents / MCP servers / shell hooks / slash 命令挂进共享
    // registry。HOME 缺失或 plugins 目录不存在时返回空句柄,不阻塞启动。
    // 每个成功挂载的插件 emit `PluginLoaded` 走 lifecycle 通道 → 通知流。
    let mcp_for_plugins = mcp_manager.clone().unwrap_or_else(|| {
        let (tx, _rx) = tokio::sync::mpsc::channel::<reflect_mcp::McpLifecycleEvent>(16);
        Arc::new(reflect_mcp::McpConnectionManager::new(tx))
    });
    let plugin_runtime = reflect_plugin::runtime::bootstrap_plugins(
        tools.clone(),
        thread.hook_engine(),
        mcp_for_plugins,
        skills_for_plugins,
        factory,
        &reflect_cfg.plugins.enabled_plugins,
        Some(lifecycle_tx),
    )
    .await;

    // 10. 进入 Reflect 风格的 TUI 主循环，接入 agent thread。
    crate::tui::run_async(
        args,
        thread,
        TuiRuntimeHandles {
            lifecycle_rx,
            cancel,
            persistent_permissions,
            registry,
            config: reflect_cfg,
            prior_messages,
            plugin_runtime,
        },
    )
    .await
}

/// M2:resume 三选一解析(`--resume <id>` / `-c` / `-r N`)。
/// 与 exec 的 `resolve_resume_thread_id` 同语义;返回目标线程 id。
fn resolve_resume_target(args: &TuiArgs) -> Result<Option<reflect_protocol::ThreadId>> {
    if let Some(s) = &args.resume {
        let parsed = uuid::Uuid::parse_str(s)
            .map_err(|e| anyhow::anyhow!("invalid thread id '{s}': {e}"))?;
        return Ok(Some(reflect_protocol::ThreadId(parsed)));
    }
    if args.continue_last || args.resume_by.is_some() {
        let base = reflect_rollout::path::default_base();
        let tid = reflect_rollout::index::resolve_session_index(
            &base,
            args.continue_last,
            args.resume_by,
        )?;
        return Ok(Some(tid));
    }
    Ok(None)
}
