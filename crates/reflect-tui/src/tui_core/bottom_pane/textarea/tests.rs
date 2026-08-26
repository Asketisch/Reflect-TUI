//! 文本输入区 的测试集。
//!
//! 从 bottom_pane/textarea.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;
use crate::tui_core::key_hint;
// 为避免未使用警告，crossterm 类型在此处不导入
use pretty_assertions::assert_eq;
use rand::prelude::*;

fn rand_grapheme(rng: &mut rand::rngs::StdRng) -> String {
    let r: u8 = rng.random_range(0..100);
    match r {
        0..=4 => "\n".to_string(),
        5..=12 => " ".to_string(),
        13..=35 => (rng.random_range(b'a'..=b'z') as char).to_string(),
        36..=45 => (rng.random_range(b'A'..=b'Z') as char).to_string(),
        46..=52 => (rng.random_range(b'0'..=b'9') as char).to_string(),
        53..=65 => {
            // 部分表情符号（宽图符簇）
            let choices = ["👍", "😊", "🐍", "🚀", "🧪", "🌟"];
            choices[rng.random_range(0..choices.len())].to_string()
        }
        66..=75 => {
            // 中日韩宽字符
            let choices = ["漢", "字", "測", "試", "你", "好", "界", "编", "码"];
            choices[rng.random_range(0..choices.len())].to_string()
        }
        76..=85 => {
            // 组合标记序列
            let base = ["e", "a", "o", "n", "u"][rng.random_range(0..5)];
            let marks = ["\u{0301}", "\u{0308}", "\u{0302}", "\u{0303}"];
            format!("{base}{}", marks[rng.random_range(0..marks.len())])
        }
        86..=92 => {
            // 部分非拉丁单代码点（希腊、西里尔、希伯来文）
            let choices = ["Ω", "β", "Ж", "ю", "ש", "م", "ह"];
            choices[rng.random_range(0..choices.len())].to_string()
        }
        _ => {
            // ZWJ 序列（单图符簇但多代码点）
            let choices = [
                "👩\u{200D}💻", // 女技术人员
                "👨\u{200D}💻", // 男技术人员
                "🏳️\u{200D}🌈", // 彩虹旗
            ];
            choices[rng.random_range(0..choices.len())].to_string()
        }
    }
}

fn ta_with(text: &str) -> TextArea {
    let mut t = TextArea::new();
    t.insert_str(text);
    t
}

#[test]
fn insert_and_replace_update_cursor_and_text() {
    // 插入辅助函数
    let mut t = ta_with("hello");
    t.set_cursor(/*pos*/ 5);
    t.insert_str("!");
    assert_eq!(t.text(), "hello!");
    assert_eq!(t.cursor(), 6);

    t.insert_str_at(/*pos*/ 0, "X");
    assert_eq!(t.text(), "Xhello!");
    assert_eq!(t.cursor(), 7);

    // 在光标后插入不应移动光标
    t.set_cursor(/*pos*/ 1);
    let end = t.text().len();
    t.insert_str_at(end, "Y");
    assert_eq!(t.text(), "Xhello!Y");
    assert_eq!(t.cursor(), 1);

    // replace_range 测试用例
    // 1）光标位于范围之前
    let mut t = ta_with("abcd");
    t.set_cursor(/*pos*/ 1);
    t.replace_range(2..3, "Z");
    assert_eq!(t.text(), "abZd");
    assert_eq!(t.cursor(), 1);

    // 2）光标位于范围内
    let mut t = ta_with("abcd");
    t.set_cursor(/*pos*/ 2);
    t.replace_range(1..3, "Q");
    assert_eq!(t.text(), "aQd");
    assert_eq!(t.cursor(), 2);

    // 3）光标位于范围之后，并按差值偏移
    let mut t = ta_with("abcd");
    t.set_cursor(/*pos*/ 4);
    t.replace_range(0..1, "AA");
    assert_eq!(t.text(), "AAbcd");
    assert_eq!(t.cursor(), 5);
}

#[test]
fn insert_str_at_clamps_to_char_boundary() {
    let mut t = TextArea::new();
    t.insert_str("你");
    t.set_cursor(/*pos*/ 0);
    t.insert_str_at(/*pos*/ 1, "A");
    assert_eq!(t.text(), "A你");
    assert_eq!(t.cursor(), 1);
}

#[test]
fn set_text_clamps_cursor_to_char_boundary() {
    let mut t = TextArea::new();
    t.insert_str("abcd");
    t.set_cursor(/*pos*/ 1);
    t.set_text_clearing_elements("你");
    assert_eq!(t.cursor(), 0);
    t.insert_str("a");
    assert_eq!(t.text(), "a你");
}

#[test]
fn delete_backward_and_forward_edges() {
    let mut t = ta_with("abc");
    t.set_cursor(/*pos*/ 1);
    t.delete_backward(/*n*/ 1);
    assert_eq!(t.text(), "bc");
    assert_eq!(t.cursor(), 0);

    // 在起始位置向后删除是空操作
    t.set_cursor(/*pos*/ 0);
    t.delete_backward(/*n*/ 1);
    assert_eq!(t.text(), "bc");
    assert_eq!(t.cursor(), 0);

    // 向前删除移除下一个图符簇
    t.set_cursor(/*pos*/ 1);
    t.delete_forward(/*n*/ 1);
    assert_eq!(t.text(), "b");
    assert_eq!(t.cursor(), 1);

    // 在末尾向前删除是空操作
    t.set_cursor(t.text().len());
    t.delete_forward(/*n*/ 1);
    assert_eq!(t.text(), "b");
}

#[test]
fn delete_forward_deletes_element_at_left_edge() {
    let mut t = TextArea::new();
    t.insert_str("a");
    t.insert_element("<element>");
    t.insert_str("b");

    let elem_start = t.elements[0].range.start;
    t.set_cursor(elem_start);
    t.delete_forward(/*n*/ 1);

    assert_eq!(t.text(), "ab");
    assert_eq!(t.cursor(), elem_start);
}

#[test]
fn vim_insert_and_escape() {
    let mut t = TextArea::new();
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(t.text(), "h");
    assert_eq!(t.vim_mode_label(), Some("Normal"));
    assert_eq!(t.cursor(), 0);
}

#[test]
fn vim_insert_key_enters_insert_mode() {
    let mut t = TextArea::new();
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));

    assert_eq!(t.text(), "h");
    assert_eq!(t.vim_mode_label(), Some("Insert"));
}

#[test]
fn vim_normal_arrow_keys_move_cursor() {
    let mut t = ta_with("ab\ncd");
    t.set_cursor(/*pos*/ 1);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(t.cursor(), 2);

    t.input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(t.cursor(), 5);

    t.input(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(t.cursor(), 4);

    t.input(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(t.cursor(), 1);
}

#[test]
fn vim_escape_from_insert_at_start_does_not_underflow() {
    let mut t = TextArea::new();
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(t.vim_mode_label(), Some("Normal"));
    assert_eq!(t.cursor(), 0);
}

#[test]
fn vim_escape_from_insert_at_line_start_stays_on_line() {
    let mut t = ta_with("one\ntwo");
    t.set_cursor(/*pos*/ "one\n".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(t.vim_mode_label(), Some("Normal"));
    assert_eq!(t.cursor(), "one\n".len());
}

#[test]
fn vim_escape_moves_by_grapheme_boundary() {
    let mut t = ta_with("👍👍");
    t.set_cursor(t.text().len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(t.vim_mode_label(), Some("Normal"));
    assert_eq!(t.cursor(), "👍".len());
}

#[test]
fn vim_escape_respects_atomic_element_boundary() {
    let mut t = TextArea::new();
    t.insert_str("a");
    t.insert_element("<element>");
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(t.vim_mode_label(), Some("Normal"));
    assert_eq!(t.cursor(), 1);
}

#[test]
fn vim_shift_i_enters_insert_at_first_non_blank_with_shift_only_binding() {
    let mut t = ta_with("hello\n  world");
    t.vim_normal_keymap.insert_line_start = vec![key_hint::shift(KeyCode::Char('i'))];
    t.set_cursor(/*pos*/ "hello\n  wor".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('I'), KeyModifiers::NONE));

    assert_eq!(t.vim_mode_label(), Some("Insert"));
    assert_eq!(t.cursor(), "hello\n  ".len());
}

#[test]
fn vim_shift_a_enters_insert_at_line_end_with_shift_only_binding() {
    let mut t = ta_with("hello\nworld");
    t.vim_normal_keymap.append_line_end = vec![key_hint::shift(KeyCode::Char('a'))];
    t.set_cursor(/*pos*/ 8);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::NONE));

    assert_eq!(t.vim_mode_label(), Some("Insert"));
    assert_eq!(t.cursor(), 11);
}

#[test]
fn vim_shift_c_changes_to_line_end_and_enters_insert_mode() {
    let mut t = ta_with("hello world\nnext line");
    t.set_cursor(/*pos*/ 6);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::SHIFT));

    assert_eq!(t.text(), "hello \nnext line");
    assert_eq!(t.vim_mode_label(), Some("Insert"));
    assert_eq!(t.cursor(), 6);
    assert_eq!(t.kill_buffer, "world");
}

