# Agent Note: Drop macOS support

Status: implemented

[English](2026-08-28-drop-macos-support.md)

## Problem

macOS 此前只能通过 MoltenVK 作为窗口化目标。MoltenVK 的 Vulkan 特性集落后于 RHI 的目标,迫使 instance、swapchain 和 buffer 代码中保留平台特定的适配,CI 矩阵也因此多出第三行。项目并不向 macOS 用户交付。

## Decision

受支持的目标平台为 Windows 和 Linux。具体如下:

- CI 只在 `ubuntu-latest` 和 `windows-latest` 上运行 clippy 和测试,setup-slang action 同时移除 macOS 的归档下载与库路径分支。
- RHI 的 `platform_surface_extensions` 不再有 macOS 分支;Windows 和 Linux 以外的操作系统不请求任何 surface 扩展,因此工作区在这些平台上仍可编译,但窗口化渲染会在创建 surface 时失败(无头使用不受影响)。
- `Instance::new` 不再设置 `ENUMERATE_PORTABILITY_KHR`;也不再有任何地方请求 `VK_KHR_portability_enumeration`。
- [持久映射的 host-visible 内存](../bug-fix/2026-08-21-host-visible-buffer-reuse-persistent-map.md)与[重建时指定旧 swapchain](../bug-fix/2026-08-21-swapchain-recreate-names-old-swapchain.md)两处行为保留:Vulkan 规范本身就要求它们(不得对已被映射的 `VkDeviceMemory` 再次调用 `vkMapMemory`;替代 swapchain 必须指定当前绑定在 surface 上的那个)。它们的注释现在引用规范条款,而不再提 MoltenVK。
- `moonfield-window` 中的 `NativeKeyCode::MacOS` 保留:该枚举是 winit 跨平台 `NativeKeyCode` 的完整镜像(同样包含 Android 和 XKB 变体),不属于平台支持代码。

## Alternatives considered

- **将 macOS 保留为只编译的次级目标。** 拒绝:MoltenVK 限制了 RHI 可依赖的 Vulkan 特性,而一个不做测试的目标仍然会为不存在的发售用户塑造代码形态。
- **连同 macOS 一起删除共享映射与旧 swapchain 适配。** 拒绝:两者都是 Vulkan 规范的要求,并非 MoltenVK 的怪癖;MoltenVK 只是最先强制执行它们的驱动。
- **在 macOS 上直接让编译失败(`compile_error!`)。** 拒绝:工作区仍需要能在 Mac 上编译以进行非渲染工作(ECS、资产、编辑器逻辑);硬性报错相比仅无头运行没有任何收益。

## Consequences

- MoltenVK 特定的代码路径与注释已移除;RHI 面向符合规范的 Windows 和 Linux Vulkan 驱动。
- 在 macOS 上,应用按既有文档中的无头容忍行为运行:不插入 `RenderDevice`,窗口化消费方持续重试,Vulkan 测试跳过。
- clippy 与 test 矩阵从三行减为两行。
- 早期笔记中对 MoltenVK 与 macOS 的提及作为各自决策的历史记录保留。
