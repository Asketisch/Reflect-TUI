//! MCP 服务端 elicitation 页脚提示换行辅助。从 mcp_server_elicitation.rs 抽出。

use super::*;

pub(super) fn wrap_footer_tips(width: u16, tips: Vec<FooterTip>) -> Vec<Vec<FooterTip>> {
    let max_width = width.max(1) as usize;
    let separator_width = UnicodeWidthStr::width(FOOTER_SEPARATOR);
    if tips.is_empty() {
        return vec![Vec::new()];
    }

    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut used = 0usize;

    for tip in tips {
        let tip_width = UnicodeWidthStr::width(tip.text.as_str()).min(max_width);
        let extra = if current.is_empty() {
            tip_width
        } else {
            separator_width.saturating_add(tip_width)
        };
        if !current.is_empty() && used.saturating_add(extra) > max_width {
            lines.push(current);
            current = Vec::new();
            used = 0;
        }
        if current.is_empty() {
            used = tip_width;
        } else {
            used = used
                .saturating_add(separator_width)
                .saturating_add(tip_width);
        }
        current.push(tip);
    }

    if current.is_empty() {
        lines.push(Vec::new());
    } else {
        lines.push(current);
    }
    lines
}
