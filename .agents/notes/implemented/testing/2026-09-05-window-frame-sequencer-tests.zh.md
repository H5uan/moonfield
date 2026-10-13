# Agent Note: Window frame sequencer extracted for GPU-free tests

Status: implemented

[English](2026-09-05-window-frame-sequencer-tests.md)

## Problem

`moonfield-render-core` 的 `WindowSurfaceData::acquire`/`submit` 承载着窗口帧循环的
整数状态机——帧槽计算 `(frame_submitted - 1) % MAX_FRAMES_IN_FLIGHT`、timeline 等待值
`frame_submitted - MAX_FRAMES_IN_FLIGHT`、timeline signal 值、in-progress 标志和
recreate 标志——与 Vulkan 调用交织在一起，没有 GPU 就无法测试。而工作区的 GPU 测试
在没有兼容驱动的机器上全部跳过，导致这套状态机在任何地方都没有被执行过的覆盖。

## Decision

序列状态与算术现在收敛到 `window.rs` 内的纯数据 `FrameSequencer`：`plan_acquire`
（槽位 + 等待值，重复 acquire 返回 `None`）、`note_acquired`、`note_out_of_date`、
`note_recreated`、`take_for_submit`（image + 槽位 + signal 值）、`finish_submit`
及读取访问器。`WindowSurfaceData` 保留 Vulkan 调用，并将其穿插在 sequencer 的状态
转换之间，顺序与副作用完全不变——这是一次忠实抽取，不是重新设计。

七个单元测试穷举覆盖这套算术：环填满前不等待、环回绕后的等待值、槽位循环、
signal 值等于帧号、重复 acquire 拒绝、out-of-date 中止不推进计数器、
suboptimal/recreate 标志生命周期、以及无 acquire 直接 submit 的 panic 契约。

## Alternatives considered

- **在 trait 后面 mock swapchain/device，直接测 `WindowSurfaceData`。** 否决：RHI 的
  句柄类型是具体的 `ash` 封装、没有 trait 接缝，专为测试造一个抽象是生产代码并不
  需要的负担。
- **靠编辑器集成测试覆盖。** 否决：帧边界竞态和 off-by-one 等待在能正常 present 的
  demo 里几乎不会触发；这里的失败模式是静默损坏，不是崩溃。
- **等有 GPU 的 CI 再说。** 否决：这套算术根本不需要 GPU；把它的覆盖与驱动可用性
  绑在一起是把两个独立问题混为一谈（CI 没有兼容 GPU 的问题依旧存在）。

## Consequences

- `cargo test -p moonfield-render-core` 现在在任何机器上（有无 GPU 均可）都会执行
  帧循环序列逻辑（新增 7 个测试，共 11 个）。
- 抽取过程中发现一个既有异常，现已修复：`queue_present` 硬错误（非 `SurfaceOutOfDate`）
  返回时，timeline 已被 signal 但 `frame_submitted` 不推进，下一次 `submit` 会重复
  signal 同一个 timeline 值——对 timeline 信号量这是非法的。现在 `submit` 在队列提交
  成功后立即调用 `finish_submit`；`presented_frames` 统计的是已提交的帧（present 本身
  可能已失败）。
- `FrameSequencer` 是 `window.rs` 私有类型；公开 API 无变化。
