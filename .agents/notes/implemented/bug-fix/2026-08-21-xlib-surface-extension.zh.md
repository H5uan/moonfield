# Agent Note: X11 window surfaces require VK_KHR_xlib_surface

Status: implemented

[English](2026-08-21-xlib-surface-extension.md)

## Problem

在 X11 会话下启动编辑器会崩溃,报 `Unable to load create_xlib_surface_khr`,panic
位于 ash 生成的 `extensions_generated.rs` —— 这是 ash 在加载 `vkCreateXlibSurfaceKHR`
失败时安装的桩函数。`ash_window` 会把 Xlib 窗口句柄路由到 `create_xlib_surface`,
但 Vulkan 实例创建时没有启用 `VK_KHR_xlib_surface`,驱动因此不暴露该函数,桩函数
在首次调用时直接 panic。

## Decision

`platform_surface_extensions()` 的 Linux 分支在 `VK_KHR_xcb_surface` 和
`VK_KHR_wayland_surface` 之外启用 `VK_KHR_xlib_surface`。由此覆盖 winit 0.30 在
Linux 上可能产生的全部句柄:X11 会话下的 Xlib 与 XCB 窗口句柄、Wayland 会话下的
wayland surface。扩展列表是静态的,因为共享实例先于任何窗口创建。

## Alternatives considered

- **在创建 surface 时按窗口的实际句柄类型挑选扩展。** 拒绝:实例先于窗口创建,
  且 `VkInstance` 无法按窗口重建。
- **只启用 XCB 路径。** 拒绝:winit 的 X11 后端分发的是 `XlibWindowHandle`/
  `XlibDisplayHandle`;XCB surface 路径根本走不到,因此该会话下 Xlib 扩展必不可少。
- **在 X11 下回退到 headless 实例。** 拒绝:这会静默禁用窗口化渲染,而窗口化渲染是
  编辑器的主要输出方式。

## Consequences

- X11 下窗口化渲染可用;Wayland 会话不受影响 —— 额外启用的扩展在那里是空操作。
- Linux 实例声明四个 surface 扩展。按 Vulkan 规范它们都是可选的,即使某个不受支持,
  `vkCreateInstance` 仍能成功。