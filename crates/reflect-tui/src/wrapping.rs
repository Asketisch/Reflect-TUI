use unicode_width::UnicodeWidthStr;

pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    for ch in text.chars() {
        let w = ch.to_string().width();
        if !row.is_empty() && row.width() + w > width {
            rows.push(std::mem::take(&mut row));
        }
        row.push(ch);
    }
    if !row.is_empty() || text.is_empty() {
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragments_and_resize_are_safe() {
        assert_eq!(wrap("😀你好", 4), vec!["😀你", "好"]);
        assert!(wrap("abc", 0).iter().all(|s| s.width() <= 1));
    }
}
