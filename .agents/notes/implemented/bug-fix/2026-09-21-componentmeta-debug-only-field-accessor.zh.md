# Agent Note: ComponentMeta's debug-only field is read through one cfg-bridging accessor

Status: implemented

[English](2026-09-21-componentmeta-debug-only-field-accessor.md)

## Problem

`ComponentMeta.type_name` 只在 `#[cfg(debug_assertions)]` 下存在，而
`Archetype::borrow`、`Archetype::borrow_raw`、`Archetype::borrow_mut` 的
panic 消息无条件读取 `self.metas[column].type_name`。dev profile 下
`debug_assertions` 开启，可以编译；`cargo build --release` 以 E0609
（不存在字段 `type_name`）失败。CI 的 clippy 与 test 作业都在 dev
profile 下运行，门禁里没有任何不启用 `debug_assertions` 的构建。

## Decision

对该字段的全部读取都经由唯一的访问器 `ComponentMeta::type_name()`：
`debug_assertions` 下返回存储的名字，否则返回 `"<unknown>"`。
`assert_component_meta` 的重复组件 panic 由两个按 `cfg` 选择的分支
收敛为使用同一访问器的单分支。

## Alternatives considered

- **在 `T` 在作用域内的调用点直接调用 `core::any::type_name::<T>()`。**
  否决：在任何 profile 下都精确，但只有 `borrow` 和 `borrow_mut` 有
  `T`；`borrow_raw` 是类型擦除的，仍需自己的 `cfg` 分支，同一条消息
  留下两套机制。
- **保留逐调用点的 `cfg` 分支（`assert_component_meta` 原来的写法）。**
  否决：每条新 panic 消息都要重新实现一次 release 回退；访问器只拥有
  它一次。

## Consequences

- release 构建恢复可编译；其中的借用冲突与重复组件 panic 消息里组件
  名显示为 `"<unknown>"`。
- 给 `ComponentMeta` 再加 `debug_assertions` 门控字段时，扩展访问器即可，
  不需要在调用点补 `cfg` 分支。
- 门禁仍然不构建任何 release target；仅 release 可见的破坏对 CI 依然
  不可见。