#[test]
fn vim_uppercase_c_changes_to_line_end() {
    let mut t = ta_with("hello world\nnext line");
    t.set_cursor(/*pos*/ 6);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE));

    assert_eq!(t.text(), "hello \nnext line");
    assert_eq!(t.vim_mode_label(), Some("Insert"));
    assert_eq!(t.cursor(), 6);
}

#[test]
fn vim_s_substitutes_current_character_and_enters_insert_mode() {
    let mut t = ta_with("abc");
    t.set_cursor(/*pos*/ 1);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));

    assert_eq!(t.text(), "ac");
    assert_eq!(t.cursor(), 1);
    assert_eq!(t.vim_mode_label(), Some("Insert"));

    t.input(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));

    assert_eq!(t.text(), "aXc");
    assert_eq!(t.cursor(), 2);
    assert_eq!(t.vim_mode_label(), Some("Insert"));
}

#[test]
fn vim_s_on_empty_line_enters_insert_without_deleting_newline() {
    let mut t = ta_with("before\n\nnext");
    t.set_cursor(/*pos*/ "before\n".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));

    assert_eq!(t.text(), "before\n\nnext");
    assert_eq!(t.cursor(), "before\n".len());
    assert_eq!(t.vim_mode_label(), Some("Insert"));
}

#[test]
fn vim_d_at_line_end_does_not_remove_newline() {
    let mut t = ta_with("hello\nworld");
    t.set_cursor(/*pos*/ "hello".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE));

    assert_eq!(t.text(), "hello\nworld");
    assert_eq!(t.vim_mode_label(), Some("Normal"));
    assert_eq!(t.kill_buffer, "");
}

#[test]
fn vim_c_at_line_end_enters_insert_without_removing_newline() {
    let mut t = ta_with("hello\nworld");
    t.set_cursor(/*pos*/ "hello".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE));

    assert_eq!(t.text(), "hello\nworld");
    assert_eq!(t.vim_mode_label(), Some("Insert"));
    assert_eq!(t.cursor(), "hello".len());
    assert_eq!(t.kill_buffer, "");
}

#[test]
fn vim_shift_o_opens_line_above_with_shift_only_binding() {
    let mut t = ta_with("hello\nworld");
    t.vim_normal_keymap.open_line_above = vec![key_hint::shift(KeyCode::Char('o'))];
    t.set_cursor(/*pos*/ 8);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('O'), KeyModifiers::NONE));

    assert_eq!(t.text(), "hello\n\nworld");
    assert_eq!(t.vim_mode_label(), Some("Insert"));
    assert_eq!(t.cursor(), 6);
}

#[test]
fn vim_o_opens_line_below_on_inserted_line() {
    let mut t = ta_with("one\ntwo");
    t.set_cursor(/*pos*/ 1);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));

    assert_eq!(t.text(), "one\n\ntwo");
    assert_eq!(t.vim_mode_label(), Some("Insert"));
    assert_eq!(t.cursor(), "one\n".len());
}

#[test]
fn vim_o_opens_line_below_final_line_and_moves_to_new_line() {
    let mut t = ta_with("one");
    t.set_cursor(/*pos*/ 1);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));

    assert_eq!(t.text(), "one\n");
    assert_eq!(t.vim_mode_label(), Some("Insert"));
    assert_eq!(t.cursor(), "one\n".len());
}

#[test]
fn vim_delete_word() {
    let mut t = ta_with("hello world");
    t.set_cursor(/*pos*/ 0);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));

    assert_eq!(t.text(), "world");
    assert_eq!(t.kill_buffer, "hello ");
}

#[test]
fn vim_change_inner_word_deletes_word_and_enters_insert() {
    let mut t = ta_with("hello world");
    t.set_cursor(/*pos*/ "hello ".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));

    assert_eq!(t.text(), "hello ");
    assert_eq!(t.kill_buffer, "world");
    assert_eq!(t.cursor(), "hello ".len());
    assert_eq!(t.vim_mode_label(), Some("Insert"));
}

#[test]
fn vim_word_text_objects_cover_delete_yank_and_big_word() {
    let mut t = ta_with("hello world");
    t.set_cursor(/*pos*/ 1);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));

    assert_eq!(t.text(), "hello world");
    assert_eq!(t.kill_buffer, "hello ");
    assert_eq!(t.vim_mode_label(), Some("Normal"));

    let mut t = ta_with("foo.bar/baz qux");
    t.set_cursor(/*pos*/ "foo.".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE));

    assert_eq!(t.text(), " qux");
    assert_eq!(t.kill_buffer, "foo.bar/baz");
}

#[test]
fn vim_word_text_objects_accept_cursor_at_word_end() {
    let mut t = ta_with("hello world");
    t.set_cursor(/*pos*/ "hello".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));

    assert_eq!(t.text(), "world");
    assert_eq!(t.kill_buffer, "hello ");

    let mut t = ta_with("foo bar");
    t.set_cursor(t.text().len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::NONE));

    assert_eq!(t.text(), "foo ");
    assert_eq!(t.kill_buffer, "bar");
    assert_eq!(t.cursor(), "foo ".len());
    assert_eq!(t.vim_mode_label(), Some("Insert"));
}

#[test]
fn vim_delimiter_text_objects_select_innermost_pair_and_aliases() {
    let mut t = ta_with("a(b(c)d)e");
    t.set_cursor(/*pos*/ "a(b(".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));

    assert_eq!(t.text(), "a(b()d)e");
    assert_eq!(t.kill_buffer, "c");
    assert_eq!(t.vim_mode_label(), Some("Insert"));

    let mut t = ta_with("a [b] c");
    t.set_cursor(/*pos*/ "a [".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));

    assert_eq!(t.text(), "a  c");
    assert_eq!(t.kill_buffer, "[b]");
}

