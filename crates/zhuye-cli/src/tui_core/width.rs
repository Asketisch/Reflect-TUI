//! 带固定前缀列的 transcript 渲染宽度护栏。
//!
//! 多条渲染路径在布局内容前会为项目符号、装订线或标签
//! 预留固定列数。当终端非常窄时，这些预留列可能耗尽整个宽度，
//! 使内容可用的空间为零或负数。
//!
//! 这些辅助函数集中处理减法并强制执行严格正数
//! 契约：返回 `Some(n)`（`n > 0`），当没有可用内容宽度时返回 `None`。
//! 调用方把 `None` 当作“仅渲染前缀的回退”，而不是在零宽度下
//! 尝试折行渲染，那样会产生空或不稳定的输出。

/// 预留固定列后返回可用的内容宽度。
///
/// 保证严格的宽度为正数（`Some(n)` 且 `n > 0`），或当
/// 预留列耗尽全部宽度时返回 `None`。
///
/// 把 `None` 当作“仅渲染前缀的回退”。把它强制转为 `0` 并仍
/// 尝试折行渲染，在极窄终端宽度下通常会产生空或不稳定的输出。
pub(crate) fn usable_content_width(total_width: usize, reserved_cols: usize) -> Option<usize> {
    total_width
        .checked_sub(reserved_cols)
        .filter(|remaining| *remaining > 0)
}

/// [`usable_content_width`] 的 `u16` 便捷包装。
///
/// 这让以 `u16` 接收终端尺寸的调用点保持宽度运算的统一，
/// 同时保留宽度耗尽时的相同 `None` 契约。
pub(crate) fn usable_content_width_u16(total_width: u16, reserved_cols: u16) -> Option<usize> {
    usable_content_width(usize::from(total_width), usize::from(reserved_cols))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn usable_content_width_returns_none_when_reserved_exhausts_width() {
        assert_eq!(
            usable_content_width(/*total_width*/ 0, /*reserved_cols*/ 0),
            None
        );
        assert_eq!(
            usable_content_width(/*total_width*/ 2, /*reserved_cols*/ 2),
            None
        );
        assert_eq!(
            usable_content_width(/*total_width*/ 3, /*reserved_cols*/ 4),
            None
        );
        assert_eq!(
            usable_content_width(/*total_width*/ 5, /*reserved_cols*/ 4),
            Some(1)
        );
    }

    #[test]
    fn usable_content_width_u16_matches_usize_variant() {
        assert_eq!(
            usable_content_width_u16(/*total_width*/ 2, /*reserved_cols*/ 2),
            None
        );
        assert_eq!(
            usable_content_width_u16(/*total_width*/ 5, /*reserved_cols*/ 4),
            Some(1)
        );
    }
}
