# Agent Note: Unified image layouts — GENERAL everywhere

Status: implemented

[English](2026-08-26-unified-image-layouts.md)

## Problem

RHI 为每一张图像都维护一个按用途区分的 image layout。`AttachmentLayout`
映射到 `PRESENT_SRC_KHR`、`SHADER_READ_ONLY_OPTIMAL` 和
`DEPTH_STENCIL_ATTACHMENT_OPTIMAL`;上传与回读要在
`TRANSFER_DST_OPTIMAL` / `TRANSFER_SRC_OPTIMAL` 之间转换;描述符写入处硬编码了
`SHADER_READ_ONLY_OPTIMAL`。每一次转换都增加一次实际 layout 与描述符集合或
render pass 附件里声明的 layout 偏离的机会——典型的 validation 级隐患——而引擎
被迫维护一个它本不想拥有的 layout 状态机。

## Decision

- 所有内部图像(`Texture`、`OffscreenTarget` 的 color 与 depth)在整个生命周期
  内都保持在 `VK_IMAGE_LAYOUT_GENERAL`。barrier 只承担 stage/access 同步;其
  `old_layout` / `new_layout` 均为 `GENERAL`。
- `AttachmentLayout::to_vk` 将 `ShaderRead` 与 `DepthStencil` 映射为 `GENERAL`;
  `Present` 保持 `PRESENT_SRC_KHR`——presents 是统一布局承诺明确不覆盖的场景之一。
- 保留图像创建时的 `UNDEFINED` 初始布局,首个 barrier 仍为 `UNDEFINED -> GENERAL`:
  初始化是另一个明确的例外,也是让新建图像内容变为已定义的操作。
- 描述符写入使用 `GENERAL`,声明的 layout 因此恒等于命令缓冲实际执行时的 layout。
- `VK_KHR_unified_image_layouts` 按机会启用。创建设备时通过
  `vkGetPhysicalDeviceFeatures2` 探测 `PhysicalDeviceUnifiedImageLayoutsFeaturesKHR`;
  支持时把扩展名与置位 `unifiedImageLayouts` 的 feature 加入设备创建;不支持时
  不做任何门控——`GENERAL` 在无扩展时同样合法,RHI 只存在一条代码路径。

## Alternatives considered

- **连 swapchain 一起全统一(`Present -> GENERAL`)**:否决——presentation 布局
  是扩展明确的例外;swapchain 每帧本来就要一次转换,保留 `PRESENT_SRC_KHR`
  没有额外成本。
- **保留按用途布局、只启用扩展**:否决——本次变更的目的正是删除 layout 状态机;
  扩展的效率承诺只在所有内部用途都走 `GENERAL` 之后才有意义。
- **像 `VK_EXT_descriptor_heap` 那样硬性要求扩展**:否决——CI 在 lavapipe 上
  通过 `headless_triangle` 真实创建设备;带可选启用的探测只有几行,无需
  probe-and-skip 测试辅助函数就能保持 CI 绿色。与 descriptor heap 不同,这里
  即使驱动缺少该扩展,布局代码依然正确。

## Consequences

- 除 `UNDEFINED -> GENERAL` 初始化与 swapchain present 路径外,RHI 不再做任何
  layout 转换。
- 描述符 / render pass / 命令层的 layout 错配在构造上就不可能发生:所有内部
  图像处处都是 `GENERAL`。
- 在暴露 `VK_KHR_unified_image_layouts` 的驱动上,`GENERAL` 在几乎所有用途下
  都是驱动承诺的高效布局;没有它代码依然正确,只是在部分硬件上可能非最优。
- 标准 validation 不再能抓住 layout 错配(已无错配可言);Synchronization
  Validation——RHI 通过 synchronization2 本就处于的验证范式——成为安全网。
- 本机已验证:`headless_triangle` 与 `egui_headless` 在编辑机驱动上通过;
  `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo fmt --check`
  无告警。