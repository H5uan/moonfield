# Agent Note: Deferred swapchain retirement on recreate

Status: implemented

[English](2026-09-13-deferred-swapchain-retirement.md)

## Problem

`WindowSurfaceData::recreate` 在重建交换链之前先调用
`device.wait_idle()`。resize 与 surface lost 事件都走这条路径，因此拖动
窗口边缘时每帧都会让整个设备停摆一次——present、抽取和渲染全部堵在这一
个调用上。这个等待只为了一条 Vulkan 规则：交换链不得在 GPU 仍在处理的
帧还在 present 它时被销毁。全设备 idle 是该规则最粗暴的见证方式。

## Decision

`recreate` 不再 idle 设备（`crates/moonfield-render-core/src/window.rs`）：

- 旧交换链通过 rhi 的 `Swapchain::succeed` 作为 `oldSwapchain` 传给驱动，
  该接口创建替代交换链的同时把旧对象留在调用方手中。调用会使旧交换链
  退休（即使失败亦然）：它不能再被 acquire，但已 acquire 的图像仍可
  present，且一个 surface 可挂任意多个退休交换链——规范约束的是同一
  native window 只能有一个*非退休*交换链，因此在旧交换链存活期间不带提示
  地创建新交换链是不合法的。由于 `succeed` 失败时旧交换链同样被退休，
  `swapchain_retired` 标志把重试引导到无提示的 `Swapchain::new`（此时
  合法：不存在会被重复传递的非退休交换链）。
- 旧交换链与旧深度缓冲进入每窗口的退休列表（`RetiredSwapchain`），并携带
  退休时刻帧循环的已提交帧计数（`FrameContext::presented_frames`）。
- `create_window_surfaces` 每 tick 排空该列表：当
  `presented_frames >= retired_at + MAX_FRAMES_IN_FLIGHT` 时销毁对应条目。
  帧循环在 `begin_frame` 中的在飞等待（timeline 值
  `frame_submitted - MAX_FRAMES_IN_FLIGHT`）保证此时退休时刻之前提交的
  所有帧——唯一还可能 present 旧交换链的帧——都已在队列上完成。这正是
  `wait_idle` 提供的保证，只是范围收窄到真正相关的帧。
- 退休对象持有 `Arc<DeviceShared>` 保活（见
  [Shared device context for GPU object lifetimes](2026-09-13-shared-device-context-lifecycle.md)），
  因此它们无论在何时销毁都是安全的。

recreate 被编排到窗口必然空闲的唯一时刻：`create_window_surfaces` 在
`acquire_window_frames` *之前*运行，此时没有任何窗口持有已 acquire 的图像
（上一 tick 的 present 已将其取走）。因此交换绝不跨越在飞的 present，同一
tick 的 acquire 直接落在新的交换链上，resize 不产生空白 tick。acquire 把
suboptimal 视为可用——present 照常进行，由驱动缩放图像——因此拖动期间
窗口每 tick 都有 present；只有硬失败（`ERROR_OUT_OF_DATE`）的 acquire 才
产生无 present 的 tick，交换链在下一 tick 开始时重建。

演进记录：第一版用裸 `Swapchain::new`（不带 `oldSwapchain`）创建替代交换链
——同一 surface 出现两个非退休交换链，违反规范，驱动会剥离旧图像（闪
黑）。第二版把 recreate 保持在 acquire 之后，并跳过尺寸不匹配的窗口；用编
辑器的 `MOONFIELD_EDITOR_SIM_RESIZE` 钩子（每 tick 改写主窗口的缓存尺寸，
模拟拖动）实测，持续 resize 下**每一个** tick 都没有 present（900/900 帧
tick 无任何窗口 present）——操作系统合成器用窗口背景填充未 present 的区
域，这就是闪烁。把 recreate 移到 acquire 之前并正常 present suboptimal 图
像后，同一测量变为 0/900。

## Alternatives considered

- **扩展 rhi 让 `recreate` 返回旧交换链。** 能力上与 `succeed` 等价，但要改
  既有签名；独立的构造函数保留了「调用方先 idle」的 `recreate`（对会先
  idle 的调用方仍然有效），调用点可读性也更好。
- **继续用 `Swapchain::new` 创建替代交换链（不带提示）。** 在旧交换链存活
  期间这违反规范——同一 surface 两个非退休交换链——驱动会剥离旧图像，
  是闪烁的第一个成因。
- **跳过待重建窗口的 acquire。** 会产生无 present 的 tick，由合成器用窗口
  背景填充；实测持续 resize 下空白率 100%（闪烁的第二个成因）。suboptimal
  的 present 是规范允许且由驱动缩放的，没有理由跳过。
- **在帧中途、acquire 之后 recreate。** 会把从旧交换链 acquire 的 image
  index 通过新交换链句柄 present（并向其中从未 acquire 的图像录制渲染
  命令）——未定义内容。
- **复用 rhi 的 `RetirementRing` 存放整个交换链。** 退休环的
  `RetireAction` 词汇是 crate 内部的，表达的是销毁步骤（销毁
  view/image/buffer），而不是完整的公共对象；把存活的 `Swapchain` 推入
  其中需要新增公共接口。按窗口维护列表、以帧循环自身的计数为键更简单，
  且不需要改动 rhi 的退休环。
- **仅在 surface-lost 路径保留 `wait_idle`。** surface-lost 与 resize 共用
  `recreate`，约束同为「不得在 present 期间销毁」；部分保留等待只会在
  两条热路径之一上留下卡顿，而没有正确性收益。

## Consequences

- 窗口 resize 既不使设备停摆也不产生空白 tick：驱动拿到 `oldSwapchain`
  过渡提示，帧绝不跨越交换链，suboptimal 帧照常 present。每次 resize 的
  代价是一个交换链和一个深度缓冲的分配。
- 快速连续 resize 的窗口最多同时持有少量退休交换链（上限为 recreate 频率
  × `MAX_FRAMES_IN_FLIGHT` 帧）；每个都会在两个已提交帧内被确定性地销毁。
- rhi 新增一个公共构造函数 `Swapchain::succeed`；边界门禁通过（签名只含
  crate 自有类型）。`Swapchain::recreate` 保留「调用方须先 idle」的约定，
  目前工作区内无调用方。
- `create_window_surfaces` 改为在 `acquire_window_frames` 之前运行；其
  recreate 守卫 `!frame_in_progress()` 表达的是不变量，而不是推迟手段。
- 编辑器保留 `MOONFIELD_EDITOR_SIM_RESIZE=1` 钩子（与
  `MOONFIELD_EDITOR_AUTO_CLOSE` 并列），作为日后回归检查的 resize 复现器。
- render-core 的公共 API 未变：`recreate` 的 `presented_frames` 参数与
  `swapchain_retired` 标志都是私有的，下游 crate 无需改动。