#[test]
fn vim_empty_inner_text_objects_are_valid_targets() {
    let mut t = ta_with("call()");
    t.set_cursor(/*pos*/ "call(".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('('), KeyModifiers::NONE));

    assert_eq!(t.text(), "call()");
    assert_eq!(t.kill_buffer, "");
    assert_eq!(t.cursor(), "call(".len());
    assert_eq!(t.vim_mode_label(), Some("Insert"));

    let mut t = ta_with(r#"say "" now"#);
    t.set_cursor(/*pos*/ r#"say ""#.len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('"'), KeyModifiers::NONE));

    assert_eq!(t.text(), r#"say "" now"#);
    assert_eq!(t.kill_buffer, "");
    assert_eq!(t.cursor(), r#"say ""#.len());
    assert_eq!(t.vim_mode_label(), Some("Insert"));
}

#[test]
fn vim_quote_text_objects_are_line_local_and_handle_escapes() {
    let mut t = ta_with(r#"say "a \"b\" c" now"#);
    t.set_cursor(/*pos*/ r#"say "a \"#.len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('"'), KeyModifiers::SHIFT));

    assert_eq!(t.text(), r#"say "" now"#);
    assert_eq!(t.kill_buffer, r#"a \"b\" c"#);
    assert_eq!(t.vim_mode_label(), Some("Insert"));

    let mut t = ta_with("one \"two\nthree\" four");
    t.set_cursor(/*pos*/ "one \"two\n".len());
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('"'), KeyModifiers::NONE));

    assert_eq!(t.text(), "one \"two\nthree\" four");
    assert_eq!(t.kill_buffer, "");
}

#[test]
fn vim_text_object_cancellation_and_unsupported_change_motions_do_not_edit() {
    let mut t = ta_with("hello world");
    t.set_cursor(/*pos*/ 1);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));

    assert_eq!(t.text(), "hello world");
    assert_eq!(t.kill_buffer, "");
    assert_eq!(t.vim_mode_label(), Some("Normal"));
    assert!(!t.is_vim_operator_pending());

    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    assert!(t.is_vim_operator_pending());
    t.input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(t.text(), "hello world");
    assert_eq!(t.kill_buffer, "");
    assert!(!t.is_vim_operator_pending());
}

#[test]
fn vim_operator_invalid_motion_is_consumed() {
    let mut t = ta_with("hello");
    t.set_cursor(/*pos*/ 0);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(t.is_vim_operator_pending());

    t.input(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));

    assert_eq!(t.text(), "hello");
    assert_eq!(t.vim_mode_label(), Some("Normal"));
    assert_eq!(t.cursor(), 0);
    assert!(!t.is_vim_operator_pending());
}

#[test]
fn vim_e_lands_on_word_end_character() {
    let mut t = ta_with("abc");
    t.set_cursor(/*pos*/ 0);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

    assert_eq!(t.cursor(), 2);

    t.input(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

    assert_eq!(t.text(), "ab");
    assert_eq!(t.kill_buffer, "c");
}

#[test]
fn vim_e_advances_from_each_word_end() {
    let mut t = ta_with("alpha beta gamma");
    t.set_cursor("alph".len()); // codespell:ignore alph
    t.set_vim_enabled(/*enabled*/ true);
    let mut states = Vec::new();

    for _ in 0..3 {
        t.input(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        states.push(format!("{}\n{}^", t.text(), " ".repeat(t.cursor())));
    }

    insta::assert_snapshot!("vim_e_advances_from_each_word_end", states.join("\n\n"));
}

#[test]
fn vim_delete_to_word_end_advances_from_existing_word_end() {
    let mut t = ta_with("alpha beta gamma");
    t.set_cursor("alph".len()); // codespell:ignore alph
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

    assert_eq!(t.text(), "alph gamma"); // codespell:ignore alph
    assert_eq!(t.kill_buffer, "a beta");
}

#[test]
fn vim_e_from_word_end_can_land_on_trailing_space() {
    let mut t = ta_with("alpha   ");
    t.set_cursor("alph".len()); // codespell:ignore alph
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

    assert_eq!(t.cursor(), "alpha  ".len());
}

#[test]
fn vim_e_advances_across_atomic_element_word_ends() {
    let mut t = TextArea::new();
    t.insert_str("alpha ");
    t.insert_element("<element>");
    t.insert_str(" gamma");
    let element_start = t.elements[0].range.start;
    t.set_cursor("alph".len()); // codespell:ignore alph
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(t.cursor(), element_start);

    t.input(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(t.cursor(), "alpha <element> gamm".len());
}

#[test]
fn vim_dollar_lands_on_line_end_character() {
    let mut t = ta_with("abc\n123");
    t.set_cursor(/*pos*/ 1);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('$'), KeyModifiers::NONE));

    assert_eq!(t.cursor(), 2);

    t.input(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

    assert_eq!(t.text(), "ab\n123");
    assert_eq!(t.kill_buffer, "c");
}

#[test]
fn vim_linewise_yank_pastes_below_current_line() {
    let mut t = ta_with("abc\n123\nxyz");
    t.set_cursor(/*pos*/ 1);
    t.set_vim_enabled(/*enabled*/ true);

    t.input(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    t.input(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));

    assert_eq!(t.text(), "abc\nabc\n123\nxyz");
    assert_eq!(t.cursor(), "abc\n".len());
    assert_eq!(t.kill_buffer, "abc\n");
    assert_eq!(t.kill_buffer_kind, KillBufferKind::Linewise);
}

#[test]
fn delete_backward_word_and_kill_line_variants() {
    // 在末尾处向后删除单词会删除整个前一个单词
    let mut t = ta_with("hello   world  ");
    t.set_cursor(t.text().len());
    t.delete_backward_word();
    assert_eq!(t.text(), "hello   ");
    assert_eq!(t.cursor(), 8);

    // 从单词内部删除：从单词开头到光标位置
    let mut t = ta_with("foo bar");
    t.set_cursor(/*pos*/ 6); // 位于“bar”内部（'a' 之后）
    t.delete_backward_word();
    assert_eq!(t.text(), "foo r");
    assert_eq!(t.cursor(), 4);

    // 从末尾处仅删除最后一个单词
    let mut t = ta_with("foo bar");
    t.set_cursor(t.text().len());
    t.delete_backward_word();
    assert_eq!(t.text(), "foo ");
    assert_eq!(t.cursor(), 4);

    // kill_to_end_of_line：不在行尾时
    let mut t = ta_with("abc\ndef");
    t.set_cursor(/*pos*/ 1); // 位于第一行中间
    t.kill_to_end_of_line();
    assert_eq!(t.text(), "a\ndef");
    assert_eq!(t.cursor(), 1);

    // kill_to_end_of_line：在行尾时删除换行符
    let mut t = ta_with("abc\ndef");
    t.set_cursor(/*pos*/ 3); // 第一行行尾
    t.kill_to_end_of_line();
    assert_eq!(t.text(), "abcdef");
    assert_eq!(t.cursor(), 3);

    // kill_to_beginning_of_line：从行中间开始
    let mut t = ta_with("abc\ndef");
    t.set_cursor(/*pos*/ 5); // 位于第二行，'e' 之后
    t.kill_to_beginning_of_line();
    assert_eq!(t.text(), "abc\nef");

    // kill_to_beginning_of_line：在非首行开头时移除前一行的换行符
    let mut t = ta_with("abc\ndef");
    t.set_cursor(/*pos*/ 4); // 第二行开头
    t.kill_to_beginning_of_line();
    assert_eq!(t.text(), "abcdef");
    assert_eq!(t.cursor(), 3);
}

#[test]
fn kill_current_line_removes_current_line_linewise() {
    let mut t = ta_with("abc\ndef\nghi");
    t.set_cursor(/*pos*/ 5);

    t.kill_current_line();

    assert_eq!(t.text(), "abc\nghi");
    assert_eq!(t.cursor(), 4);
    assert_eq!(t.kill_buffer, "def\n");
    assert_eq!(t.kill_buffer_kind, KillBufferKind::Linewise);
}

#[test]
fn kill_current_line_keeps_previous_newline_for_final_line() {
    let mut t = ta_with("abc\ndef");
    t.set_cursor(/*pos*/ 5);

    t.kill_current_line();

    assert_eq!(t.text(), "abc\n");
    assert_eq!(t.cursor(), 4);
    assert_eq!(t.kill_buffer, "def");
    assert_eq!(t.kill_buffer_kind, KillBufferKind::Linewise);
}

#[test]
fn kill_whole_line_keymap_dispatch_uses_linewise_kill() {
    let mut t = ta_with("abc\ndef\nghi");
    t.set_cursor(/*pos*/ 5);
    let mut keymap = RuntimeKeymap::defaults().editor;
    keymap.kill_line_start.clear();
    keymap.kill_whole_line = vec![key_hint::ctrl(KeyCode::Char('u'))];

    t.input_with_keymap(
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        &keymap,
    );

    assert_eq!(t.text(), "abc\nghi");
    assert_eq!(t.cursor(), 4);
    assert_eq!(t.kill_buffer, "def\n");
    assert_eq!(t.kill_buffer_kind, KillBufferKind::Linewise);
}

#[test]
fn delete_forward_word_variants() {
    let mut t = ta_with("hello   world ");
    t.set_cursor(/*pos*/ 0);
    t.delete_forward_word();
    assert_eq!(t.text(), "   world ");
    assert_eq!(t.cursor(), 0);

    let mut t = ta_with("hello   world ");
    t.set_cursor(/*pos*/ 1);
    t.delete_forward_word();
    assert_eq!(t.text(), "h   world ");
    assert_eq!(t.cursor(), 1);

    let mut t = ta_with("hello   world");
    t.set_cursor(t.text().len());
    t.delete_forward_word();
    assert_eq!(t.text(), "hello   world");
    assert_eq!(t.cursor(), t.text().len());

    let mut t = ta_with("foo   \nbar");
    t.set_cursor(/*pos*/ 3);
    t.delete_forward_word();
    assert_eq!(t.text(), "foo");
    assert_eq!(t.cursor(), 3);

    let mut t = ta_with("foo\nbar");
    t.set_cursor(/*pos*/ 3);
    t.delete_forward_word();
    assert_eq!(t.text(), "foo");
    assert_eq!(t.cursor(), 3);

    let mut t = ta_with("hello   world ");
    t.set_cursor(t.text().len() + 10);
    t.delete_forward_word();
    assert_eq!(t.text(), "hello   world ");
    assert_eq!(t.cursor(), t.text().len());
}

#[test]
fn delete_forward_word_handles_atomic_elements() {
    let mut t = TextArea::new();
    t.insert_element("<element>");
    t.insert_str(" tail");

    t.set_cursor(/*pos*/ 0);
    t.delete_forward_word();
    assert_eq!(t.text(), " tail");
    assert_eq!(t.cursor(), 0);

    let mut t = TextArea::new();
    t.insert_str("   ");
    t.insert_element("<element>");
    t.insert_str(" tail");

    t.set_cursor(/*pos*/ 0);
    t.delete_forward_word();
    assert_eq!(t.text(), " tail");
    assert_eq!(t.cursor(), 0);

    let mut t = TextArea::new();
    t.insert_str("prefix ");
    t.insert_element("<element>");
    t.insert_str(" tail");

    // 光标位于元素中间，delete_forward_word 删除该元素
    let elem_range = t.elements[0].range.clone();
    t.cursor_pos = elem_range.start + (elem_range.len() / 2);
    t.delete_forward_word();
    assert_eq!(t.text(), "prefix  tail");
    assert_eq!(t.cursor(), elem_range.start);
}

#[test]
fn delete_backward_word_respects_word_separators() {
    let mut t = ta_with("path/to/file");
    t.set_cursor(t.text().len());
    t.delete_backward_word();
    assert_eq!(t.text(), "path/to/");
    assert_eq!(t.cursor(), t.text().len());

    t.delete_backward_word();
    assert_eq!(t.text(), "path/to");
    assert_eq!(t.cursor(), t.text().len());

    let mut t = ta_with("foo/ ");
    t.set_cursor(t.text().len());
    t.delete_backward_word();
    assert_eq!(t.text(), "foo");
    assert_eq!(t.cursor(), 3);

    let mut t = ta_with("foo /");
    t.set_cursor(t.text().len());
    t.delete_backward_word();
    assert_eq!(t.text(), "foo ");
    assert_eq!(t.cursor(), 4);
}

#[test]
fn delete_forward_word_respects_word_separators() {
    let mut t = ta_with("path/to/file");
    t.set_cursor(/*pos*/ 0);
    t.delete_forward_word();
    assert_eq!(t.text(), "/to/file");
    assert_eq!(t.cursor(), 0);

    t.delete_forward_word();
    assert_eq!(t.text(), "to/file");
    assert_eq!(t.cursor(), 0);

    let mut t = ta_with("/ foo");
    t.set_cursor(/*pos*/ 0);
    t.delete_forward_word();
    assert_eq!(t.text(), " foo");
    assert_eq!(t.cursor(), 0);

    let mut t = ta_with(" /foo");
    t.set_cursor(/*pos*/ 0);
    t.delete_forward_word();
    assert_eq!(t.text(), "foo");
    assert_eq!(t.cursor(), 0);
}

#[test]
fn yank_restores_last_kill() {
    let mut t = ta_with("hello");
    t.set_cursor(/*pos*/ 0);
    t.kill_to_end_of_line();
    assert_eq!(t.text(), "");
    assert_eq!(t.cursor(), 0);

    t.yank();
    assert_eq!(t.text(), "hello");
    assert_eq!(t.cursor(), 5);

    let mut t = ta_with("hello world");
    t.set_cursor(t.text().len());
    t.delete_backward_word();
    assert_eq!(t.text(), "hello ");
    assert_eq!(t.cursor(), 6);

    t.yank();
    assert_eq!(t.text(), "hello world");
    assert_eq!(t.cursor(), 11);

    let mut t = ta_with("hello");
    t.set_cursor(/*pos*/ 5);
    t.kill_to_beginning_of_line();
    assert_eq!(t.text(), "");
    assert_eq!(t.cursor(), 0);

    t.yank();
    assert_eq!(t.text(), "hello");
    assert_eq!(t.cursor(), 5);
}

#[test]
fn kill_buffer_persists_across_set_text() {
    let mut t = ta_with("restore me");
    t.set_cursor(/*pos*/ 0);
    t.kill_to_end_of_line();
    assert!(t.text().is_empty());

    t.set_text_clearing_elements("/diff");
    t.set_text_clearing_elements("");
    t.yank();

    assert_eq!(t.text(), "restore me");
    assert_eq!(t.cursor(), "restore me".len());
}

#[test]
fn cursor_left_and_right_handle_graphemes() {
    let mut t = ta_with("a👍b");
    t.set_cursor(t.text().len());

    t.move_cursor_left(); // before 'b'
    let after_first_left = t.cursor();
    t.move_cursor_left(); // before '👍'
    let after_second_left = t.cursor();
    t.move_cursor_left(); // before 'a'
    let after_third_left = t.cursor();

    assert!(after_first_left < t.text().len());
    assert!(after_second_left < after_first_left);
    assert!(after_third_left < after_second_left);

    // 安全地向右移回末尾
    t.move_cursor_right();
    t.move_cursor_right();
    t.move_cursor_right();
    assert_eq!(t.cursor(), t.text().len());
}

#[test]
fn control_b_and_f_move_cursor() {
    let mut t = ta_with("abcd");
    t.set_cursor(/*pos*/ 1);

    t.input(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
    assert_eq!(t.cursor(), 2);

    t.input(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    assert_eq!(t.cursor(), 1);
}

#[test]
fn control_b_f_fallback_control_chars_move_cursor() {
    let mut t = ta_with("abcd");
    t.set_cursor(/*pos*/ 2);

    // 模拟发送不带 CONTROL 修饰符的 C0 控制字符的终端。
    // ^B (U+0002) 应向左移动
    t.input(KeyEvent::new(KeyCode::Char('\u{0002}'), KeyModifiers::NONE));
    assert_eq!(t.cursor(), 1);

    // ^F (U+0006) 应向右移动
    t.input(KeyEvent::new(KeyCode::Char('\u{0006}'), KeyModifiers::NONE));
    assert_eq!(t.cursor(), 2);
}

#[test]
fn c0_line_feed_inserts_newline_through_insert_newline_keymap() {
    let mut t = ta_with("ab");
    t.set_cursor(/*pos*/ 1);

    t.input(KeyEvent::new(KeyCode::Char('\u{000a}'), KeyModifiers::NONE));

    assert_eq!(t.text(), "a\nb");
    assert_eq!(t.cursor(), 2);
}

#[test]
fn c0_control_chars_respect_unbound_editor_movement() {
    let mut t = ta_with("a\nb");
    t.set_cursor(/*pos*/ 2);
    let mut keymap = RuntimeKeymap::defaults().editor;
    keymap.move_up.clear();

    t.input_with_keymap(
        KeyEvent::new(KeyCode::Char('\u{0010}'), KeyModifiers::NONE),
        &keymap,
    );

    assert_eq!(t.cursor(), 2);
}

#[test]
fn c0_control_chars_respect_remapped_editor_movement() {
    let mut t = ta_with("a\nb");
    t.set_cursor(/*pos*/ 0);
    let mut keymap = RuntimeKeymap::defaults().editor;
    keymap.move_up.clear();
    keymap.move_down = vec![crate::tui_core::key_hint::ctrl(KeyCode::Char('p'))];

    t.input_with_keymap(
        KeyEvent::new(KeyCode::Char('\u{0010}'), KeyModifiers::NONE),
        &keymap,
    );

    assert_eq!(t.cursor(), 2);
}

#[test]
fn delete_backward_word_alt_keys() {
    // 测试自定义的 Alt+Ctrl+h 绑定
    let mut t = ta_with("hello world");
    t.set_cursor(t.text().len()); // 光标位于末尾
    t.input(KeyEvent::new(
        KeyCode::Char('h'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    ));
    assert_eq!(t.text(), "hello ");
    assert_eq!(t.cursor(), 6);

    // 测试标准的 Alt+Backspace 绑定
    let mut t = ta_with("hello world");
    t.set_cursor(t.text().len()); // 光标位于末尾
    t.input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
    assert_eq!(t.text(), "hello ");
    assert_eq!(t.cursor(), 6);
}

#[test]
fn shift_backspace_and_shift_delete_keep_grapheme_delete_behavior() {
    let mut t = ta_with("abc");
    t.set_cursor(/*pos*/ 2);

    t.input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::SHIFT));
    assert_eq!(t.text(), "ac");
    assert_eq!(t.cursor(), 1);

    let mut t = ta_with("abc");
    t.set_cursor(/*pos*/ 1);

    t.input(KeyEvent::new(KeyCode::Delete, KeyModifiers::SHIFT));
    assert_eq!(t.text(), "ac");
    assert_eq!(t.cursor(), 1);
}

#[test]
fn control_backspace_variants_delete_backward_word() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        let mut t = ta_with("hello world");
        t.set_cursor(t.text().len());

        t.input(KeyEvent::new(KeyCode::Backspace, modifiers));
        assert_eq!(t.text(), "hello ");
        assert_eq!(t.cursor(), 6);
    }
}

#[test]
fn control_delete_variants_delete_forward_word() {
    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ] {
        let mut t = ta_with("hello world");
        t.set_cursor(/*pos*/ 0);

        t.input(KeyEvent::new(KeyCode::Delete, modifiers));
        assert_eq!(t.text(), " world");
        assert_eq!(t.cursor(), 0);
    }
}

#[test]
fn delete_backward_word_handles_narrow_no_break_space() {
    let mut t = ta_with("32\u{202F}AM");
    t.set_cursor(t.text().len());
    t.input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT));
    pretty_assertions::assert_eq!(t.text(), "32\u{202F}");
    pretty_assertions::assert_eq!(t.cursor(), t.text().len());
}

