# Agent Note: Closing the rhi public API — no backend types escape

Status: implemented

[English](2026-09-05-rhi-public-api-boundary-closure.md)

## Problem

`moonfield-rhi` 的 AGENTS.md 禁止在公开 API 中泄漏 `Vk*` 句柄，但这条规则是文字，
代码是另一回事：约 15 处逃生口（`raw()`/`from_raw`/`borrow_raw` 访问器、公开签名中的
`vk::` 类型、`From<ash::vk::Result> for Error`）。引擎层穿过它们完成三件 RHI 从未
提供一等 API 的操作（present 支持校验、带旧句柄的 swapchain 重建、借用 swapchain 的
image view），而 crate 自己的集成测试——公开 API 的外部用户——调用了 34 次
`.raw()`。一条被代码无视的规则比没有规则更糟：没人分得清哪条边是被许可的。

## Decision

彻底封闭边界；让正当需求走一等 API。

1. **一等 API** 覆盖引擎层的三个操作：`Instance::supports_present(&Device, &Surface)`、
   `Swapchain::recreate`（内部自传旧句柄）、`Swapchain::image_view(index)` 返回借用的
   `TextureView`。`render-core` 的窗口层现在只使用这些。
2. **公共面清理**：所有 `raw()` 访问器收窄为 `pub(crate)`；真正无人使用的项删除
   （`CommandPool::raw`、未使用的 `compute_queue` 字段/访问器、`Instance::surface_instance`、
   `Swapchain::images`/`format` 访问器及 `images` 字段）。签名去 ash 化：`Device::new`
   改收 `Option<&Surface>`，`Surface::from_window` 去掉 `ash::Entry` 参数，
   `Swapchain::extent` 返回 crate 自己的 `Extent2d`，`CompiledShader` 的字段变为
   `pub(crate)`，`FrameUploader::upload_image` 和 `QueueFamilyIndices::find` 收窄为
   `pub(crate)`，`From<ash::vk::Result>`/`From<ash::LoadingError>` 实现被替换为
   `pub(crate) fn Error::from_vk` 加五处显式 `map_err`。
3. **测试搬入 crate 内部**：18 个集成测试文件（3400 行）从 `tests/` 迁入
   `src/gpu_tests/`，成为 `#[cfg(test)]` 模块——它们断言的本来就是内部细节
   （descriptor 槽位、屏障、readback 指针），在 crate 内可直接验证 `pub(crate)`。
   `tests/common/mod.rs` 变成 `src/gpu_tests/common.rs`；`headless_triangle` 示例是
   该测试的精简重复，已删除。无驱动机器上的优雅 skip 行为不变。
4. **机械强制**：`scripts/verify_rhi_boundary.py` 在 `moonfield-rhi/src` 的任何公开项
   提及 `ash::`/`vk::`/`gpu_allocator` 时让 CI（新增 `rhi-boundary` job）失败。规则写在
   `crates/moonfield-rhi/AGENTS.md`；冒烟测试命令改为
   `cargo test -p moonfield-rhi gpu_tests::headless_triangle`。

## Alternatives considered

- **保留逃生口并文档化为受许可的 interop。** 否决：文字规则已经失败过一次；
  "受许可的逃生口"策略仍然允许消费者在裸句柄上构建，每多一处使用都让边界更难
  恢复。
- **为测试提供 feature 门控或 `#[doc(hidden)]` 的内部 API。** 否决：仍是逃生口——
  公开就是公开，"隐藏"项一旦被下游 crate 发现就会成为事实依赖。
- **只收窄无人使用的部分，其余保持公开。** 否决：半封闭的边界只会吸引下一次泄漏；
  全量盘点表明剩余的逃生口本来就只有 crate 内部在用，封闭的代价其实很小。

## Consequences

- `moonfield-rhi` 的公开 API 不含任何 `ash`、`vk::`、`gpu_allocator` 类型；
  `scripts/verify_rhi_boundary.py` 在每次 push 时证明这一点。
- RHI 可独立复用：依赖锥只有 `moonfield-math` 和外部 crate，消费者无法对 Vulkan
  内部实现产生依赖。
- 新增能力现在必须走"在 rhi 加一等 API"的路——这是刻意的摩擦，换取边界的纯度。
- GPU 测试作为 crate 内单元测试运行（42 个，无兼容驱动时仍优雅跳过）；crate 不再有
  `tests/` 或 `examples/` 目录。
- `Stage` 的 `pub const` 初始化值中出现了 `vk::`——这不是泄漏（公开的类型标注是
  crate 自己的 `Stage`）；守卫脚本将 const/static 的初始化值视为实现细节。
