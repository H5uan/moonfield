# Agent Note: Swapchain recreate names the old swapchain

Status: implemented

[English](2026-08-21-swapchain-recreate-names-old-swapchain.md)

## Problem

调整编辑器窗口大小时失败,报
`VK_ERROR_NATIVE_WINDOW_IN_USE_KHR: vkCreateSwapchainKHR(): oldSwapchain does
not match the VkSwapchain that is in use by the surface`。`WindowRenderer::recreate`
创建替换 swapchain 时没有传入当前 swapchain,导致 `oldSwapchain` 字段为空。MoltenVK
要求新 swapchain 指名当前绑定到 surface 的那个;对着空句柄重建就会失败。

## Decision

`Swapchain::new` 增加 `old_swapchain: Option<vk::SwapchainKHR>` 参数,并将其设为
`SwapchainCreateInfoKHR.oldSwapchain`。`WindowRenderer::recreate` 传入当前
swapchain 的裸句柄;首次创建(`WindowRenderer::new`)传 `None`。旧 swapchain 在新
swapchain 创建完成、设备空闲后被丢弃。

## Alternatives considered

- **只在 `ERROR_OUT_OF_DATE_KHR` 路径重建。** 拒绝:resize 以同样方式失败,也需要同样的修复;
  两条路径都会汇入 `recreate`。
- **保留第二个存活的 swapchain,延迟销毁。** 拒绝:把旧句柄作为 `oldSwapchain` 传入并在新
  swapchain 创建后丢弃,是标准的 Vulkan 回收写法,且此时设备已空闲。

## Consequences

- 窗口 resize 可无错重建 swapchain 及其 framebuffer。
- `swapchain` 与 `framebuffers` 字段就地重建;旧 swapchain 的 image view 在替换后由
  `Swapchain::drop` 销毁。
