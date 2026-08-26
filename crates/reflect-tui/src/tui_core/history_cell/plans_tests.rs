use super::*;
use pretty_assertions::assert_eq;

#[test]
fn finalized_plan_reuses_lines_primed_by_transcript_height() {
    let cell = new_proposed_plan("1. Inspect **markdown**".to_string(), Path::new("/tmp"));
    let width = 48;

    // 标题 + 空白分隔行 + 正文 + 尾部空行。移除重复的顶部内边距空行后，
    // 行数从 8 降至 6（此前两个仅含空白的行使计数虚增）。
    assert_eq!(cell.desired_transcript_height(width), 6);
    cell.rendered_lines
        .cached
        .lock()
        .expect("render cache lock")
        .as_mut()
        .expect("render cache should be populated")
        .1 = vec![HyperlinkLine::from("cached")];

    assert_eq!(
        visible_lines(cell.transcript_hyperlink_lines(width)),
        vec![Line::from("cached")]
    );
}

/// 渲染后的计划绝不能包含两个连续空行。
///
/// 回归保护：计划包装器以前会在“• Proposed Plan”标题后立即生成一个标题分隔空行以及一个多余的
/// 顶部内边距空行，从而产生明显的双重间隔。
fn assert_no_consecutive_blank_lines(lines: &[Line<'_>]) {
    let blank = |line: &Line<'_>| -> bool {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
            .trim()
            .is_empty()
    };
    for window in lines.windows(2) {
        assert!(
            !(blank(&window[0]) && blank(&window[1])),
            "rendered plan has two consecutive blank lines: {:?}",
            lines
                .iter()
                .map(|l| {
                    let t: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                    if blank(l) { "<BLANK>".to_string() } else { t }
                })
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn finalized_plan_has_no_consecutive_blank_lines() {
    let md = "# Plan Title\n\nIntro paragraph one.\n\n## Phase 1\n\n- Step A\n- Step B\n\nPara after list.\n";
    let cell = new_proposed_plan(md.to_string(), Path::new("/tmp"));
    let width = 48;
    let lines = visible_lines(cell.display_hyperlink_lines(width));
    assert_no_consecutive_blank_lines(&lines);
}

#[test]
fn finalized_plan_single_blank_after_header() {
    let md = "# Plan Title\nBody.\n";
    let cell = new_proposed_plan(md.to_string(), Path::new("/tmp"));
    let width = 48;
    let lines = visible_lines(cell.display_hyperlink_lines(width));
    let rendered: Vec<String> = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
        .collect();
    // 先是标题，接着恰好一个空白分隔行，然后是计划正文。
    assert_eq!(rendered[0].trim_end(), "• Proposed Plan");
    assert!(rendered[1].trim().is_empty(), "expected one blank after header, got {:?}", rendered[1]);
    assert!(!rendered[2].trim().is_empty(), "expected body right after the single blank, got {:?}", rendered);
}

#[test]
fn finalized_plan_heading_spans_carry_distinct_fg_colors() {
    let md = "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n";
    let cell = new_proposed_plan(md.to_string(), Path::new("/tmp"));
    let width = 48;
    let lines = visible_lines(cell.display_hyperlink_lines(width));
    // 收集每个标题行中首个非空且非前缀 span 的 fg 颜色。
    let mut seen_fgs = Vec::new();
    for line in &lines {
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let trimmed = text.trim_start();
        if trimmed.starts_with('#') {
            // “#”标记所在的 span 携带标题样式；记录其 fg。
            if let Some(span) = line.spans.iter().find(|s| s.content.starts_with('#')) {
                seen_fgs.push(format!("{:?}", span.style.fg));
            }
        }
    }
    eprintln!("DIAG heading fgs = {:?}", seen_fgs);
    let unique: std::collections::HashSet<_> = seen_fgs.iter().collect();
    assert_eq!(seen_fgs.len(), 6, "expected six heading lines, got {:?}", seen_fgs);
    assert_eq!(unique.len(), 6, "heading colors must all differ, got {:?}", seen_fgs);
}
