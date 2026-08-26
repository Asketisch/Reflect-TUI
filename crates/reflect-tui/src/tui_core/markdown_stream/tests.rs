//! 流式 Markdown 收集器（markdown_stream） 的测试集。
//!
//! 从 markdown_stream.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use ratatui::style::Color;

#[tokio::test]
async fn no_commit_until_newline() {
    let mut c = super::MarkdownStreamCollector::new(/*width*/ None, &super::test_cwd());
    c.push_delta("Hello, world");
    let out = c.commit_complete_lines();
    assert!(out.is_empty(), "should not commit without newline");
    c.push_delta("!\n");
    let out2 = c.commit_complete_lines();
    assert_eq!(out2.len(), 1, "one completed line after newline");
}

#[tokio::test]
async fn finalize_commits_partial_line() {
    let mut c = super::MarkdownStreamCollector::new(/*width*/ None, &super::test_cwd());
    c.push_delta("Line without newline");
    let out = c.finalize_and_drain();
    assert_eq!(out.len(), 1);
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn e2e_stream_blockquote_simple_is_green() {
    let out = super::simulate_stream_markdown_for_tests(&["> Hello\n"], /*finalize*/ true);
    assert_eq!(out.len(), 1);
    let l = &out[0];
    assert_eq!(
        l.style.fg,
        Some(Color::Green),
        "expected blockquote line fg green, got {:?}",
        l.style.fg
    );
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn e2e_stream_blockquote_nested_is_green() {
    let out = super::simulate_stream_markdown_for_tests(
        &["> Level 1\n>> Level 2\n"],
        /*finalize*/ true,
    );
    // 过滤掉可能在段落开头插入的空行。
    let non_blank: Vec<_> = out
        .into_iter()
        .filter(|l| {
            let s = l
                .spans
                .iter()
                .map(|sp| sp.content.clone())
                .collect::<Vec<_>>()
                .join("");
            let t = s.trim();
            // 忽略在段落边界处插入的仅含引用符号（如 ">"）的空行。
            !(t.is_empty() || t == ">")
        })
        .collect();
    assert_eq!(non_blank.len(), 2);
    assert_eq!(non_blank[0].style.fg, Some(Color::Green));
    assert_eq!(non_blank[1].style.fg, Some(Color::Green));
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn e2e_stream_blockquote_with_list_items_is_green() {
    let out = super::simulate_stream_markdown_for_tests(
        &["> - item 1\n> - item 2\n"],
        /*finalize*/ true,
    );
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].style.fg, Some(Color::Green));
    assert_eq!(out[1].style.fg, Some(Color::Green));
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn e2e_stream_nested_mixed_lists_ordered_marker_is_light_blue() {
    let md = [
        "1. First\n",
        "   - Second level\n",
        "     1. Third level (ordered)\n",
        "        - Fourth level (bullet)\n",
        "          - Fifth level to test indent consistency\n",
    ];
    let out = super::simulate_stream_markdown_for_tests(&md, /*finalize*/ true);
    // 找到包含第三级有序列表文本的那一行
    let find_idx = out.iter().position(|l| {
        l.spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<String>()
            .contains("Third level (ordered)")
    });
    let idx = find_idx.expect("expected third-level ordered line");
    let line = &out[idx];
    // 期望该行中至少有一个 span 使用浅蓝色样式
    let has_light_blue = line
        .spans
        .iter()
        .any(|s| s.style.fg == Some(ratatui::style::Color::LightBlue));
    assert!(
        has_light_blue,
        "expected an ordered-list marker span with light blue fg on: {line:?}"
    );
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn e2e_stream_blockquote_wrap_preserves_green_style() {
    let long = "> This is a very long quoted line that should wrap across multiple columns to verify style preservation.";
    let out = super::simulate_stream_markdown_for_tests(&[long, "\n"], /*finalize*/ true);
    // 按较窄的宽度换行，强制产生多行输出。
    let wrapped = crate::tui_core::wrapping::word_wrap_lines(
        out.iter(),
        crate::tui_core::wrapping::RtOptions::new(/*width*/ 24),
    );
    // 过滤掉纯空白行
    let non_blank: Vec<_> = wrapped
        .into_iter()
        .filter(|l| {
            let s = l
                .spans
                .iter()
                .map(|sp| sp.content.clone())
                .collect::<Vec<_>>()
                .join("");
            !s.trim().is_empty()
        })
        .collect();
    assert!(
        non_blank.len() >= 2,
        "expected wrapped blockquote to span multiple lines"
    );
    for (i, l) in non_blank.iter().enumerate() {
        assert_eq!(
            l.spans[0].style.fg,
            Some(Color::Green),
            "wrapped line {} should preserve green style, got {:?}",
            i,
            l.spans[0].style.fg
        );
    }
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn heading_starts_on_new_line_when_following_paragraph() {
    // 先流式发送一行段落，再在下一行发送一个标题。
    // 期望得到两行不同的渲染结果："Hello." 与 "Heading"。
    let mut c = super::MarkdownStreamCollector::new(/*width*/ None, &super::test_cwd());
    c.push_delta("Hello.\n");
    let out1 = c.commit_complete_lines();
    let s1: Vec<String> = out1
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        out1.len(),
        1,
        "first commit should contain only the paragraph line, got {}: {:?}",
        out1.len(),
        s1
    );

    c.push_delta("## Heading\n");
    let out2 = c.commit_complete_lines();
    let s2: Vec<String> = out2
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        s2,
        vec!["", "## Heading"],
        "expected a blank separator then the heading line"
    );

    let line_to_string = |l: &ratatui::text::Line<'_>| -> String {
        l.spans
            .iter()
            .map(|s| s.content.clone())
            .collect::<Vec<_>>()
            .join("")
    };

    assert_eq!(line_to_string(&out1[0]), "Hello.");
    assert_eq!(line_to_string(&out2[1]), "## Heading");
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn heading_not_inlined_when_split_across_chunks() {
    // 先发送不带行尾换行的段落，然后发送以换行开头并包含标题文本的分块，
    // 最后再发送一个换行。收集器应先只提交段落行，之后再把标题作为独立的一行提交。
    let mut c = super::MarkdownStreamCollector::new(/*width*/ None, &super::test_cwd());
    c.push_delta("Sounds good!");
    // 此时还不应有提交
    assert!(c.commit_complete_lines().is_empty());

    // 引入用于完成段落的换行符，以及标题的开头部分。
    c.push_delta("\n## Adding Bird subcommand");
    let out1 = c.commit_complete_lines();
    let s1: Vec<String> = out1
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        s1,
        vec!["Sounds good!"],
        "expected paragraph followed by blank separator before heading chunk"
    );

    // 现在用行尾换行符结束这一标题行。
    c.push_delta("\n");
    let out2 = c.commit_complete_lines();
    let s2: Vec<String> = out2
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        s2,
        vec!["", "## Adding Bird subcommand"],
        "expected the heading line only on the final commit"
    );

    // 合理性检查：对简单一行做原始 markdown 渲染不会产生多余的额外内容。
    let mut rendered: Vec<ratatui::text::Line<'static>> = Vec::new();
    let test_cwd = super::test_cwd();
    crate::tui_core::markdown::append_markdown(
        "Hello.\n",
        /*width*/ None,
        Some(test_cwd.as_path()),
        &mut rendered,
    );
    let rendered_strings: Vec<String> = rendered
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    assert_eq!(
        rendered_strings,
        vec!["Hello."],
        "unexpected markdown lines: {rendered_strings:?}"
    );
}

