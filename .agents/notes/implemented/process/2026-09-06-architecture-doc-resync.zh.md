# Agent Note: resync architecture.md to the shipped tick model

Status: implemented

[English](2026-09-06-architecture-doc-resync.md)

## Problem

`docs/architecture.md` 已经与代码脱节。Frame loop 和 Time 两节仍在描述重构前的
设计——时钟由 winit 后端推进、渲染在 tick 之外驱动——与已随代码落地的模型相矛盾,
后者记录在
[runner and tick aligned to Bevy](../architecture/2026-08-27-runner-and-tick-aligned-to-bevy.md)
和
[TimeUpdateStrategy](../architecture/2026-08-27-time-update-strategy.md)
两篇笔记中。较小的偏差也在累积:splat 字段名为 `sh_dc`/`sh_rest`,不是
`f_dc`/`f_rest`;dock 标签页标题是 Outliner/Details/Content Browser,不是
Hierarchy/Inspector,资产加载和场景 Save/Load 位于 Content Browser 面板;还有一句
`GltfLoader` 的描述是残缺的重复句。另有两处机制没有任何文档归属:ECS 变更检测和
`moonfield-ml` 训练运行时。

## Decision

将 Frame loop 和 Time 两节改写为已落地的模型:一次 tick 依次是 `First`(消息缓冲
交换、时钟推进)→ 固定步长循环 → `Update` → 渲染管线 → `Last`;时钟由 `First`
中的 `time_update_system` 按 `TimeUpdateStrategy` 资源推进,后端从不接触时间。这两
节同时写明:编辑器二进制的插件栈没有添加 `TimePlugin`,因此没有任何东西推进它的时
钟。新增 Change detection 一节(逐组件 tick、`Ref`/`Mut` 包装器、`MAX_CHANGE_AGE`
钳制、运行时借用计数、没有 `Changed`/`Added` 过滤器)和 ML training 一节
(`moonfield-ml` 位于 app 框架之外、经 RHI 编译器做 Slang autodiff、COLMAP 文本解析
器、`Trainer::run` / `Adam::record_step` 仍是 `todo!()` 桩,实际跑通的 autodiff 路径
是 `gpu_tests::gaussian_fit` 测试)。修正面板名称、splat 字段名、残缺的
`GltfLoader` 句子、`App::update` 的过时文档注释,以及根 `AGENTS.md` 中
`moonfield-time` 的名录行。

## Alternatives considered

- **机制说明只留在 Agent Notes 里。** 否决:笔记记录决策,architecture.md 才是机制
  的汇总文档;读者需要一个能看到 tick、变更检测和 ML 运行时现状的地方。
- **删掉过时的章节而不是重写。** 否决:frame loop 和时间模型正是该文件引言承诺承载
  的机制;删掉留下的是空洞,不是修复。

## Consequences

- architecture.md 重新与其描述的代码一致;文档集与 implemented 笔记之间唯一的内部
  矛盾已消除。
- 变更检测和 ML 训练有了机制归属,其当前限制(无过滤器、循环仍是桩)作为事实写明。
- 文档漂移没有 CI 门禁;发现它仍依赖评审和定期审计。
