# Pull Request

> ⚠️ 请阅读 [CONTRIBUTING.md](../blob/main/CONTRIBUTING.md) 后再提 PR。
> 本仓库**不接受纯英文 commit message / PR 描述**。

## 变更概要

<!-- 用一两句话说明这次改动解决什么问题,或新增什么能力 -->

## 改动类型

- [ ] Bug 修复(非破坏性,修复 issue #___)
- [ ] 新功能(非破坏性,新增 ___)
- [ ] 重构(无功能变化)
- [ ] 文档 / 注释
- [ ] 构建 / CI / 工具链
- [ ] Submodule 升级(reflect-agent 指针变更,**请单独提交**)
- [ ] 其他:

## 关联 Issue / 上下文

<!-- 关联的 issue 编号、Discussions、设计文档 -->

## 自检清单

提交前请确认:

- [ ] 本地运行 `cargo fmt --all -- --check` 通过
- [ ] 本地运行 `cargo clippy --workspace --all-targets -- -D warnings` 通过
- [ ] 本地运行 `cargo test --workspace` 通过
- [ ] commit message **使用中文**
- [ ] PR 描述 **使用中文**
- [ ] 若涉及 submodule 升级,本 PR 仅含 submodule 指针变更 + 接线代码,
      不混入其他业务改动
- [ ] 新增功能附单元测试
- [ ] 涉及用户可见 UI / 命令的改动,在「行为变化」段描述清楚

## 行为变化(若有)

<!-- 新的 slash 命令 / 新配置项 / 行为差异 / 迁移指南 -->

## 截图 / 录屏(若有 UI 改动)

<!-- 粘贴图片 -->

## 备注

<!-- 其他需要 reviewer 关注的事项 -->