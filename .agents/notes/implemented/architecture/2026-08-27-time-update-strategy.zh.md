# Agent Note: TimeUpdateStrategy 把时钟推进移入调度

Status: implemented

[English](2026-08-27-time-update-strategy.md)

## Problem

在 runner/tick 对齐([2026-08-27-runner-and-tick-aligned-to-bevy.zh.md](2026-08-27-runner-and-tick-aligned-to-bevy.zh.md))之后,时钟推进是最后一个仍手工焊死在 winit runner 里的帧步骤:`run_frame` 在 `App::update` 前调用 `moonfield_time::update_time`。headless runner 完全没有时间;测试只能在每次 update 前手工改动 `Time<Virtual>` 来驱动时钟;而选定的长期形态——Bevy 的 `TimeUpdateStrategy`——仍不存在。

## Decision

- `moonfield-time` 新增 `TimeUpdateStrategy`(镜像 Bevy):默认 `Automatic` 读取系统时钟;`ManualInstant(Instant)`、`ManualDuration(Duration)`、`FixedTimesteps(u32)` 提供确定性来源。
- 新增 `time_update_system`,按策略推进 `Real → Virtual → 通用 Time`,走既有的 `update_time_with_instant`/`update_time_with_duration` 路径。它由 `TimePlugin` 注册进 `First` 调度;`TimePlugin` 现在也插入 `TimeUpdateStrategy` 资源。
- winit runner 的 `run_frame` 不再触碰时间;它只剩 `app.update()`(外加 redraw/exit 杂务)。headless 的 `run_updates` 首次自动推进时间。
- 固定步长测试改用 `ManualInstant` 驱动时钟:首帧播种 real 时钟锚点(零 delta),后续帧与之求差。由于手动路径会把原始 delta 灌进虚拟时钟,测试显式禁用 250 ms 的 `max_delta` clamp,以保留旧的"不 clamp"语义。

## Alternatives considered

- **时钟推进留在 runner。** 否决:headless runner 依旧没有时间,winit runner 也保留了非调度帧步骤——正是对齐提交在别处消除的东西。
- **仅当添加 `TimePlugin` 时注册系统。** 按此实现:`time_update_system` 是 `TimePlugin` 的系统;`update_time_with_*` 自由函数保留给一次性测试。

## Consequences

- 每个 runner——winit 或 headless——现在都从同一个地方(`First` 调度)获得每 tick 时钟推进。
- 测试与回放/网络路径通过 `TimeUpdateStrategy` 资源获得确定性时钟,不再需要在 update 前改动时钟。
- 虚拟时钟的 `max_delta` clamp 现在也作用于手动策略(此前 `advance_by` 绕过它);需要大步长的测试必须显式调高或禁用该 clamp。
- `update_time`/`update_time_with_instant`/`update_time_with_duration` 保持公开;自由函数仍会惰性插入缺失的时钟。