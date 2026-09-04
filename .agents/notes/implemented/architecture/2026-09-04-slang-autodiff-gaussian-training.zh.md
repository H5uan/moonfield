# Agent Note: Gaussian Splatting trains on Slang autodiff over the Lunar Mare RHI

Status: implemented

[English](2026-09-04-slang-autodiff-gaussian-training.md)

## Problem

Gaussian Splatting 计划(朴素 3DGS、2DGS、Stoch3DGS 复现,以及 ReSTIR 接入)需要一条训练路径:可微渲染 kernel 加优化器循环。候选方案是 burn 深度学习框架(wgpu/cubecl 后端)和编译到 SPIR-V、直接跑在 Lunar Mare 上的 Slang autodiff。burn 会引入第二个 GPU device 世界——它的 wgpu device 无法与 RHI 的 ash device 共享 Vulkan 显存,每个训练步都要在两个 device 之间走 CPU 往返。

## Decision

训练与渲染运行在同一个 Vulkan device 上。可微 kernel 用 Slang 编写(`[Differentiable]`、`bwd_diff`,涉及全局内存副作用处用自定义 `[BackwardDerivative]` 包装),在运行时经 RHI 的 `Compiler` 编译为 SPIR-V,以 compute 方式 dispatch。框架本白给的训练运行时——Adam、loss kernel、checkpoint——改为手写;对 Gaussian Splatting 而言参数集本身就是模型,autodiff 图只有一层,框架能带来的东西很少。

`crates/moonfield-rhi/tests/gaussian_fit.rs` 这条 spike 端到端验证了该路径:64 个 2D 高斯由 Slang 生成的 backward kernel 与手写 Adam compute kernel 拟合 128×128 目标图,600 次迭代,loss 从 2662.11 降到 35.96(比值 0.0135),多次运行结果逐位一致,耗时约 1.4 秒。Slang v2026.12 接受 `IDifferentiable` struct、`no_diff` 参数,以及对 exp/sigmoid/旋转矩阵数学的 `bwd_diff`,全程无 SPIR-V 或 Vulkan validation 错误。

## Alternatives considered

- **burn + wgpu 后端。** 拒绝:burn 的 device 是 wgpu device,无法与 RHI 的 ash device 统一;tensor 与 RHI buffer 每个训练步都要经 CPU 内存交换。burn 的核心价值——深层 autodiff 图、kernel autotune、后端可移植性——覆盖的是 Gaussian Splatting 训练并不存在的需求。
- **burn + LibTorch/CUDA 后端仅用于训练。** 拒绝:原生部署更重,而这一训练循环的 kernel 无论如何都要手写(3DGS 参考实现的 backward 也是手写 CUDA kernel),双 device 割裂依旧存在。
- **SlangPy 或 slang-torch。** 拒绝:两者都是面向 Python/PyTorch 用户的宿主侧绑定;slang-torch 已废弃且仅支持 CUDA,而本 workspace 由 Rust 经 Lunar Mare 驱动 shader。

## Consequences

- 单一 device 世界:训练参数、梯度与渲染资源共享同一个 Vulkan device 和内存模型,训练状态可就地渲染,Stoch3DGS 的 estimator 可以作为同一个 Slang module 被训练与渲染两条路径共享。
- 仓库自持训练运行时(Adam、学习率调度、checkpoint 序列化、densification 簿记),没有框架依赖,也没有版本升级要跟。
- 梯度 scatter 不得依赖浮点原子操作:spike 的逐(像素, 高斯)梯度 buffer 加 reduction kernel 的规模是 像素数 × 高斯数,到真实 3DGS 规模时需要 warp 级归约或 `VK_EXT_shader_atomic_float`。
- burn 不是 workspace 依赖。若后续特性需要超出 kernel 内 Slang autodiff 能力的真正神经网络,届时再重新评估。