fn lines_to_plain_strings(lines: &[ratatui::text::Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn table_header_commits_without_holdback() {
    let mut c = super::MarkdownStreamCollector::new(/*width*/ None, &super::test_cwd());
    c.push_delta("| A | B |\n");
    let out1 = c.commit_complete_lines();
    let out1_str = lines_to_plain_strings(&out1);
    assert_eq!(out1_str, vec!["| A | B |".to_string()]);

    c.push_delta("| --- | --- |\n");
    let out = c.commit_complete_lines();
    let out_str = lines_to_plain_strings(&out);
    assert!(
        !out_str.is_empty(),
        "expected output to continue committing after delimiter: {out_str:?}"
    );

    c.push_delta("| 1 | 2 |\n");
    let out2 = c.commit_complete_lines();
    assert!(
        !out2.is_empty(),
        "expected output to continue committing after body row"
    );

    c.push_delta("\n");
    let _ = c.commit_complete_lines();
}

#[tokio::test]
async fn pipe_text_without_table_prefix_is_not_delayed() {
    let mut c = super::MarkdownStreamCollector::new(/*width*/ None, &super::test_cwd());
    c.push_delta("Escaped pipe in text: a | b | c\n");
    let out = c.commit_complete_lines();
    let out_str = lines_to_plain_strings(&out);
    assert_eq!(out_str, vec!["Escaped pipe in text: a | b | c".to_string()]);
}

#[tokio::test]
async fn lists_and_fences_commit_without_duplication() {
    // 列表场景
    assert_streamed_equals_full(&["- a\n- ", "b\n- c\n"]).await;

    // 围栏代码场景：以小分块进行流式发送
    assert_streamed_equals_full(&["```", "\nco", "de 1\ncode 2\n", "```\n"]).await;
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn utf8_boundary_safety_and_wide_chars() {
    // 表情符号（宽字符）、中日韩文字、控制字符、数字 + 组合长音符序列
    let input = "🙂🙂🙂\n汉字漢字\nA\u{0003}0\u{0304}\n";
    let deltas = vec![
        "🙂",
        "🙂",
        "🙂\n汉",
        "字漢",
        "字\nA",
        "\u{0003}",
        "0",
        "\u{0304}",
        "\n",
    ];

    let streamed = simulate_stream_markdown_for_tests(&deltas, /*finalize*/ true);
    let streamed_str = lines_to_plain_strings(&streamed);

    let mut rendered_all: Vec<ratatui::text::Line<'static>> = Vec::new();
    let test_cwd = super::test_cwd();
    crate::tui_core::markdown::append_markdown(
        input,
        /*width*/ None,
        Some(test_cwd.as_path()),
        &mut rendered_all,
    );
    let rendered_all_str = lines_to_plain_strings(&rendered_all);

    assert_eq!(
        streamed_str, rendered_all_str,
        "utf8/wide-char streaming should equal full render without duplication or truncation"
    );
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn e2e_stream_deep_nested_third_level_marker_is_light_blue() {
    let md = "1. First\n   - Second level\n     1. Third level (ordered)\n        - Fourth level (bullet)\n          - Fifth level to test indent consistency\n";
    let streamed = super::simulate_stream_markdown_for_tests(&[md], /*finalize*/ true);
    let streamed_strs = lines_to_plain_strings(&streamed);

    // 在流式输出中定位第三级列表行；不要依赖精确的缩进。
    let target_suffix = "1. Third level (ordered)";
    let mut found = None;
    for line in &streamed {
        let s: String = line.spans.iter().map(|sp| sp.content.clone()).collect();
        if s.contains(target_suffix) {
            found = Some(line.clone());
            break;
        }
    }
    let line = found.unwrap_or_else(|| {
        panic!("expected to find the third-level ordered list line; got: {streamed_strs:?}")
    });

    // 期望列表标记（含缩进与 "1."）位于第一个 span 中，
    // 并使用 LightBlue 着色；后续内容应为默认颜色。
    assert!(
        !line.spans.is_empty(),
        "expected non-empty spans for the third-level line"
    );
    let marker_span = &line.spans[0];
    assert_eq!(
        marker_span.style.fg,
        Some(Color::LightBlue),
        "expected LightBlue 3rd-level ordered marker, got {:?}",
        marker_span.style.fg
    );
    // 找到第一个非空且非空白的内容 span，并验证它使用默认颜色。
    let mut content_fg = None;
    for sp in &line.spans[1..] {
        let t = sp.content.trim();
        if !t.is_empty() {
            content_fg = Some(sp.style.fg);
            break;
        }
    }
    assert_eq!(
        content_fg.flatten(),
        None,
        "expected default color for 3rd-level content, got {content_fg:?}"
    );
}

#[tokio::test]
async fn empty_fenced_block_is_dropped_and_separator_preserved_before_heading() {
    // 一个空的围栏代码块后面紧跟标题时，不应渲染出围栏本身，
    // 但应保留一行空白分隔行，使标题从新的一行开始。
    let deltas = vec!["```bash\n```\n", "## Heading\n"]; // 空代码块与其结束围栏在同一次提交中
    let streamed = simulate_stream_markdown_for_tests(&deltas, /*finalize*/ true);
    let texts = lines_to_plain_strings(&streamed);
    assert!(
        texts.iter().all(|s| !s.contains("```")),
        "no fence markers expected: {texts:?}"
    );
    // 期望出现标题且不含围栏标记。开头处的空白分隔行可能渲染，也可能不渲染。
    assert!(
        texts.iter().any(|s| s == "## Heading"),
        "expected heading line: {texts:?}"
    );
}

#[tokio::test]
async fn paragraph_then_empty_fence_then_heading_keeps_heading_on_new_line() {
    let deltas = vec!["Para.\n", "```\n```\n", "## Title\n"]; // 空围栏代码块在一次提交中
    let streamed = simulate_stream_markdown_for_tests(&deltas, /*finalize*/ true);
    let texts = lines_to_plain_strings(&streamed);
    let para_idx = match texts.iter().position(|s| s == "Para.") {
        Some(i) => i,
        None => panic!("para present"),
    };
    let head_idx = match texts.iter().position(|s| s == "## Title") {
        Some(i) => i,
        None => panic!("heading present"),
    };
    assert!(
        head_idx > para_idx,
        "heading should not merge with paragraph: {texts:?}"
    );
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn loose_list_with_split_dashes_matches_full_render() {
    // 由辅助工具找到的最小化失败序列：两个分块即可复现该不一致。
    let deltas = vec!["- item.\n\n", "-"];

    let streamed = simulate_stream_markdown_for_tests(&deltas, /*finalize*/ true);
    let streamed_strs = lines_to_plain_strings(&streamed);

    let full: String = deltas.iter().copied().collect();
    let mut rendered_all: Vec<ratatui::text::Line<'static>> = Vec::new();
    let test_cwd = super::test_cwd();
    crate::tui_core::markdown::append_markdown(
        &full,
        /*width*/ None,
        Some(test_cwd.as_path()),
        &mut rendered_all,
    );
    let rendered_all_strs = lines_to_plain_strings(&rendered_all);

    assert_eq!(
        streamed_strs, rendered_all_strs,
        "streamed output should match full render without dangling '-' lines"
    );
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn loose_vs_tight_list_items_streaming_matches_full() {
    // 增量数据取自 2025-08-27T00:33:18.216Z 附近的会话日志
    let deltas = vec![
        "\n\n",
        "Loose",
        " vs",
        ".",
        " tight",
        " list",
        " items",
        ":\n",
        "1",
        ".",
        " Tight",
        " item",
        "\n",
        "2",
        ".",
        " Another",
        " tight",
        " item",
        "\n\n",
        "1",
        ".",
        " Loose",
        " item",
        " with",
        " its",
        " own",
        " paragraph",
        ".\n\n",
        "  ",
        " This",
        " paragraph",
        " belongs",
        " to",
        " the",
        " same",
        " list",
        " item",
        ".\n\n",
        "2",
        ".",
        " Second",
        " loose",
        " item",
        " with",
        " a",
        " nested",
        " list",
        " after",
        " a",
        " blank",
        " line",
        ".\n\n",
        "  ",
        " -",
        " Nested",
        " bullet",
        " under",
        " a",
        " loose",
        " item",
        "\n",
        "  ",
        " -",
        " Another",
        " nested",
        " bullet",
        "\n\n",
    ];

    let streamed = simulate_stream_markdown_for_tests(&deltas, /*finalize*/ true);
    let streamed_strs = lines_to_plain_strings(&streamed);

    // 仅为便于诊断而计算一次完整渲染。
    let full: String = deltas.iter().copied().collect();
    let mut rendered_all: Vec<ratatui::text::Line<'static>> = Vec::new();
    let test_cwd = super::test_cwd();
    crate::tui_core::markdown::append_markdown(
        &full,
        /*width*/ None,
        Some(test_cwd.as_path()),
        &mut rendered_all,
    );

    // 同时断言精确的期望纯文本行，以便更清晰。
    let expected = vec![
        "Loose vs. tight list items:".to_string(),
        "".to_string(),
        "1. Tight item".to_string(),
        "2. Another tight item".to_string(),
        "3. Loose item with its own paragraph.".to_string(),
        "".to_string(),
        "   This paragraph belongs to the same list item.".to_string(),
        "".to_string(),
        "4. Second loose item with a nested list after a blank line.".to_string(),
        "    - Nested bullet under a loose item".to_string(),
        "    - Another nested bullet".to_string(),
    ];
    assert_eq!(
        streamed_strs, expected,
        "expected exact rendered lines for loose/tight section"
    );
}

// 源自模糊测试发现的针对性测试。每个都断言流式渲染结果 == 完整渲染结果。
async fn assert_streamed_equals_full(deltas: &[&str]) {
    let streamed = simulate_stream_markdown_for_tests(deltas, /*finalize*/ true);
    let streamed_strs = lines_to_plain_strings(&streamed);
    let full: String = deltas.iter().copied().collect();
    let mut rendered: Vec<ratatui::text::Line<'static>> = Vec::new();
    let test_cwd = super::test_cwd();
    crate::tui_core::markdown::append_markdown(
        &full,
        /*width*/ None,
        Some(test_cwd.as_path()),
        &mut rendered,
    );
    let rendered_strs = lines_to_plain_strings(&rendered);
    assert_eq!(streamed_strs, rendered_strs, "full:\n---\n{full}\n---");
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn fuzz_class_bullet_duplication_variant_1() {
    assert_streamed_equals_full(&["aph.\n- let one\n- bull", "et two\n\n  second paragraph \n"])
        .await;
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn fuzz_class_bullet_duplication_variant_2() {
    assert_streamed_equals_full(&[
        "- e\n  c",
        "e\n- bullet two\n\n  second paragraph in bullet two\n",
    ])
    .await;
}

#[tokio::test]
async fn streaming_html_block_then_text_matches_full() {
    assert_streamed_equals_full(&["HTML block:\n", "<div>inline block</div>\n", "more stuff\n"])
        .await;
}

#[tokio::test]
async fn table_like_lines_inside_fenced_code_are_not_held() {
    assert_streamed_equals_full(&["```\n", "| a | b |\n", "```\n"]).await;
}

#[tokio::test]
#[ignore = "Phase 1: requires full Reflect implementation"]
async fn collector_source_chunks_round_trip_into_agent_fence_unwrapping() {
    let deltas = [
        "```md\n",
        "| A | B |\n",
        "|---|---|\n",
        "| 1 | 2 |\n",
        "```\n",
    ];
    let mut collector =
        super::MarkdownStreamCollector::new(/*width*/ None, &super::test_cwd());
    let mut committed_source = String::new();

    for delta in deltas {
        collector.push_delta(delta);
        if delta.contains('\n')
            && let Some(range) = collector.commit_complete_source()
        {
            committed_source.push_str(&collector.committed_source()[range]);
        }
    }
    assert_eq!(collector.committed_source(), committed_source);
    let raw_source = collector.finalize_and_take_source();

    let mut rendered = Vec::new();
    crate::tui_core::markdown::append_markdown_agent(
        &raw_source,
        /*width*/ None,
        &mut rendered,
    );
    let rendered_strs = lines_to_plain_strings(&rendered);

    assert!(
        rendered_strs.iter().any(|line| line.contains('━')),
        "expected markdown-fenced table to render with a separator: {rendered_strs:?}"
    );
    assert!(
        !rendered_strs.iter().any(|line| line.trim() == "| A | B |"),
        "did not expect raw table header after markdown-fence unwrapping: {rendered_strs:?}"
    );
}

#[test]
fn finalizing_empty_collector_returns_empty_source() {
    let mut collector =
        super::MarkdownStreamCollector::new(/*width*/ None, &super::test_cwd());

    assert_eq!(collector.finalize_and_take_source(), String::new());
}
