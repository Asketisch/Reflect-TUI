//! Agent 待办清单的界面呈现(v1.x `TodoWrite` 接线)。
//!
//! 背景:submodule 的 `TodoWrite` 工具把清单写进 `ToolOutput.metadata.todos`,
//! core 的 `tool_exec` 节点把它渲染成紧凑文本存进**会话级**槽
//! (`AgentConfig::todos_slot`),`pre_loop` 再把该槽注入进模型上下文 ——
//! 清单因此「只出不进」不再是问题。
//!
//! 但 TUI 此前**完全看不到**这份清单:模型写完 todo、清单也回灌进了上下文,
//! 用户界面里却一个字都没有(没有 Codex / Claude 那样的 checklist 面板)。
//! 本模块把会话级槽里的紧凑清单渲染成人读形式。
//!
//! 清单文本格式(core `render_todo_checklist` 产出):
//! ```text
//! - [x] 已完成项
//! - [~] 进行中项
//! - [ ] 待办项
//! ```

/// 清单的计数统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TodoCounts {
    pub completed: usize,
    pub in_progress: usize,
    pub pending: usize,
}

impl TodoCounts {
    /// 清单总条目数。
    pub fn total(&self) -> usize {
        self.completed + self.in_progress + self.pending
    }
}

/// 统计清单里的条目数(按 `- [x]` / `- [~]` / `- [ ]` 前缀)。
pub fn count_todos(raw: &str) -> TodoCounts {
    let mut c = TodoCounts::default();
    for line in raw.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("- [") else {
            continue;
        };
        match rest.as_bytes().first() {
            Some(b'x') => c.completed += 1,
            Some(b'~') => c.in_progress += 1,
            Some(b' ') => c.pending += 1,
            _ => {}
        }
    }
    c
}

/// 把清单渲染成界面文本。
///
/// `None` = 没有清单(还没写过 todo)→ 给出明确说明而不是空串。
pub fn format_checklist(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let c = count_todos(raw);
    let mut out = format!(
        "/todo: {} 项(✅ {} 完成 · 🔄 {} 进行中 · ⬜ {} 待办)",
        c.total(),
        c.completed,
        c.in_progress,
        c.pending
    );
    for line in trimmed.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        out.push_str(&format!("\n  {t}"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "- [x] 读需求\n- [~] 写实现\n- [ ] 补测试\n";

    #[test]
    fn counts_marks_by_state() {
        let c = count_todos(SAMPLE);
        assert_eq!(c.completed, 1);
        assert_eq!(c.in_progress, 1);
        assert_eq!(c.pending, 1);
        assert_eq!(c.total(), 3);
    }

    #[test]
    fn empty_raw_has_no_checklist() {
        assert!(format_checklist("").is_none());
        assert!(format_checklist("   \n  ").is_none());
    }

    #[test]
    fn checklist_shows_summary_and_items() {
        let s = format_checklist(SAMPLE).unwrap();
        assert!(s.contains("3 项"));
        assert!(s.contains("1 完成"));
        assert!(s.contains("1 进行中"));
        assert!(s.contains("1 待办"));
        assert!(s.contains("- [x] 读需求"));
        assert!(s.contains("- [ ] 补测试"));
    }

    #[test]
    fn unknown_marks_are_not_counted() {
        let c = count_todos("- [?] weird\n- [x] ok\n");
        assert_eq!(c.completed, 1);
        assert_eq!(c.total(), 1);
    }

    #[test]
    fn lines_without_prefix_are_ignored() {
        let c = count_todos("random text\n- [x] ok\n");
        assert_eq!(c.completed, 1);
        assert_eq!(c.total(), 1);
    }
}
