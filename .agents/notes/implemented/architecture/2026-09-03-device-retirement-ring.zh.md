# Agent Note: Device retirement ring for deferred GPU teardown

Status: implemented

[English](2026-09-03-device-retirement-ring.md)

## Problem

bindless 资源以裸值寻址——push data 里的 buffer device address、root data 里的
heap slot 索引——帧一旦提交，CPU 侧无从判断 GPU 是否仍引用某个资源。RHI 此前
在 `Drop` 里立即销毁 buffer、allocation 和 image，把安全契约压给调用方（"the
caller defers destruction past the in-flight frames"）。每个消费者只能手工兑现
或直接停顿：`OffscreenTarget::resize` 每次都让整个设备 idle，egui 后端自备了
一套按帧槽的延迟释放环，而各处 buffer 替换路径（bump arena 换块、egui 顶点
buffer 增长、prepared-mesh 剔除）销毁的 buffer 仍可能被在飞帧读取。

## Decision

- `Device` 持有 `RetirementRing`：每个帧槽一条 teardown 队列，存放原子化的
  `RetireAction`（销毁 buffer 与 image、归还 heap slot），由各资源的 `Drop` 组合压入。
- 受覆盖的资源——`Buffer`、`GpuAllocation`、bump arena 的块、`Texture`、
  `OffscreenTarget`——`Drop` 时把 teardown 压入当前帧槽而非就地销毁。`Device::begin_gpu_frame` 排空帧循环
  即将录制的那个槽：in-flight timeline 的 wait 已保证该槽上一次提交完成。
  `Device::flush_retirements` 为测试与析构排空全部槽，仅在 GPU idle 时可调用。
- `Device::drop` 先让设备 idle，再 drop 懒建的 uploader 与 descriptor heap
  单例使其后备 allocation 入环，然后排空——所有 teardown 都发生在
  `vkDestroyDevice` 之前，而不是在它之后的字段析构阶段。

## Alternatives considered

- **销毁前后让整个设备 idle。** 正确但冻结 GPU；resize 路径每次拖动视口都要
  付出这个代价。
- **每个资源一把 fence。** 能逐资源跟踪，但同步对象成倍增加，且对 heap slot
  的复用顺序没有约束力。
- **GPU 侧引用计数。** untyped bindless 模型让裸指针和 slot 索引穿过 shader，
  没有可以挂计数的钩子。

## Consequences

- `Buffer`、`GpuAllocation`、bump arena 块、`Texture`、`OffscreenTarget` 的
  teardown 在 drop 之后 `RETIRE_RING` 帧执行；在飞帧按构造读到完好内存，
  buffer 替换路径不再依赖调用方自律。
- bump allocator 在裸 `ash::Device` 之外另持一个 `RetirementRing` 句柄（其
  块构造是 lifetime-free 的，拿不到 `&Device`）。
- 帧循环驱动 ring：`acquire` 排空即将录制的槽（在等待 in-flight timeline
  之后），`submit_window_frames` 在帧命令缓冲之前 flush 共享 uploader——
  同队列的提交顺序让上传先行执行。`RenderPlugin` 断言
  `MAX_FRAMES_IN_FLIGHT == RETIRE_RING`。
- 排空在 ring 锁外执行，且 `drain_all` 循环到静止：teardown 可能级联——
  某个 action 释放最后一个 `Arc<DescriptorHeap>` 时，heap 的后备 allocation
  随之入环。
- `OffscreenTarget::resize` 为新 image 一并分配新 heap slot；旧槽与旧 image
  走 retire。heap descriptor 只在创建时写一次、永不重写，resize 路径不再
  让整个设备 idle。持有者在 `texture_handle` 变化时重新注册——编辑器的
  视口绑定随之刷新。
- egui 后端按帧槽的延迟释放环已删除：纹理的 drop 与释放统一走 ring，
  上传搭乘共享 uploader。
- 设备析构顺序固定：idle、drop 懒建单例、排空 ring、拆除 allocator、销毁
  设备。
