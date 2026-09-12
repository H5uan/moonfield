# Agent Note: Access-scoped bindless barriers, and the sync1 path removed

Status: implemented

[English](2026-09-12-access-scoped-barriers-and-sync2-only.md)

## Problem

`CommandBuffer::barrier` 原来接收一对 stage 加一个 `BarrierHazard` 枚举
（`Memory` / `Descriptors`），并且总是发出最宽的 access 掩码——两侧都是
`MEMORY_READ | MEMORY_WRITE`，`Descriptors` 只是在目标侧追加
`SHADER_SAMPLED_READ`。调用方无法表达一条 barrier 到底在排序什么，于是
render-core 的门（door）切换只能记录 `ALL → ALL` 的 stage 对，作为被接受
的过度同步。此外 crate 还保留着一条 legacy sync1 `pipeline_barrier`
（`vkCmdPipelineBarrier`），只用于 upload/offscreen 路径的 image 布局迁移——
与 sync2 并存的第二套 barrier API，掩码语义更弱。

## Decision

- `sync.rs` 新增 `Access` newtype，包装 `vk::AccessFlags2`，完全仿照
  `Stage`（关联常量、`|` 组合）：`NONE`、`INDIRECT_COMMAND_READ`、
  `SHADER_READ`、`SHADER_WRITE`、`SHADER_SAMPLED_READ`、
  `COLOR_ATTACHMENT_READ`/`COLOR_ATTACHMENT_WRITE`、
  `DEPTH_STENCIL_READ`/`DEPTH_STENCIL_WRITE`、`TRANSFER_READ`/
  `TRANSFER_WRITE`、`MEMORY_READ`/`MEMORY_WRITE`，以及 `ALL`
  （`MEMORY_READ | MEMORY_WRITE`）。`Stage` 新增 `ALL_GRAPHICS`。
- `CommandBuffer::barrier(before, before_access, after, after_access)`
  ——四个位置参数的 scope——仍然只发一条全局 `MemoryBarrier2`，但带上
  真实的 access 掩码。`BarrierHazard` 被删除；其 `Descriptors` 分支以
  目标侧 access 中的 `SHADER_SAMPLED_READ` 形式保留在所有消费方会采样
  descriptor heap 的调用点（在 Vulkan 的 access 层级里 `SHADER_READ`
  已包含它；各调用点仍显式写出）。
- render-core 的门切换改为按作用域记录：compute→rendering 是
  `(COMPUTE, SHADER_WRITE) → (VERTEX | FRAGMENT, SHADER_READ |
  SHADER_SAMPLED_READ)`（vertex pulling 加 heap 采样）；rendering→compute
  是 `(ALL_GRAPHICS, COLOR_ATTACHMENT_WRITE | DEPTH_STENCIL_WRITE |
  SHADER_WRITE) → (COMPUTE, SHADER_READ | SHADER_SAMPLED_READ |
  SHADER_WRITE)`；rendering→rendering 用同一生产侧配对 `(ALL_GRAPHICS,
  attachment 读写 | SHADER_READ | SHADER_SAMPLED_READ)`；dispatch 链是
  `(COMPUTE, SHADER_WRITE) → (COMPUTE, SHADER_READ | SHADER_WRITE)`。
  `ALL_GRAPHICS` 是光栅 pass 上有意的 stage 放宽：`Stage` 没有
  fragment-test 和 attachment 常量，而 access 掩码保持如实。
- sync1 路径移除。image 布局迁移（uploader 的初始化/上传、offscreen 的
  初始化/回读）改经 crate 内部的 `CommandBuffer::image_barriers` 发出
  sync2 `vk::ImageMemoryBarrier2`；`pipeline_barrier` 被删除。crate 内
  不再有任何 `vkCmdPipelineBarrier`。
- 纹理初始化迁移保持逐条发出，不做批量：uploader 中每个迁移都与对应的
  copy 交错记录，而"设备持有 pending 列表、在命令缓冲 begin 时统一排空"
  的批量方案会为了每纹理省一条 barrier 而重新设计 uploader/offscreen
  的记录流程。

## Alternatives considered

- **用两个 scope 结构体（`Scope { stage, access }`）代替四个位置参数。**
  为一种调用形状新增两个类型；四参数形式与参考 API 一致，调用点也更
  平直。
- **设备持有的 `UNDEFINED → GENERAL` 初始化批量列表。** 批量能为每个
  纹理省一条 barrier，但迫使 uploader 与 offscreen 共享一个 pending
  列表；让每个迁移紧邻自己的 copy 能保持记录流程线性。
- **更细的光栅 stage（`COLOR_ATTACHMENT_OUTPUT`、fragment-test 各阶段）
  代替 `ALL_GRAPHICS`。** 需要新增常量并逐一与 access 掩码正确配对，
  却没有行为收益——真实的 hazard 已由 access 掩码表达，而
  `ALL_GRAPHICS` 仍然排除了 compute 和 transfer。

## Consequences

- 每个 barrier 调用点都写明自己的 hazard；stage/access 配对错误会触发
  sync2 校验失败，而不是悄悄地过度同步。
- render-core 不再插入 `ALL → ALL` barrier；涉及光栅的切换按设计放宽到
  `ALL_GRAPHICS`，并在各门的注释中说明。
- crate 每种形状只有一个 barrier 入口，且都是 sync2：全局内存
  `barrier`（公开）与 `image_barriers`（crate 内部）。
- bindless barrier 的 GPU 测试改为按 access 掩码变体（shader 读写、
  sampled 读）组织，不再使用已删除的 hazard 枚举。