#[test]
fn delete_forward_word_with_without_alt_modifier() {
    let mut t = ta_with("hello world");
    t.set_cursor(/*pos*/ 0);
    t.input(KeyEvent::new(KeyCode::Delete, KeyModifiers::ALT));
    assert_eq!(t.text(), " world");
    assert_eq!(t.cursor(), 0);

    let mut t = ta_with("hello");
    t.set_cursor(/*pos*/ 0);
    t.input(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(t.text(), "ello");
    assert_eq!(t.cursor(), 0);
}

#[test]
fn delete_forward_word_alt_d() {
    let mut t = ta_with("hello world");
    t.set_cursor(/*pos*/ 6);
    t.input(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT));
    pretty_assertions::assert_eq!(t.text(), "hello ");
    pretty_assertions::assert_eq!(t.cursor(), 6);
}

#[test]
fn control_h_backspace() {
    // 测试 Ctrl+H 作为退格键
    let mut t = ta_with("12345");
    t.set_cursor(/*pos*/ 3); // 光标位于 '3' 之后
    t.input(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert_eq!(t.text(), "1245");
    assert_eq!(t.cursor(), 2);

    // 测试开头的 Ctrl+H（应为空操作）
    t.set_cursor(/*pos*/ 0);
    t.input(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert_eq!(t.text(), "1245");
    assert_eq!(t.cursor(), 0);

    // 测试末位的 Ctrl+H
    t.set_cursor(t.text().len());
    t.input(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL));
    assert_eq!(t.text(), "124");
    assert_eq!(t.cursor(), 3);
}

#[cfg_attr(not(windows), ignore = "AltGr modifier only applies on Windows")]
#[test]
fn altgr_ctrl_alt_char_inserts_literal() {
    let mut t = ta_with("");
    t.input(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL | KeyModifiers::ALT,
    ));
    assert_eq!(t.text(), "c");
    assert_eq!(t.cursor(), 1);
}

