# Agent Note: Swapchain recreate names the old swapchain

Status: implemented

[中文](2026-08-21-swapchain-recreate-names-old-swapchain.zh.md)

## Problem

Resizing the editor window failed with
`VK_ERROR_NATIVE_WINDOW_IN_USE_KHR: vkCreateSwapchainKHR(): oldSwapchain does
not match the VkSwapchain that is in use by the surface`. `WindowRenderer::recreate`
created the replacement swapchain without passing the current swapchain, so its
`oldSwapchain` field was null. MoltenVK requires the new swapchain to name the
one currently bound to the surface; recreating against a null handle fails.

## Decision

`Swapchain::new` takes an `old_swapchain: Option<vk::SwapchainKHR>` and sets it
as `oldSwapchain` on `SwapchainCreateInfoKHR`. `WindowRenderer::recreate` passes
the current swapchain's raw handle; the first creation (`WindowRenderer::new`)
passes `None`. The old swapchain is dropped after the new one is created, once
the device is idle.

## Alternatives considered

- **Recreate on the `ERROR_OUT_OF_DATE_KHR` path only.** Rejected: resize fails
  the same way and needs the same fix; both paths funnel through `recreate`.
- **Keep a second live swapchain and defer destruction.** Rejected: passing the
  old handle as `oldSwapchain` and dropping it after the new creation is the
  standard Vulkan recycle pattern, and the device is already idle at that point.

## Consequences

- Window resize recreates the swapchain and its framebuffers without error.
- The `swapchain` and `framebuffers` fields are rebuilt in place; image views
  from the old swapchain are destroyed by `Swapchain::drop` after replacement.
