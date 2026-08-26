//! 努力点火（spark）动画的测试。
//!
//! 上游模块带有像素级 insta 快照测试，依赖 ratatui 的 `Buffer` 渲染路径
//! 以及 `pretty_assertions` 内部实现，而 Reflect 移植版并未以相同方式使用它们。
//! 本文件刻意保留为编译容器占位，以便 `effort_ignition.rs` 中的 `mod tests;`
//! 声明能够解析；随着动画趋于稳定，可在这里补充具体的 Reflect 侧测试。

#![allow(dead_code)]
