//! 为 vendored 代码提供兼容垫片,这些代码使用 `rand 0.9` 风格的
//! `random_range()` API,而本 crate 使用 `rand 0.8`(仅暴露 `gen_range()`)。
//!
//! 在调用 `rng.random_range(...)` 的模块中添加
//! `use crate::random_range_compat::*;` 即可让这些调用解析通过。

use rand::Rng;

/// 为 `rand 0.8` 的 RNG 类型扩展 `random_range` 的扩展 trait。
pub trait RandomRangeCompat {
    fn random_range<T, R>(&mut self, range: R) -> T
    where
        T: rand::distributions::uniform::SampleUniform,
        R: rand::distributions::uniform::SampleRange<T>;

    fn random_ratio(&mut self, numerator: u32, denominator: u32) -> bool {
        self.random_range(0..denominator) < numerator
    }

    fn random_bool(&mut self, probability: f64) -> bool {
        assert!(probability >= 0.0 && probability <= 1.0);
        self.random_range(0.0..1.0) < probability
    }
}

impl<R: Rng> RandomRangeCompat for R {
    fn random_range<T, RR>(&mut self, range: RR) -> T
    where
        T: rand::distributions::uniform::SampleUniform,
        RR: rand::distributions::uniform::SampleRange<T>,
    {
        self.gen_range(range)
    }
}
