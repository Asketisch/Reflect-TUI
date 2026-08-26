//! 外部 agent 配置迁移 的测试集。
//!
//! 从 external_agent_config_migration/mod.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::ExternalAgentConfigMigrationOutcome;
use super::ExternalAgentConfigMigrationScreen;
use super::MigrationView;
use crate::app_server_protocol::ExternalAgentConfigMigrationItem;
use crate::app_server_protocol::ExternalAgentConfigMigrationItemType;
use crate::app_server_protocol::PluginsMigration;
use crate::app_server_protocol::SessionMigration;
use crate::tui_core::custom_terminal::Terminal;
use crate::tui_core::test_backend::VT100Backend;
use crate::tui_core::tui::FrameRequester;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use insta::assert_snapshot;
use pretty_assertions::assert_eq;
use ratatui::layout::Rect;
use std::path::PathBuf;

fn sample_plugin_details() -> crate::app_server_protocol::MigrationDetails {
    crate::app_server_protocol::MigrationDetails {
        plugins: vec![
            PluginsMigration {
                marketplace_name: "acme-tools".to_string(),
                plugin_names: vec![
                    "deployer".to_string(),
                    "formatter".to_string(),
                    "lint".to_string(),
                ],
            },
            PluginsMigration {
                marketplace_name: "team-marketplace".to_string(),
                plugin_names: vec!["asana".to_string()],
            },
            PluginsMigration {
                marketplace_name: "debug".to_string(),
                plugin_names: vec!["sample".to_string()],
            },
            PluginsMigration {
                marketplace_name: "data-tools".to_string(),
                plugin_names: vec!["warehouse".to_string()],
            },
        ],
        ..Default::default()
    }
}

#[cfg(windows)]
fn sample_project_root() -> PathBuf {
    PathBuf::from(r"C:\workspace\project")
}

#[cfg(not(windows))]
fn sample_project_root() -> PathBuf {
    PathBuf::from("/workspace/project")
}

fn sample_project_path(path: &str) -> String {
    sample_project_root().join(path).display().to_string()
}

fn sample_items() -> Vec<ExternalAgentConfigMigrationItem> {
    let project_root = sample_project_root();
    vec![
        ExternalAgentConfigMigrationItem {
            item_type: ExternalAgentConfigMigrationItemType::Config,
            description:
                "Migrate /Users/alex/.claude/settings.json into /Users/alex/.reflect/config.toml"
                    .to_string(),
            cwd: None,
            details: None,
        },
        ExternalAgentConfigMigrationItem {
            item_type: ExternalAgentConfigMigrationItemType::Sessions,
            description: "Migrate recent external agent sessions".to_string(),
            cwd: None,
            details: Some(crate::app_server_protocol::MigrationDetails {
                sessions: vec![SessionMigration {
                    path: PathBuf::from("/Users/alex/.claude/projects/project/session.jsonl"),
                    cwd: project_root.clone(),
                    title: Some("Investigate migration UX".to_string()),
                }],
                ..Default::default()
            }),
        },
        ExternalAgentConfigMigrationItem {
            item_type: ExternalAgentConfigMigrationItemType::Plugins,
            description: format!(
                "Migrate enabled plugins from {}",
                sample_project_path(".claude/settings.json")
            ),
            cwd: Some(project_root.clone()),
            details: Some(sample_plugin_details()),
        },
        ExternalAgentConfigMigrationItem {
            item_type: ExternalAgentConfigMigrationItemType::AgentsMd,
            description: format!(
                "Migrate {} to {}",
                sample_project_path("CLAUDE.md"),
                sample_project_path("AGENTS.md")
            ),
            cwd: Some(project_root),
            details: None,
        },
    ]
}

fn sample_items_with_memory() -> Vec<ExternalAgentConfigMigrationItem> {
    let mut items = sample_items();
    items.insert(
            1,
            ExternalAgentConfigMigrationItem {
                item_type: ExternalAgentConfigMigrationItemType::Memory,
                description: "Migrate memory files from /Users/alex/.claude/projects to /Users/alex/.reflect/memories/extensions/external_agent_import/resources".to_string(),
                cwd: None,
                details: Some(crate::app_server_protocol::MigrationDetails {
                    memory: vec!["project".to_string()],
                    ..Default::default()
                }),
            },
        );
    items
}

fn render_screen(screen: &ExternalAgentConfigMigrationScreen, width: u16, height: u16) -> String {
    let backend = VT100Backend::new(width, height);
    let mut terminal = Terminal::with_options(backend).expect("terminal");
    terminal.set_viewport_area(Rect::new(0, 0, width, height));
    {
        let mut frame = terminal.get_frame();
        frame.render_widget_ref(screen, frame.area());
    }
    terminal.flush().expect("flush");
    terminal
        .backend()
        .to_string()
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn prompt_snapshot() {
    let items = sample_items_with_memory();
    let screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );

    let rendered = render_screen(&screen, /*width*/ 80, /*height*/ 24);
    #[cfg(windows)]
    assert_snapshot!("external_agent_config_migration_prompt_windows", rendered);
    #[cfg(not(windows))]
    assert_snapshot!("external_agent_config_migration_prompt", rendered);
}

