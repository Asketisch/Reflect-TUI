//! 稳定 Reflect 风格 TUI 的 Reflect runtime 引导启动。
//!
//! 加载 config,构建 model registry / provider / tools / AgentThread,
//! 然后在 `tui::run_with_thread` 中进入 Reflect 风格的聊天循环。

use crate::TuiArgs;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use reflect_core::{AgentConfig, AgentThread};
use reflect_hooks::builtins::{PlanModeGate, build_read_before_edit};
use reflect_llm::ModelRegistry;
use reflect_permissions::{FilePermissionStore, StorePermissionResolver};
use reflect_tools::{ToolRegistry, builtins};

/// 加载 config + 构建 Reflect runtime,然后进入 TUI 主循环。
pub fn run(args: TuiArgs) -> Result<()> {
    crate::logging::init();

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
    let model = format!("{}/{}", provider, reflect_cfg.model_for(provider));

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

    // 4. Workspace + memory + permissions。
    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let workspace = if args.auto_root {
        reflect_core::detect_project_root(&workspace)
    } else {
        workspace
    };
    let home_path = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let memory_store: Arc<dyn reflect_memory::MemoryStore> =
        Arc::new(reflect_memory::FileMemoryStore::new(&workspace, &home_path));

    let (_permission_store, permission_resolver) = match FilePermissionStore::with_default_home() {
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
                Arc::new(StorePermissionResolver::new(Arc::clone(&chained)));
            (Some(chained), Some(resolver))
        }
        Err(e) => {
            tracing::warn!(error = %e, "FilePermissionStore 未挂载,/permissions 引导");
            (None, None)
        }
    };

    // 5. Session recorder + AgentConfig。
    let session_id = reflect_protocol::ThreadId::new();
    // recorder 提前创建：既要进 m4（父 agent 落库），又要 clone 给
    // SubAgentFactory（子 agent 的 Fork 记录 + per-child JSONL）。
    let recorder: Arc<dyn reflect_protocol::RolloutRecorder> = Arc::new(
        reflect_rollout::JsonlRolloutWriter::new(
            reflect_rollout::path::default_base(),
            session_id,
        ),
    );
    let cfg = {
        let mut m4 = reflect_core::config::default_m4_deps("default");
        m4.recorder = Some(recorder.clone());
        m4.memory = memory_store.clone();
        let mut c = AgentConfig::new(model.clone(), workspace.clone())
            .with_approvals(true)
            .with_m4(m4)
            .with_session_id(session_id)
            .with_context_window_overrides(
                reflect_cfg
                    .context_windows
                    .as_ref()
                    .map(|c| c.entries.clone())
                    .unwrap_or_default(),
            );
        if let Some(resolver) = permission_resolver.clone() {
            c = c.with_permission_resolver(resolver);
        }
        c
    };

    // 6. AgentThread + hooks。
    // v1.x:AgentThread::new 新增 sanitizer + hook_engine 两参(把 config.toml
    // [sanitize]/[hooks] 真正接进 queue)。TUI 暂传 None:sanitizer 回退
    // with_defaults,hook_engine 回退空 engine —— 与历史行为一致,后续可按需注入。
    // registry / tools clone：AgentThread 持有 Arc clone；保留原 Arc 供下方
    // SubAgentFactory（共享同一 ToolRegistry，注册的 call_<role> 对 agent 立即可见）。
    let thread = Arc::new(AgentThread::new(cfg, registry.clone(), tools.clone(), None, None));
    thread.register_hook(PlanModeGate::default_mode());
    let rbe_section = reflect_cfg.hooks.read_before_edit.as_ref();
    let (_rbe_state, rbe_hook) = build_read_before_edit(
        rbe_section.and_then(|s| s.enabled),
        rbe_section.and_then(|s| s.mtime_drift_tolerance_ms),
    );
    thread.register_hook(rbe_hook);

    // 7. --plan-mode CLI flag。
    if args.plan_mode {
        thread
            .config()
            .set_permission_mode(reflect_protocol::PermissionMode::Plan);
        tracing::info!("tui started with --plan-mode");
    }

    // 8. 接入 SubAgentFactory —— 让 TUI agent 也能派发子 agent (call_<role>)。
    //    此前仅 headless 路径 (reflect-exec) 注册了 SubAgentFactory，TUI agent
    //    工具表里没有 call_explorer，用户要求"派发 subagent"时模型反馈"无此工具"。
    //    这里与 reflect-exec::build_m5_surface 对齐：合并 home/ws/toml spec，
    //    全空则硬编码 explorer 兜底。subagent_registry 保持 None ——
    //    CallSubAgentTool::execute 在 None 时走 `if let Some` 安全跳过 record
    //    （已由其单测 execute_works_without_registry 覆盖）。
    {
        use reflect_subagent::{
            load_subagents_dir, merge_by_priority, CallSubAgentTool, SubAgentFactory, SubAgentSpec,
        };
        use tokio_util::sync::CancellationToken;
        let factory = Arc::new(SubAgentFactory::new(
            session_id,
            model.clone(),
            registry.clone(),
            // child_registry = None：父子共享同一 ModelRegistry（默认行为）。
            None,
            tools.clone(),
            CancellationToken::new(),
            Some(recorder.clone()),
        ));
        // 合并 spec：TOML [[subagents]] > workspace .reflect/subagents/*.md
        // > home ~/.reflect/subagents/*.md > 硬编码 explorer(全部为空时)。
        let home_md = load_subagents_dir(&home_path.join(".reflect").join("subagents"))
            .unwrap_or_default();
        let ws_md = load_subagents_dir(&workspace.join(".reflect").join("subagents"))
            .unwrap_or_default();
        let toml_specs = reflect_cfg.subagents.clone();
        let mut configs = merge_by_priority(vec![home_md, ws_md, toml_specs]);
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
        for sc in configs {
            let spec = SubAgentSpec {
                name: sc.name,
                role: sc.role.clone(),
                model: sc.model,
                system_prompt: sc.system_prompt,
                allowed_tools: sc.allowed_tools,
                data_transfer: Default::default(),
                max_turns: sc.max_turns,
                allowed_skills: sc.allowed_skills,
            };
            let tool: Arc<dyn reflect_tools::Tool> =
                Arc::new(CallSubAgentTool::new(factory.clone(), spec));
            tools.register_runtime_tool(tool);
        }
    }

    // 9. 进入 Reflect 风格的 TUI 主循环，接入 agent thread。
    crate::tui::run_async(args, thread).await
}