#[test]
fn cursor_vertical_movement_across_lines_and_bounds() {
    let mut t = ta_with("short\nloooooooooong\nmid");
    // 将光标放在第二行第 5 列
    let second_line_start = 6; // 位于第一个 '\n' 之后
    t.set_cursor(second_line_start + 5);

    // 向上移动：保留目标列，按行长截断
    t.move_cursor_up();
    assert_eq!(t.cursor(), 5); // 第一行长度为 5

    // 再次向上移动会到达文本开头
    t.move_cursor_up();
    assert_eq!(t.cursor(), 0);

    // 向下移动：从开头跟踪目标列
    t.move_cursor_down();
    // 第一次向下移动时，应落在第二行第 0 列（目标列记为 0）
    let pos_after_down = t.cursor();
    assert!(pos_after_down >= second_line_start);

    // 再次向下移动到第三行；截断到其长度
    t.move_cursor_down();
    let third_line_start = t.text().find("mid").unwrap();
    let third_line_end = third_line_start + 3;
    assert!(t.cursor() >= third_line_start && t.cursor() <= third_line_end);

    // 在最后一行向下移动会跳到末尾
    t.move_cursor_down();
    assert_eq!(t.cursor(), t.text().len());
}

#[test]
fn home_end_and_emacs_style_home_end() {
    let mut t = ta_with("one\ntwo\nthree");
    // 位于第二行中间
    let second_line_start = t.text().find("two").unwrap();
    t.set_cursor(second_line_start + 1);

    t.move_cursor_to_beginning_of_line(/*move_up_at_bol*/ false);
    assert_eq!(t.cursor(), second_line_start);

    // Ctrl-A 行为：如果在行首，则跳到上一行开头
    t.move_cursor_to_beginning_of_line(/*move_up_at_bol*/ true);
    assert_eq!(t.cursor(), 0); // 第一行开头

    // 移动到第一行的行尾
    t.move_cursor_to_end_of_line(/*move_down_at_eol*/ false);
    assert_eq!(t.cursor(), 3);

    // Ctrl-E：如果在行尾，则跳到下一行末尾
    t.move_cursor_to_end_of_line(/*move_down_at_eol*/ true);
    // 第二行（"two"）的行尾正好在其 '\n' 之前
    let end_second_nl = t.text().find("\nthree").unwrap();
    assert_eq!(t.cursor(), end_second_nl);
}

