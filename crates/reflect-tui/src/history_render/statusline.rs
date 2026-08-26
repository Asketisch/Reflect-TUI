//! 富状态行(HUD):对齐老 TUI 底栏双行样式。
//!
//! - **identity 行**:`⠹ model · workspace git:(branch) · Context █░░░░░░░░░ 6%`
//!   (忙碌时前置 spinner)。
//! - **metrics 行**:`✓ Glob ×1 │ ✓ Bash ×1 │`(扫 history 最近工具调用)。
//!
//! 窄屏防御(老 TUI `hud::hud_height` 的核心修复点):
//! - `height >= 12` → 2 行(完整 HUD)
//! - `8 <= height < 12` → 1 行(只 identity,给 scrollback 让空间)
//! - `height < 8` → 0 行(不渲)
//!
//! 这是 Reflect 侧薄层:不改 Reflect,产 `Vec<Line>` 交给主循环画在视口底部。

use crate::adapter::UiHistoryItem;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use reflect_protocol::PermissionMode;

/// Braille spinner 帧(对齐老 TUI `widgets/spinner.rs`)。
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 工具统计最多展示的不同 tool 数(对齐老 TUI `MAX_TOOL_DISTINCT`)。
const MAX_TOOL_DISTINCT: usize = 8;

/// 取第 `tick` 帧的 spinner 字形(循环)。
pub fn spinner_frame(tick: u32) -> &'static str {
    SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()]
}

/// HUD 行数(窄屏防御):>=12→2,8-11→1,<8→0。
///
/// 输入是终端总高度(不是给 HUD 的剩余高度)。对齐老 TUI `hud::hud_height`。
/// 这是对老 TUI 窄屏 panic 的防御:`<8` 行时直接不渲,避免挤压 composer。
pub fn hud_height(area_height: u16) -> u16 {
    if area_height >= 12 {
        2
    } else if area_height >= 8 {
        1
    } else {
        0
    }
}

/// 上下文用量数据(由 `TokenCount` / `SessionConfigured` 事件填充)。
#[derive(Debug, Clone, Default)]
pub struct ContextUsage {
    /// 上下文窗口大小(token,做分母)。`None` → 省略上下文条。
    pub window_size: Option<u32>,
    /// 最近一次 token 用量(input + cached 做分子)。
    pub last_input_tokens: Option<u32>,
    pub last_cached_tokens: Option<u32>,
    /// 累计调用成本(USD)。`None` → 省略 cost 列。
    pub total_cost_usd: Option<f64>,
}

impl ContextUsage {
    /// 计算上下文百分比(0-100);缺数据 → `None`。
    fn percent(&self) -> Option<u8> {
        let window = self.window_size.filter(|w| *w > 0)?;
        let input = self.last_input_tokens?;
        let cached = self.last_cached_tokens.unwrap_or(0);
        let numerator = (input as u64).saturating_add(cached as u64);
        let pct = numerator * 100 / window as u64;
        Some(pct.min(100) as u8)
    }
}

