//! 用于模糊过滤的简单大小写不敏感子序列匹配器。
//!
//! 第一个参数是被搜索的原文本(haystack);
//! 第二个参数是要作为子序列匹配的关键词(needle)。
//! 返回匹配字符在**原始** `haystack` 中的索引(字符位置)及一个分数,
//! 分数越小越好。

pub fn fuzzy_match(haystack: &str, needle: &str) -> Option<(Vec<usize>, i32)> {
    if needle.is_empty() {
        return Some((Vec::new(), i32::MAX));
    }

    let mut lowered_chars: Vec<char> = Vec::new();
    let mut lowered_to_orig_char_idx: Vec<usize> = Vec::new();
    for (orig_idx, ch) in haystack.chars().enumerate() {
        for lc in ch.to_lowercase() {
            lowered_chars.push(lc);
            lowered_to_orig_char_idx.push(orig_idx);
        }
    }

    let lowered_needle: Vec<char> = needle.to_lowercase().chars().collect();

    let mut result_orig_indices: Vec<usize> = Vec::with_capacity(lowered_needle.len());
    let mut last_lower_pos: Option<usize> = None;
    let mut cur = 0usize;
    for &nc in lowered_needle.iter() {
        let mut found_at: Option<usize> = None;
        while cur < lowered_chars.len() {
            if lowered_chars[cur] == nc {
                found_at = Some(cur);
                cur += 1;
                break;
            }
            cur += 1;
        }
        let pos = found_at?;
        result_orig_indices.push(lowered_to_orig_char_idx[pos]);
        last_lower_pos = Some(pos);
    }

    let first_lower_pos = if result_orig_indices.is_empty() {
        0usize
    } else {
        let target_orig = result_orig_indices[0];
        lowered_to_orig_char_idx
            .iter()
            .position(|&oi| oi == target_orig)
            .unwrap_or(0)
    };
    // 单击匹配时 last 默认为 first;分数 = 首次/末次命中之间的额外跨度
    // 减去 needle 长度(≥0)。
    // 当首命中位于索引 0 时,通过减去 100 强力奖励前缀匹配。
    let last_lower_pos = last_lower_pos.unwrap_or(first_lower_pos);
    let window =
        (last_lower_pos as i32 - first_lower_pos as i32 + 1) - (lowered_needle.len() as i32);
    let mut score = window.max(0);
    if first_lower_pos == 0 {
        score -= 100;
    }

    result_orig_indices.sort_unstable();
    result_orig_indices.dedup();
    Some((result_orig_indices, score))
}