#[test]
fn end_of_line_or_down_at_end_of_text() {
    let mut t = ta_with("one\ntwo");
    // 将光标放在文本的绝对末尾
    t.set_cursor(t.text().len());
    // 应保持在末尾而不崩溃
    t.move_cursor_to_end_of_line(/*move_down_at_eol*/ true);
    assert_eq!(t.cursor(), t.text().len());

    // 同时验证非末行行尾的行为：
    let eol_first_line = 3; // “one\ntwo”中 '\n' 的索引
    t.set_cursor(eol_first_line);
    t.move_cursor_to_end_of_line(/*move_down_at_eol*/ true);
    assert_eq!(t.cursor(), t.text().len()); // 移动到下一行（末行）末尾
}

#[test]
fn word_navigation_helpers() {
    let t = ta_with("  alpha  beta   gamma");
    let mut t = t; // 设为可变以调用 set_cursor
    // 将光标放在 "alpha" 之后
    let after_alpha = t.text().find("alpha").unwrap() + "alpha".len();
    t.set_cursor(after_alpha);
    assert_eq!(t.beginning_of_previous_word(), 2); // 跳过开头空格

    // 将光标放在 beta 开头
    let beta_start = t.text().find("beta").unwrap();
    t.set_cursor(beta_start);
    assert_eq!(t.end_of_next_word(), beta_start + "beta".len());

    // 在末尾时，end_of_next_word 返回 len
    t.set_cursor(t.text().len());
    assert_eq!(t.end_of_next_word(), t.text().len());
}

#[test]
fn word_navigation_cjk_each_char_is_boundary() {
    let text = "你好世界";
    let mut t = ta_with(text);

    t.set_cursor(/*pos*/ text.len());
    assert_eq!(t.beginning_of_previous_word(), 9);

    t.set_cursor(/*pos*/ 9);
    assert_eq!(t.beginning_of_previous_word(), 6);

    t.set_cursor(/*pos*/ 6);
    assert_eq!(t.beginning_of_previous_word(), 3);

    t.set_cursor(/*pos*/ 3);
    assert_eq!(t.beginning_of_previous_word(), 0);
}

#[test]
fn word_navigation_cjk_forward() {
    let text = "你好世界";
    let mut t = ta_with(text);

    t.set_cursor(/*pos*/ 0);
    assert_eq!(t.end_of_next_word(), 3);

    t.set_cursor(/*pos*/ 3);
    assert_eq!(t.end_of_next_word(), 6);

    t.set_cursor(/*pos*/ 6);
    assert_eq!(t.end_of_next_word(), 9);

    t.set_cursor(/*pos*/ 9);
    assert_eq!(t.end_of_next_word(), 12);
}

#[test]
fn word_navigation_mixed_ascii_cjk() {
    let text = "hello你好";
    let mut t = ta_with(text);

    t.set_cursor(/*pos*/ 0);
    assert_eq!(t.end_of_next_word(), 5);

    t.set_cursor(/*pos*/ 5);
    assert_eq!(t.end_of_next_word(), 8);

    t.set_cursor(/*pos*/ text.len());
    assert_eq!(t.beginning_of_previous_word(), 8);

    t.set_cursor(/*pos*/ 8);
    assert_eq!(t.beginning_of_previous_word(), 5);

    t.set_cursor(/*pos*/ 5);
    assert_eq!(t.beginning_of_previous_word(), 0);
}

#[test]
fn word_navigation_preserves_separator_breaks_within_unicode_segments() {
    let mut t = ta_with("can't 32.3 foo.bar");

    t.set_cursor(/*pos*/ 5);
    assert_eq!(t.beginning_of_previous_word(), 4);

    t.set_cursor(/*pos*/ 4);
    assert_eq!(t.beginning_of_previous_word(), 3);

    t.set_cursor(/*pos*/ 10);
    assert_eq!(t.beginning_of_previous_word(), 9);

    t.set_cursor(/*pos*/ 18);
    assert_eq!(t.beginning_of_previous_word(), 15);
}

#[test]
fn wrapping_and_cursor_positions() {
    let mut t = ta_with("hello world here");
    let area = Rect::new(0, 0, 6, 10); // 宽度 6 -> 单词换行
    // desired height 计入折行后的行数
    assert!(t.desired_height(area.width) >= 3);

    // 将光标放在 "world" 中
    let world_start = t.text().find("world").unwrap();
    t.set_cursor(world_start + 3);
    let (_x, y) = t.cursor_pos(area).unwrap();
    assert_eq!(y, 1); // world 应位于第二个换行后的行中

    // 带状态和小高度时，光标映射到可见行
    let mut state = TextAreaState::default();
    let small_area = Rect::new(0, 0, 6, 1);
    // 第一次调用：光标不可见 → 有效滚动确保其可见
    let (_x, y) = t.cursor_pos_with_state(small_area, state).unwrap();
    assert_eq!(y, 0);

    // 带状态渲染以更新实际滚动值
    let mut buf = Buffer::empty(small_area);
    ratatui::widgets::StatefulWidgetRef::render_ref(&(&t), small_area, &mut buf, &mut state);
    // 渲染后，state.scroll 应被调整使光标行适配
    let effective_lines = t.desired_height(small_area.width);
    assert!(state.scroll < effective_lines);
}

#[test]
fn render_highlights_apply_style_without_mutating_text() {
    let t = ta_with("hello world");
    let area = Rect::new(0, 0, 20, 1);
    let mut state = TextAreaState::default();
    let mut buf = Buffer::empty(area);
    let highlight_style = Style::default().add_modifier(ratatui::style::Modifier::REVERSED);

    t.render_ref_styled_with_highlights(
        area,
        &mut buf,
        &mut state,
        Style::default(),
        &[(6..11, highlight_style)],
    );

    assert_eq!(t.text(), "hello world");
    assert!(
        !buf[(0, 0)]
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    assert!(
        buf[(6, 0)]
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    assert!(
        buf[(10, 0)]
            .style()
            .add_modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
}

#[test]
fn tabs_render_as_spaces_and_align_with_cursor_snapshot() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let text = "❌\tSimulation\tformatter[large/dataset.py]\t7.4 ms\t8.1 ms\t-8.29%";
    let mut t = ta_with(text);
    t.set_cursor(text.len());

    let mut terminal = Terminal::new(TestBackend::new(/*width*/ 100, /*height*/ 1)).unwrap();
    terminal
        .draw(|frame| {
            ratatui::widgets::WidgetRef::render_ref(&(&t), frame.area(), frame.buffer_mut());
        })
        .unwrap();

    let cursor = t.cursor_pos(terminal.backend().buffer().area).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(cursor.0 - 1, cursor.1)].symbol(),
        "%"
    );
    insta::assert_snapshot!(
        "textarea_tabs_render_as_spaces_and_align_with_cursor",
        format!("cursor: {cursor:?}\n{}", terminal.backend())
    );
}

#[test]
fn tabs_wrap_at_their_rendered_width() {
    let text = "1234\t5";
    let mut t = ta_with(text);
    t.set_cursor(text.len());
    let area = Rect::new(0, 0, /*width*/ 5, /*height*/ 2);
    let mut buf = Buffer::empty(area);

    ratatui::widgets::WidgetRef::render_ref(&(&t), area, &mut buf);

    assert_eq!(t.desired_height(area.width), 2);
    assert_eq!(t.cursor_pos(area), Some((1, 1)));
    assert_eq!(buf[(0, 1)].symbol(), "5");
}