/// 渲染 identity 行(第 1 行)。
///
/// - `busy` / `tick`:忙碌时前置 spinner。
/// - `model`:模型名。
/// - `cwd`:工作目录基名。
/// - `git_branch`:当前 git 分支(非仓库 → `None`)。
/// - `approval_pending`:审批挂起时附 `[y/n]`。
/// - `usage`:上下文用量(有数据时附 `Context █░░ 6%`)。
/// - `permission_mode`:会话级权限模式(非 `Auto` 时附 `[plan]` / `[accept_edits]` …)。
pub fn render_identity_line(
    busy: bool,
    tick: u32,
    model: &str,
    cwd: &str,
    git_branch: Option<&str>,
    approval_pending: bool,
    usage: &ContextUsage,
    vim_enabled: bool,
    permission_mode: PermissionMode,
    plan_enter_pending: bool,
) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let brand = Style::default().fg(Color::Cyan);
    let mut spans: Vec<Span<'static>> = Vec::new();

    if busy {
        spans.push(Span::styled(
            format!("{} ", spinner_frame(tick)),
            brand.add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(Span::styled(model.to_string(), brand));
    spans.push(Span::styled(" · ", dim));

    if let Some(branch) = git_branch {
        spans.push(Span::styled(cwd.to_string(), dim));
        spans.push(Span::styled(" git:(", dim));
        spans.push(Span::styled(
            branch.to_string(),
            Style::default().fg(Color::Yellow),
        ));
        spans.push(Span::styled(")", dim));
    } else {
        spans.push(Span::styled(cwd.to_string(), dim));
    }

    // 上下文用量条(有数据才显示)。
    if let Some(seg) = context_bar_span(usage) {
        spans.push(Span::styled(" · ", dim));
        spans.push(seg);
    }

    // Cost 列(有数据才显示):`$0.012`(暗灰,与上下文条类似)。
    if let Some(cost) = usage.total_cost_usd {
        spans.push(Span::styled(" · ", dim));
        spans.push(Span::styled(
            format!("${:.4}", cost),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
    }

    if approval_pending {
        spans.push(Span::styled("  [y/n]", Style::default().fg(Color::Yellow)));
    }
    // v1.x Plan mode:进入确认挂起时附 `[1]enter/[3]cancel` 提示,与 approval 的
    // [y/n] 对称,让用户知道有 plan-enter 决策待处理(主循环里按 1/3/Esc 回送)。
    if plan_enter_pending {
        spans.push(Span::styled(
            "  Enter plan? [1]yes/[3]no",
            Style::default().fg(Color::Yellow),
        ));
    }

    // Vim 模式指示:仅在启用时显示(`[NORMAL]` 暗黄粗体),与老 TUI `-- NORMAL --`
    // 视觉一致;关闭时不显示,避免与默认 composer 状态混淆。
    if vim_enabled {
        spans.push(Span::styled("  ", dim));
        spans.push(Span::styled(
            "[NORMAL]",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // 会话级权限模式:`Auto`(默认)省略,其余都附 `[mode]` 段。
    // Plan = 品红(最显眼)、AcceptEdits = 青、Bypass = 红、Prompt/Deny/Bubble = 暗黄。
    if let Some(seg) = permission_mode_span(permission_mode) {
        spans.push(Span::styled("  ", dim));
        spans.push(seg);
    }

    // 退出提示:弱可见性(dim),让用户能发现退出键(Ctrl+C / Ctrl+D)。
    // 审批挂起时省略,避免与上方 `[y/n]` 抢占注意力。
    if !approval_pending {
        spans.push(Span::styled("  ", dim));
        spans.push(Span::styled(
            "Ctrl+C to quit",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ));
    }

    Line::from(spans)
}

/// 权限模式段 span。`Auto`(默认)→ `None`(省略)。
fn permission_mode_span(mode: PermissionMode) -> Option<Span<'static>> {
    let (label, color) = match mode {
        PermissionMode::Auto => return None,
        PermissionMode::Plan => ("[plan]", Color::Magenta),
        PermissionMode::AcceptEdits => ("[accept_edits]", Color::Cyan),
        PermissionMode::Bypass => ("[bypass]", Color::Red),
        PermissionMode::Prompt => ("[prompt]", Color::Yellow),
        PermissionMode::Deny => ("[deny]", Color::Red),
        PermissionMode::Bubble => ("[bubble]", Color::Blue),
    };
    Some(Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

/// 上下文用量条 span:`Context ████░░░░░░ 42%`(阈值色 + BOLD)。
/// 缺数据 → `None`(调用方省略)。
fn context_bar_span(usage: &ContextUsage) -> Option<Span<'static>> {
    let pct = usage.percent()?;
    let filled = ((pct as f64 / 10.0).round() as u8).min(10) as usize;
    let bar: String = "█".repeat(filled) + &"░".repeat(10 - filled);
    let color = if pct >= 85 {
        Color::Red
    } else if pct >= 70 {
        Color::Yellow
    } else {
        Color::Green
    };
    Some(Span::styled(
        format!("Context {bar} {pct}%"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

/// 渲染 metrics 行(第 2 行):扫 history 最近的工具调用统计。
///
/// 输出 `✓ name ×N │ ✗ name ×M │`(倒序,最多 `MAX_TOOL_DISTINCT` 个不同工具)。
/// 无工具调用 → dim 占位 `(no tool calls yet)`。
pub fn render_metrics_line(history: &[UiHistoryItem]) -> Line<'static> {
    let segments = collect_recent_tools(history);
    let dim = Style::default().fg(Color::DarkGray);

    if segments.is_empty() {
        return Line::from(Span::styled(" (no tool calls yet)", dim));
    }

    let mut spans: Vec<Span<'static>> = Vec::new();
    for seg in &segments {
        let color = if seg.failed > 0 {
            Color::Red
        } else {
            Color::Green
        };
        spans.push(Span::styled(
            format!("{} {} ×{}", seg.marker, seg.name, seg.total),
            Style::default().fg(color),
        ));
        spans.push(Span::styled(" │ ", dim));
    }
    Line::from(spans)
}

#[derive(Debug)]
struct ToolSegment {
    name: String,
    total: u32,
    failed: u32,
    marker: &'static str,
}

/// 扫 history(倒序)聚合工具调用,最多 `MAX_TOOL_DISTINCT` 个不同工具。
/// 对齐老 TUI `hud::metrics::collect_recent_tools`。
fn collect_recent_tools(history: &[UiHistoryItem]) -> Vec<ToolSegment> {
    use std::collections::HashMap;

    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<String, (u32, u32)> = HashMap::new();
    for row in history.iter().rev() {
        if order.len() >= MAX_TOOL_DISTINCT {
            break;
        }
        if let UiHistoryItem::ToolCall { name, error, .. } = row {
            if name.is_empty() {
                continue;
            }
            let entry = counts.entry(name.clone()).or_insert((0, 0));
            entry.0 += 1;
            if *error {
                entry.1 += 1;
            }
            if !order.contains(name) {
                order.push(name.clone());
            }
        }
    }
    order
        .into_iter()
        .map(|name| {
            let (total, failed) = counts.remove(&name).unwrap_or((0, 0));
            ToolSegment {
                name,
                total,
                failed,
                marker: if failed > 0 { "✗" } else { "✓" },
            }
        })
        .collect()
}

/// 启动期一次性探测当前 git 分支(best-effort;非仓库/无 git → `None`)。
///
/// 对齐老 TUI `current_git_branch`:`git rev-parse --abbrev-ref HEAD`,
/// `HEAD`(detached)/空/失败 → `None`。
pub fn detect_git_branch() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "--quiet", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if b.is_empty() || b == "HEAD" {
        None
    } else {
        Some(b)
    }
}

// ── 向后兼容:旧 `render_status_line` 单行入口(无上下文条) ──────────────
/// 渲染状态行(单行,无上下文条;保留供旧调用点/测试)。
#[allow(dead_code)]
pub fn render_status_line(
    busy: bool,
    tick: u32,
    model: &str,
    cwd: &str,
    git_branch: Option<&str>,
    approval_pending: bool,
    vim_enabled: bool,
) -> Line<'static> {
    render_identity_line(
        busy,
        tick,
        model,
        cwd,
        git_branch,
        approval_pending,
        &ContextUsage::default(),
        vim_enabled,
        PermissionMode::default(),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_cycles() {
        assert_eq!(spinner_frame(0), "⠋");
        assert_eq!(spinner_frame(1), "⠙");
        assert_eq!(spinner_frame(10), "⠋"); // wraps
    }

    #[test]
    fn hud_height_thresholds() {
        assert_eq!(hud_height(0), 0, "<8 → 0");
        assert_eq!(hud_height(7), 0, "<8 → 0");
        assert_eq!(hud_height(8), 1, "8 → 1");
        assert_eq!(hud_height(11), 1, "11 → 1");
        assert_eq!(hud_height(12), 2, "12 → 2");
        assert_eq!(hud_height(24), 2, "24 → 2");
    }

    #[test]
    fn identity_line_shows_model_and_cwd() {
        let line = render_identity_line(
            false,
            0,
            "reflect",
            "myproj",
            None,
            false,
            &ContextUsage::default(),
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("reflect"), "model: {s}");
        assert!(s.contains("myproj"), "cwd: {s}");
        assert!(!s.contains("git:"));
        assert!(!s.contains("Context"));
        assert!(!s.contains("[NORMAL]"), "vim off 时不显示 [NORMAL]");
        assert!(!s.contains("[plan]"), "Auto 不显示 mode 段");
    }

    #[test]
    fn identity_line_with_git_branch() {
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            Some("main"),
            false,
            &ContextUsage::default(),
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("git:(main)"), "git branch: {s}");
    }

    #[test]
    fn identity_line_busy_shows_spinner() {
        let line = render_identity_line(
            true,
            2,
            "m",
            "p",
            None,
            false,
            &ContextUsage::default(),
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.starts_with("⠹ "), "busy spinner: {s}");
    }

    #[test]
    fn identity_line_approval_hint() {
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            true,
            &ContextUsage::default(),
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("[y/n]"), "approval hint: {s}");
    }

    #[test]
    fn context_bar_omitted_without_data() {
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            false,
            &ContextUsage::default(),
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(!s.contains("Context"), "no context bar without data: {s}");
    }

    #[test]
    fn context_bar_shown_with_usage() {
        let usage = ContextUsage {
            window_size: Some(100_000),
            last_input_tokens: Some(6_000),
            last_cached_tokens: Some(0),
            total_cost_usd: None,
        };
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            false,
            &usage,
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("Context"), "context bar: {s}");
        assert!(s.contains("6%"), "6% usage: {s}");
        assert!(s.contains("█") || s.contains("░"), "bar glyphs: {s}");
    }

    #[test]
    fn identity_line_shows_mode_segment_for_plan() {
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            false,
            &ContextUsage::default(),
            false,
            PermissionMode::Plan,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("[plan]"), "Plan 模式应显示 [plan] 段: {s}");
    }

    #[test]
    fn identity_line_omits_mode_segment_for_auto() {
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            false,
            &ContextUsage::default(),
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(!s.contains("[plan]"), "Auto 不应显示 mode 段: {s}");
        assert!(!s.contains("[bypass]"));
    }

    #[test]
    fn identity_line_vim_normal_indicator() {
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            false,
            &ContextUsage::default(),
            true,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("[NORMAL]"), "vim 启用时显示 [NORMAL]: {s}");
        // [NORMAL] 不再是行尾 —— 退出提示在其后。
        assert!(!s.ends_with("[NORMAL]"), "[NORMAL] 不应在行尾(后有 quit 提示): {s}");
    }

    #[test]
    fn identity_line_shows_quit_hint() {
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            false,
            &ContextUsage::default(),
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("Ctrl+C to quit"), "应有退出提示: {s}");
    }

    #[test]
    fn identity_line_omits_quit_hint_when_approval_pending() {
        let line = render_identity_line(
            false,
            0,
            "m",
            "p",
            None,
            true, // 审批挂起
            &ContextUsage::default(),
            false,
            PermissionMode::Auto,
            false,
        );
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(!s.contains("Ctrl+C to quit"), "审批挂起时省略 quit 提示: {s}");
        assert!(s.contains("[y/n]"), "审批挂起应显示 [y/n]: {s}");
    }

    #[test]
    fn context_bar_threshold_colors() {
        // 85% → 红，70% → 黄，<70 → 绿（通过 percent + 条形字形存在性验证）。
        let pct85 = ContextUsage {
            window_size: Some(100),
            last_input_tokens: Some(85),
            last_cached_tokens: Some(0),
            total_cost_usd: None,
        };
        let pct60 = ContextUsage {
            window_size: Some(100),
            last_input_tokens: Some(60),
            last_cached_tokens: Some(0),
            total_cost_usd: None,
        };
        assert_eq!(pct85.percent(), Some(85));
        assert_eq!(pct60.percent(), Some(60));
    }

    #[test]
    fn metrics_line_empty_shows_placeholder() {
        let line = render_metrics_line(&[]);
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("no tool calls yet"), "placeholder: {s}");
    }

    #[test]
    fn metrics_line_aggregates_tools() {
        let history = vec![
            UiHistoryItem::ToolCall {
                name: "Bash".into(),
                output: String::new(),
                error: false,
                elapsed_ms: 1,
                diff: None,
            },
            UiHistoryItem::ToolCall {
                name: "Bash".into(),
                output: String::new(),
                error: false,
                elapsed_ms: 2,
                diff: None,
            },
            UiHistoryItem::ToolCall {
                name: "Glob".into(),
                output: String::new(),
                error: false,
                elapsed_ms: 3,
                diff: None,
            },
        ];
        let line = render_metrics_line(&history);
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("✓ Bash ×2"), "aggregated bash: {s}");
        assert!(s.contains("✓ Glob ×1"), "glob: {s}");
    }

    #[test]
    fn metrics_line_marks_failed_tools() {
        let history = vec![UiHistoryItem::ToolCall {
            name: "Bash".into(),
            output: String::new(),
            error: true,
            elapsed_ms: 1,
            diff: None,
        }];
        let line = render_metrics_line(&history);
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("✗ Bash ×1"), "failed marker: {s}");
    }

    #[test]
    fn metrics_line_caps_distinct_tools() {
        // 12 个不同工具,应只显示前 MAX_TOOL_DISTINCT(8)个。
        let history: Vec<UiHistoryItem> = (0..12)
            .map(|i| UiHistoryItem::ToolCall {
                name: format!("Tool{i}"),
                output: String::new(),
                error: false,
                elapsed_ms: 1,
                diff: None,
            })
            .collect();
        let line = render_metrics_line(&history);
        let count = line
            .spans
            .iter()
            .filter(|s| s.content.starts_with("✓"))
            .count();
        assert_eq!(count, MAX_TOOL_DISTINCT, "capped to 8 tools");
    }

    #[test]
    fn metrics_line_ignores_non_tool_items() {
        let history = vec![
            UiHistoryItem::User("hi".into()),
            UiHistoryItem::Agent("hello".into()),
            UiHistoryItem::Notice("n".into()),
        ];
        let line = render_metrics_line(&history);
        let s: String = line.spans.iter().map(|s| s.content.clone()).collect();
        assert!(s.contains("no tool calls yet"), "ignores non-tool: {s}");
    }
}
