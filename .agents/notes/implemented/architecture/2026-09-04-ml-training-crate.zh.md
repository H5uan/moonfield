# Agent Note: moonfield-ml hosts training methods; splat domain types stay in render-feature

Status: implemented

[English](2026-09-04-ml-training-crate.md)

## Problem

Slang autodiff 训练路径(见
[2026-09-04-slang-autodiff-gaussian-training](2026-09-04-slang-autodiff-gaussian-training.md))
需要在 workspace 中落地。Gaussian Splatting 是第一个训练方法但不是最后一个——后续可能接入 burn 或其他 ML 方法——而 `moonfield-render-feature` 已经持有 splat 领域类型(`GaussianScene`、`SplatCloud`、COLMAP / glTF I/O),且原有占位模块计划把训练放进渲染 crate 内。

## Decision

新 crate `moonfield-ml` 是训练运行时:`Trainer`/`TrainingMethod` 循环脚手架、Adam 优化器 kernel 接线、`Dataset`/`Checkpoint` trait 均为方法无关;每个训练方法占一个模块——`gs`(Gaussian Splatting:3DGS、2DGS、Stoch3DGS)是第一个。该 crate 以 `features = ["splat"]` 依赖 `moonfield-render-feature` 获取领域类型;render-feature 保持无训练代码。训练 shader 放在 `assets/shaders/ml/`(Adam 等方法无关 kernel)和 `assets/shaders/gs/`(训练与渲染共享的高斯数学)。headless 训练入口是 example 目标(`cargo run -p moonfield-ml --example train`);编辑器仍是 workspace 唯一的 binary。

## Alternatives considered

- **GS 专用 crate(`moonfield-gsplat`)。** 拒绝:后续 burn 或其他 ML 方法会需要第二个落脚点;运行时脚手架(优化器、训练循环、checkpoint)从一开始就是方法无关的。
- **训练放在 `render-feature::splat` 内(原占位方案)。** 拒绝:它把训练状态耦合进 render-feature crate,模糊了 workspace 想要的划分——领域类型属于 render-feature,训练方法属于 ML crate。
- **在 `moonfield-ml` 中复制 splat 领域类型。** 拒绝:`GaussianScene` 的 SoA 布局契约已由 render-feature 的 glTF I/O 持有;两份契约会漂移。

## Consequences

- 依赖方向为 `moonfield-ml` → `moonfield-render-feature`;渲染 crate 不感知训练的存在,编辑器(可同时依赖两者)日后承载训练面板无需重构。
- 2DGS 与 Stoch3DGS 作为 `ml::gs` 下的兄弟 kernel 族落地,共享 `TrainableScene` 与 Adam 接线。
- `GaussianScene` 的 SoA 布局是训练、导出与渲染共同遵守的单一契约。