#[test]
fn cursor_pos_with_state_basic_and_scroll_behaviors() {
    // 情况 1：无需折行且高度足够 —— 滚动被忽略，y 直接映射。
    let mut t = ta_with("hello world");
    t.set_cursor(/*pos*/ 3);
    let area = Rect::new(2, 5, 20, 3);
    // 即使提供了过大的滚动值，当内容适配区域时
    // 有效滚动为 0，光标位置与 cursor_pos 一致。
    let bad_state = TextAreaState { scroll: 999 };
    let (x1, y1) = t.cursor_pos(area).unwrap();
    let (x2, y2) = t.cursor_pos_with_state(area, bad_state).unwrap();
    assert_eq!((x2, y2), (x1, y1));

    // 情况 2：光标位于当前窗口下方 —— 调整有效滚动后
    // y 应被截断到最底行（area.height - 1）。
    let mut t = ta_with("one two three four five six");
    // 强制折行为多行视觉行。
    let wrap_width = 4;
    let _ = t.desired_height(wrap_width);
    // 将光标放在靠近末尾的位置，确保其一定位于第一个窗口下方。
    t.set_cursor(t.text().len().saturating_sub(2));
    let small_area = Rect::new(0, 0, wrap_width, 2);
    let state = TextAreaState { scroll: 0 };
    let (_x, y) = t.cursor_pos_with_state(small_area, state).unwrap();
    assert_eq!(y, small_area.y + small_area.height - 1);

    // 情况 3：光标位于当前窗口上方 —— 提供的滚动值过大时
    // y 应为顶行（0）
    let mut t = ta_with("alpha beta gamma delta epsilon zeta");
    let wrap_width = 5;
    let lines = t.desired_height(wrap_width);
    // 将光标放在靠近开头的位置，使过大的滚动将其移到顶行。
    t.set_cursor(/*pos*/ 1);
    let area = Rect::new(0, 0, wrap_width, 3);
    let state = TextAreaState {
        scroll: lines.saturating_mul(2),
    };
    let (_x, y) = t.cursor_pos_with_state(area, state).unwrap();
    assert_eq!(y, area.y);
}

#[test]
fn wrapped_navigation_across_visual_lines() {
    let mut t = ta_with("abcdefghij");
    // 强制在宽度 4 处折行：行 -> ["abcd", "efgh", "ij"]
    let _ = t.desired_height(/*width*/ 4);

    // 从最开始向下移动应到达下一折行的开头（索引 4）
    t.set_cursor(/*pos*/ 0);
    t.move_cursor_down();
    assert_eq!(t.cursor(), 4);

    // 边界索引 4 处光标应显示在第二折行的开头
    t.set_cursor(/*pos*/ 4);
    let area = Rect::new(0, 0, 4, 10);
    let (x, y) = t.cursor_pos(area).unwrap();
    assert_eq!((x, y), (0, 1));

    // 带状态和小高度时，光标应可见于第 0 行第 0 列
    let small_area = Rect::new(0, 0, 4, 1);
    let state = TextAreaState::default();
    let (x, y) = t.cursor_pos_with_state(small_area, state).unwrap();
    assert_eq!((x, y), (0, 0));

    // 将光标放在第二折行的中间（"efgh"），位于 'g' 处
    t.set_cursor(/*pos*/ 6);
    // 向上移动应到上一折行的同一列 -> 索引 2（'c'）
    t.move_cursor_up();
    assert_eq!(t.cursor(), 2);

    // 向下移动应回到下一折行的同一位置 -> 返回索引 6（'g'）
    t.move_cursor_down();
    assert_eq!(t.cursor(), 6);

    // 再次向下移动应到第三折行。目标列为 2，但该行长度 2 -> 截断到末尾
    t.move_cursor_down();
    assert_eq!(t.cursor(), t.text().len());
}

#[test]
fn cursor_pos_with_state_after_movements() {
    let mut t = ta_with("abcdefghij");
    // 折行宽度 4 -> 视觉行：abcd | efgh | ij
    let _ = t.desired_height(/*width*/ 4);
    let area = Rect::new(0, 0, 4, 2);
    let mut state = TextAreaState::default();
    let mut buf = Buffer::empty(area);

    // 从开头开始
    t.set_cursor(/*pos*/ 0);
    ratatui::widgets::StatefulWidgetRef::render_ref(&(&t), area, &mut buf, &mut state);
    let (x, y) = t.cursor_pos_with_state(area, state).unwrap();
    assert_eq!((x, y), (0, 0));

    // 向下移动到第二视觉行；应在 2 行视口内的底行（第 1 行）
    t.move_cursor_down();
    ratatui::widgets::StatefulWidgetRef::render_ref(&(&t), area, &mut buf, &mut state);
    let (x, y) = t.cursor_pos_with_state(area, state).unwrap();
    assert_eq!((x, y), (0, 1));

    // 向下移动到第三视觉行；视口滚动并保持光标在底行
    t.move_cursor_down();
    ratatui::widgets::StatefulWidgetRef::render_ref(&(&t), area, &mut buf, &mut state);
    let (x, y) = t.cursor_pos_with_state(area, state).unwrap();
    assert_eq!((x, y), (0, 1));

    // 向上移动到第二视觉行；在当前滚动下出现在顶行
    t.move_cursor_up();
    ratatui::widgets::StatefulWidgetRef::render_ref(&(&t), area, &mut buf, &mut state);
    let (x, y) = t.cursor_pos_with_state(area, state).unwrap();
    assert_eq!((x, y), (0, 0));

    // 移动时保留列：在第一行设为第 2 列，向下移动
    t.set_cursor(/*pos*/ 2);
    ratatui::widgets::StatefulWidgetRef::render_ref(&(&t), area, &mut buf, &mut state);
    let (x0, y0) = t.cursor_pos_with_state(area, state).unwrap();
    assert_eq!((x0, y0), (2, 0));
    t.move_cursor_down();
    ratatui::widgets::StatefulWidgetRef::render_ref(&(&t), area, &mut buf, &mut state);
    let (x1, y1) = t.cursor_pos_with_state(area, state).unwrap();
    assert_eq!((x1, y1), (2, 1));
}

#[test]
fn wrapped_navigation_with_newlines_and_spaces() {
    // 包含空格和显式换行符以测试边界
    let mut t = ta_with("word1  word2\nword3");
    // 宽度 6 会将 "word1  " 折行，然后在换行符前折行 "word2"
    let _ = t.desired_height(/*width*/ 6);

    // 在第二折行的换行符之前放置光标，位于 "word2" 的第 1 列
    let start_word2 = t.text().find("word2").unwrap();
    t.set_cursor(start_word2 + 1);

    // 向上应到第一折行第 1 列 -> 索引 1
    t.move_cursor_up();
    assert_eq!(t.cursor(), 1);

    // 向下应返回 "word2" 上的同一视觉列
    t.move_cursor_down();
    assert_eq!(t.cursor(), start_word2 + 1);

    // 再次向下越过逻辑换行符到下一视觉行（"word3"），如有需要则截断到其长度
    t.move_cursor_down();
    let start_word3 = t.text().find("word3").unwrap();
    assert!(t.cursor() >= start_word3 && t.cursor() <= start_word3 + "word3".len());
}

#[test]
fn wrapped_navigation_with_wide_graphemes() {
    // 四个点赞表情，每个显示宽度 2，宽度 3 强制在图符簇内折行
    let mut t = ta_with("👍👍👍👍");
    let _ = t.desired_height(/*width*/ 3);

    // 将光标放在第二个表情之后（应在第一折行上）
    t.set_cursor("👍👍".len());

    // 向下移动应到下一折行的开头（保留同一列但被截断）
    t.move_cursor_down();
    // 期望落在第三个表情内部或开头
    let pos_after_down = t.cursor();
    assert!(pos_after_down >= "👍👍".len());

    // 向上移动应回到原始位置
    t.move_cursor_up();
    assert_eq!(t.cursor(), "👍👍".len());
}