#[test]
fn customize_snapshot() {
    let items = sample_items_with_memory();
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );
    screen.customize();

    let rendered = render_screen(&screen, /*width*/ 80, /*height*/ 34);
    #[cfg(windows)]
    assert_snapshot!(
        "external_agent_config_migration_customize_windows",
        rendered
    );
    #[cfg(not(windows))]
    assert_snapshot!("external_agent_config_migration_customize", rendered);
}

#[test]
fn secondary_source_customize_snapshot() {
    let items = vec![ExternalAgentConfigMigrationItem {
        item_type: ExternalAgentConfigMigrationItemType::Config,
        description:
            "Migrate /Users/alex/.cursor/cli-config.json into /Users/alex/.reflect/config.toml"
                .to_string(),
        cwd: None,
        details: None,
    }];
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );
    screen.customize();

    let rendered = render_screen(&screen, /*width*/ 80, /*height*/ 18);
    assert_snapshot!(
        "external_agent_config_migration_secondary_source_customize",
        rendered
    );
}

#[test]
fn customize_action_snapshot() {
    let items = sample_items();
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );
    screen.customize();
    screen.move_up();

    let rendered = render_screen(&screen, /*width*/ 80, /*height*/ 30);
    #[cfg(windows)]
    assert_snapshot!(
        "external_agent_config_migration_customize_action_windows",
        rendered
    );
    #[cfg(not(windows))]
    assert_snapshot!("external_agent_config_migration_customize_action", rendered);
}

#[test]
fn proceed_returns_selected_items() {
    let items = sample_items();
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );

    screen.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(screen.is_done());
    assert_eq!(
        screen.outcome(),
        ExternalAgentConfigMigrationOutcome::Proceed(items)
    );
}

#[test]
fn toggle_item_then_proceed_keeps_remaining_selection() {
    let items = sample_items();
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );

    screen.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    screen.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    screen.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    screen.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));

    assert!(screen.is_done());
    assert_eq!(
        screen.outcome(),
        ExternalAgentConfigMigrationOutcome::Proceed(vec![
            items[1].clone(),
            items[2].clone(),
            items[3].clone(),
        ])
    );
}

#[test]
fn escape_skips_prompt() {
    let items = sample_items();
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );

    screen.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(screen.is_done());
    assert_eq!(screen.outcome(), ExternalAgentConfigMigrationOutcome::Skip);
}

#[test]
fn numeric_shortcuts_follow_visible_actions_when_proceed_is_disabled() {
    let items = sample_items();
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );

    screen.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    screen.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    screen.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    screen.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));

    assert_eq!(screen.view, MigrationView::Customize);
}

#[test]
fn empty_selection_enter_opens_customize_instead_of_proceeding() {
    let items = sample_items();
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &[],
        /*error*/ None,
    );

    screen.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(!screen.is_done());
    assert_eq!(screen.view, MigrationView::Customize);
}

#[test]
fn control_exit_shortcuts_cancel_prompt() {
    let items = sample_items();
    for key_code in [KeyCode::Char('c'), KeyCode::Char('d')] {
        let mut screen = ExternalAgentConfigMigrationScreen::new(
            FrameRequester::test_dummy(),
            &items,
            &items,
            /*error*/ None,
        );

        screen.handle_key(KeyEvent::new(key_code, KeyModifiers::CONTROL));

        assert!(screen.is_done());
        assert_eq!(screen.outcome(), ExternalAgentConfigMigrationOutcome::Skip);
    }
}

#[test]
fn numeric_shortcuts_choose_actions() {
    let items = sample_items();

    let mut proceed_screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );
    proceed_screen.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    assert_eq!(
        proceed_screen.outcome(),
        ExternalAgentConfigMigrationOutcome::Proceed(items.clone())
    );

    let mut customize_screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );
    customize_screen.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    assert_eq!(customize_screen.view, MigrationView::Customize);
    customize_screen.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    assert_eq!(customize_screen.view, MigrationView::Summary);

    let mut skip_screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );
    skip_screen.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
    assert_eq!(
        skip_screen.outcome(),
        ExternalAgentConfigMigrationOutcome::Skip
    );
}

#[test]
fn summary_does_not_toggle_selection() {
    let items = sample_items();
    let mut screen = ExternalAgentConfigMigrationScreen::new(
        FrameRequester::test_dummy(),
        &items,
        &items,
        /*error*/ None,
    );

    screen.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));

    assert_eq!(screen.selected_items(), items);
}
