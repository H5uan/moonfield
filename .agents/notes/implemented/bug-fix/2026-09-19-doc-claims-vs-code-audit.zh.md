# Agent Note: Documentation corrections from a docs-vs-code audit

Status: implemented

[English](2026-09-19-doc-claims-vs-code-audit.md)

## Problem

一次"文档对代码"的审计发现，多处注释与文档描述了代码并未实现的机制：

1. `moonfield-ecs` 变更检测模块文档声称存在按系统的 `(last_run, this_run)` 窗口，
   以及在 `CHECK_TICK_THRESHOLD` 处周期性的 tick 钳制/重扫。实际上世界只持有一个
   全局 `(last_change_tick, change_tick)` 窗口供所有 tick 感知访问器共用，且
   `increment_change_tick` 只是裸的 `wrapping_add(1)`——没有钳制，没有重扫
   （`Tick::check_tick` 存在但无调用者）。
2. `moonfield-time` 的 `Time` 文档声称没有 fixed-update 调度、通用时钟从不切换为
   固定时钟。实际上 `run_fixed_main_schedule` 存在，并在每次 fixed 迭代期间把
   `Time<Fixed>` 镜像进通用 `Time`，循环结束后恢复 virtual 时钟。同一文档还把
   推进时钟归功于窗口后端；实际由 `First` 调度中的 `time_update_system` 完成。
3. `moonfield-math` 的 `Aabb3d`/`BoundingSphere` 文档声称可作为 `vec3` 对的
   storage buffer 结构体直接上传，与同 crate 的 `gpu` 模块自相矛盾：Rust 把两个
   `Vec3` 紧凑排成 24 字节，而 std430 会把第二个 `vec3` 填充到偏移 16（共 32 字节）。
4. `BoundingSphere::merge` 的文档写作"最紧"球体；中点加最大半径的公式只是
   *以中点为球心*的最紧球，并非最小包围球。
5. core 3D pass 的注释写着"背面剔除"，代码设置的却是 `CullMode::None`。

## Decision

逐处改写为描述已交付机制的文字，不去补实现缺失的机制（如需要，那是独立的工作）。
`Vec3` 对齐规则仍归 `moonfield-math::gpu` 所有；体积类型的文档现在只说明字节是
`Pod` 可上传的，且着色器侧声明才是布局的唯一事实来源。pass 注释以中立措辞说明
双面光栅化。`message.rs` 的模块文档和 `docs/architecture.md` 的变更检测一节经重新
核对无需修改（那里没有声称按系统的读取窗口；`MAX_CHANGE_AGE` 钳制与不存在
`Changed<T>`/`Added<T>` 过滤器均属实）。

## Alternatives considered

- **改为实现缺失的机制（按系统窗口、tick 重扫、背面剔除）。** 超出本次范围：
  每一项都是需要单独设计的行为变更，而当前的全局窗口与双面光栅化是正常工作的
  已交付行为。
- **只删掉夸大的句子而不补写。** 否决：真实机制（世界全局窗口、在
  `MAX_CHANGE_AGE` 处的回绕钳制、std430 不匹配）恰恰是该文档存在的理由——
  非显而易见的知识。

## Consequences

- `change_detection.rs`、`time.rs`、`real.rs`、`volumes.rs`、`core_3d/pass.rs`
  的文档与代码一致；`docs/architecture.md` 与 `message.rs` 经确认原文即准确。
- `CHECK_TICK_THRESHOLD` 的文档现在只说明它是 `MAX_CHANGE_AGE` 的比例因子；
  若日后移植 tick 重扫，其文档须从新代码重新推导。