#[test]
fn fuzz_textarea_randomized() {
    // 确定性种子以确保可复现
    // 以太平洋时间（PST/PDT）的当天为基础为 RNG 播种。这样
    // 使模糊测试在一天内保持确定性，同时每天变化
    // 以提高覆盖率。
    let pst_today_seed: u64 = (chrono::Utc::now() - chrono::Duration::hours(8))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp() as u64;
    let mut rng = rand::rngs::StdRng::seed_from_u64(pst_today_seed);

    for _case in 0..500 {
        let mut ta = TextArea::new();
        let mut state = TextAreaState::default();
        // 跟踪插入的元素负载。负载使用 '[' 和 ']' 字符，
        // 这两个字符不会被 rand_grapheme() 生成，避免意外冲突。
        let mut elem_texts: Vec<String> = Vec::new();
        let mut next_elem_id: usize = 0;
        // 从随机基础字符串开始
        let base_len = rng.random_range(0..30);
        let mut base = String::new();
        for _ in 0..base_len {
            base.push_str(&rand_grapheme(&mut rng));
        }
        ta.set_text_clearing_elements(&base);
        // 为初始光标选择一个有效的字符边界
        let mut boundaries: Vec<usize> = vec![0];
        boundaries.extend(ta.text().char_indices().map(|(i, _)| i).skip(1));
        boundaries.push(ta.text().len());
        let init = boundaries[rng.random_range(0..boundaries.len())];
        ta.set_cursor(init);

        let mut width: u16 = rng.random_range(1..=12);
        let mut height: u16 = rng.random_range(1..=4);

        for _step in 0..60 {
            // 宽高大部分稳定，偶尔变化
            if rng.random_bool(0.1) {
                width = rng.random_range(1..=12);
            }
            if rng.random_bool(0.1) {
                height = rng.random_range(1..=4);
            }

            // 选择一个操作
            match rng.random_range(0..18) {
                0 => {
                    // 在光标处插入小的随机字符串
                    let len = rng.random_range(0..6);
                    let mut s = String::new();
                    for _ in 0..len {
                        s.push_str(&rand_grapheme(&mut rng));
                    }
                    ta.insert_str(&s);
                }
                1 => {
                    // 使用小的随机切片进行 replace_range
                    let mut b: Vec<usize> = vec![0];
                    b.extend(ta.text().char_indices().map(|(i, _)| i).skip(1));
                    b.push(ta.text().len());
                    let i1 = rng.random_range(0..b.len());
                    let i2 = rng.random_range(0..b.len());
                    let (start, end) = if b[i1] <= b[i2] {
                        (b[i1], b[i2])
                    } else {
                        (b[i2], b[i1])
                    };
                    let insert_len = rng.random_range(0..=4);
                    let mut s = String::new();
                    for _ in 0..insert_len {
                        s.push_str(&rand_grapheme(&mut rng));
                    }
                    let before = ta.text().len();
                    // 如果选择的范围与元素相交，replace_range 会扩展到
                    // 元素边界，因此朴素的大小增量断言不成立。
                    let intersects_element = elem_texts.iter().any(|payload| {
                        if let Some(pstart) = ta.text().find(payload) {
                            let pend = pstart + payload.len();
                            pstart < end && pend > start
                        } else {
                            false
                        }
                    });
                    ta.replace_range(start..end, &s);
                    if !intersects_element {
                        let after = ta.text().len();
                        assert_eq!(
                            after as isize,
                            before as isize + (s.len() as isize) - ((end - start) as isize)
                        );
                    }
                }
                2 => ta.delete_backward(rng.random_range(0..=3)),
                3 => ta.delete_forward(rng.random_range(0..=3)),
                4 => ta.delete_backward_word(),
                5 => ta.kill_to_beginning_of_line(),
                6 => ta.kill_to_end_of_line(),
                7 => ta.move_cursor_left(),
                8 => ta.move_cursor_right(),
                9 => ta.move_cursor_up(),
                10 => ta.move_cursor_down(),
                11 => ta.move_cursor_to_beginning_of_line(/*move_up_at_bol*/ true),
                12 => ta.move_cursor_to_end_of_line(/*move_down_at_eol*/ true),
                13 => {
                    // 使用唯一的哨兵负载插入一个元素
                    let payload =
                        format!("[[EL#{}:{}]]", next_elem_id, rng.random_range(1000..9999));
                    next_elem_id += 1;
                    ta.insert_element(&payload);
                    elem_texts.push(payload);
                }
                14 => {
                    // 尝试在现有元素内部插入（应截断到边界）
                    if let Some(payload) = elem_texts.choose(&mut rng).cloned()
                        && let Some(start) = ta.text().find(&payload)
                    {
                        let end = start + payload.len();
                        if end - start > 2 {
                            let pos = rng.random_range(start + 1..end - 1);
                            let ins = rand_grapheme(&mut rng);
                            ta.insert_str_at(pos, &ins);
                        }
                    }
                }
                15 => {
                    // 替换与元素相交的范围 -> 整个元素应被替换
                    if let Some(payload) = elem_texts.choose(&mut rng).cloned()
                        && let Some(start) = ta.text().find(&payload)
                    {
                        let end = start + payload.len();
                        // 创建一个相交的范围 [start-δ, end-δ2)
                        let mut s = start.saturating_sub(rng.random_range(0..=2));
                        let mut e = (end + rng.random_range(0..=2)).min(ta.text().len());
                        // 对齐到字符边界以满足 String::replace_range 契约
                        let txt = ta.text();
                        while s > 0 && !txt.is_char_boundary(s) {
                            s -= 1;
                        }
                        while e < txt.len() && !txt.is_char_boundary(e) {
                            e += 1;
                        }
                        if s < e {
                            // 小的替换文本
                            let mut srep = String::new();
                            for _ in 0..rng.random_range(0..=2) {
                                srep.push_str(&rand_grapheme(&mut rng));
                            }
                            ta.replace_range(s..e, &srep);
                        }
                    }
                }
                16 => {
                    // 尝试将光标设置在元素内部的位置；应截断出来
                    if let Some(payload) = elem_texts.choose(&mut rng).cloned()
                        && let Some(start) = ta.text().find(&payload)
                    {
                        let end = start + payload.len();
                        if end - start > 2 {
                            let pos = rng.random_range(start + 1..end - 1);
                            ta.set_cursor(pos);
                        }
                    }
                }
                _ => {
                    // 跳到单词边界
                    if rng.random_bool(0.5) {
                        let p = ta.beginning_of_previous_word();
                        ta.set_cursor(p);
                    } else {
                        let p = ta.end_of_next_word();
                        ta.set_cursor(p);
                    }
                }
            }

            // 健全性不变量
            assert!(ta.cursor() <= ta.text().len());

            // 元素不变量
            for payload in &elem_texts {
                if let Some(start) = ta.text().find(payload) {
                    let end = start + payload.len();
                    // 1）元素内文本与初始设置的负载相符
                    assert_eq!(&ta.text()[start..end], payload);
                    // 2）光标绝不会严格位于元素内部
                    let c = ta.cursor();
                    assert!(
                        c <= start || c >= end,
                        "cursor inside element: {start}..{end} at {c}"
                    );
                }
            }

            // 渲染并计算光标位置；确保其在边界内且不崩溃
            let area = Rect::new(0, 0, width, height);
            // 无状态渲染到足以容纳所有折行行的区域
            let total_lines = ta.desired_height(width);
            let full_area = Rect::new(0, 0, width, total_lines.max(1));
            let mut buf = Buffer::empty(full_area);
            ratatui::widgets::WidgetRef::render_ref(&(&ta), full_area, &mut buf);

            // cursor_pos：存在时 x 必须在宽度内
            let _ = ta.cursor_pos(area);

            // cursor_pos_with_state：始终位于视口行内
            let (_x, _y) = ta
                .cursor_pos_with_state(area, state)
                .unwrap_or((area.x, area.y));

            // 有状态渲染不应崩溃，并更新滚动
            let mut sbuf = Buffer::empty(area);
            ratatui::widgets::StatefulWidgetRef::render_ref(&(&ta), area, &mut sbuf, &mut state);

            // 折行后，desired height 等于无滚动渲染的行数
            let total_lines = total_lines as usize;
            // 内容适配区域高度时，state.scroll 不得超过 total_lines
            if (height as usize) >= total_lines {
                assert_eq!(state.scroll, 0);
            }
        }
    }
}
