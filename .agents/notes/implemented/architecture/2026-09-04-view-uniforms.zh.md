# Agent Note: ViewUniforms — per-view data has one home

Status: implemented

[English](2026-09-04-view-uniforms.md)

## Problem

没有任何帧级/视图级数据抵达 GPU。view-projection 被 CPU 折进每个 draw 的
MVP——每个 item 每个视图乘一次矩阵，然后经 queue item、帧快照、arena 记录
拷贝三道——而片元着色器的光照方向是硬编码常量。lights、time、fog 无处安放，
GPU culling（splat 路线图）又需要 GPU 侧拿到 view-projection。

## Decision

- `ViewUniforms { view_proj, view_pos }` 是每视图记录：每 pass 一条，写进
  帧绘制 arena，其地址通过反射出的 `Ptr<ViewUniforms>` placement 每 pass
  推送一次。lights、time、fog 都在这个结构体里生长。
- `DrawData` 收缩为 `{ model, color }`——每 draw 数据只描述物体本身，不描述
  相机。queue 存 model 矩阵而不再相乘；pass 从 target 的真实 extent 计算
  `view_proj`（queue 不再读 `RenderTargetSizes`）。
- 顶点着色器在 GPU 上组合 `view_proj * model * position`。core 3D 像素测试
  原样通过——乘法挪了位置，结果没有变。

## Alternatives considered

- **CPU 侧 MVP 保留到 GPU-driven 重构。** lights 和 time 继续无家可归，
  GPU culling 继续被阻塞；本改动与顶点拉取独立，可单独落地。
- **视图 uniform 走 push data 内联。** 每 pass 80 字节对 8 字节：arena 记录
  对每 draw 零成本，还把 push-data 预算留给每 draw 数据。

## Consequences

- 每 draw 的 CPU 成本每 item 每视图少一次 4×4 乘法，`RenderTargetSizes`
  只在 target 确保处被读取。
- `Ptr<T>` pointee 的布局是 Slang 的 natural（类 C）布局——生成的 SPIR-V 把
  pointee 类型命名为 `..._natural`（偏移烙进指针算术），而入口参数块是
  `EntryPointParams_std430`。Rust 镜像就是逐字段对齐的普通 `#[repr(C)]`；
  `two_pointer_roots_and_ptr_struct_layout` 钉住偏移与双指针根形状。
