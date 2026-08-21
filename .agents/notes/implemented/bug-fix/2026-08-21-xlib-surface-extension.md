# Agent Note: X11 window surfaces require VK_KHR_xlib_surface

Status: implemented

[中文](2026-08-21-xlib-surface-extension.zh.md)

## Problem

Launching the editor on an X11 session panicked with
`Unable to load create_xlib_surface_khr` inside ash's generated
`extensions_generated.rs` — the stub ash installs when `vkCreateXlibSurfaceKHR`
fails to load. `ash_window` routes an Xlib window handle to
`create_xlib_surface`, but the Vulkan instance was created without
`VK_KHR_xlib_surface` enabled, so the driver never exposes the function
and the stub panics on first call.

## Decision

The Linux branch of `platform_surface_extensions()` enables
`VK_KHR_xlib_surface` alongside `VK_KHR_xcb_surface` and
`VK_KHR_wayland_surface`. That covers every handle winit 0.30 can produce
on Linux: Xlib and XCB window handles on an X11 session, wayland surfaces
on a Wayland session. The extension list is static because the shared
instance is created before any window exists.

## Alternatives considered

- **Pick extensions by the window's actual handle type at surface
  creation.** Rejected: the instance is created before the window, and a
  `VkInstance` cannot be created per window.
- **Enable only the XCB path.** Rejected: winit's X11 backend hands out
  `XlibWindowHandle`/`XlibDisplayHandle`; the XCB surface path is never
  reached, so the Xlib extension is mandatory on that session.
- **Fall back to a headless instance on X11.** Rejected: it would silently
  disable windowed rendering, the editor's primary output.

## Consequences

- Windowed rendering works on X11; Wayland sessions are unaffected — the
  additional extension is a no-op there.
- The Linux instance declares four surface extensions. All are optional
  per the Vulkan spec, so `vkCreateInstance` succeeds even where one is
  unsupported.