# Agent Note: VK_KHR_unified_image_layouts as an optional device extension

Status: implemented

[English](2026-09-19-unified-image-layouts-extension.md)

## Problem

RHI 让所有非 swapchain 图像常驻 `VK_IMAGE_LAYOUT_GENERAL`(见
[统一图像布局——处处 GENERAL](../architecture/2026-08-26-unified-image-layouts.md))。
在没有 `VK_KHR_unified_image_layouts` 的情况下,这一选择在所有用到的地方都合法,
但 `GENERAL` 的最优性只是经验之谈,并非规范承诺。该扩展(Vulkan 1.4.313,2025
年)正是为这一用法背书:启用 `unifiedImageLayouts` 后,`GENERAL` 在几乎所有用途下
合法且有规范保证的最优性能。RHI 希望在提供该扩展的驱动上获得这一承诺,同时不破坏
尚未提供它的驱动。

## Decision

- `ash::khr::unified_image_layouts::NAME` 加入
  `crates/moonfield-rhi/src/vulkan/device.rs` 的
  `OPTIONAL_DEVICE_EXTENSIONS`:枚举到支持则启用,否则告警并跳过——沿用既有的可选
  扩展模式(见[可选设备扩展](../architecture/2026-08-28-optional-device-extensions.md))。
- 启用前先探测 feature 位。仅有扩展名并不意味着有 `unifiedImageLayouts`,因此创建设备
  时把 `VkPhysicalDeviceUnifiedImageLayoutsFeaturesKHR` 链入
  `vkGetPhysicalDeviceFeatures2` 查询,feature 位为假则从候选中剔除——与
  `VK_EXT_shader_atomic_float` 的探测同形。候选保留时,把同一结构体置
  `unifiedImageLayouts(true)` 链入设备创建的 `pNext`,并按启用列表门控,使请求与启用
  列表严格一致。`unifiedImageLayoutsVideo` 保持关闭,RHI 不做视频编解码。
- 无需本地定义结构体或 sType:工作区把 ash 钉在 git master(`0.38.0+1.4.352`),其中
  已包含 `ash::khr::unified_image_layouts::NAME` 与
  `vk::PhysicalDeviceUnifiedImageLayoutsFeaturesKHR`。取值与 Khronos 注册表一致
  (扩展号 528,`VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_UNIFIED_IMAGE_LAYOUTS_FEATURES_KHR
  = 1000527000`;见 Vulkan-Headers 的 `vulkan_core.h`)。
- swapchain 的 layout 处理有意保持不变(项目负责人的决定):
  `AttachmentLayout::Present` 仍为 `PRESENT_SRC_KHR`。presentation 是该扩展明确豁免的
  场景;swapchain 每帧本就有一次转换;在支持的桌面目标(Windows/Linux)上驱动对
  `PRESENT_SRC_KHR` 的处理本身就是全性能的。
- 渲染代码零改动:图像本来就在 `GENERAL`。只有文档更新——
  `crates/moonfield-rhi/src/types.rs` 中 `AttachmentLayout` 的文档注释和 device.rs
  的扩展注释。
- 查询暴露无需新增:`Device::optional_extension_enabled(&CStr)` 已覆盖该扩展,且没有
  任何代码路径依赖该 feature 位,故未添加专用访问器。

## Alternatives considered

- **像 `VK_EXT_descriptor_heap` 那样硬性要求扩展**:否决——编辑机的 AMD 驱动
  (Vulkan 1.4.349)尚未暴露它,而没有扩展时 `GENERAL` 依然合法;可选启用保持了单一
  代码路径,也保持这些驱动可用。
- **swapchain 也迁到 `GENERAL`**:否决(负责人决定)——presentation 被扩展明确豁免,
  迁移没有收益。
- **为 ash 手写结构体与 sType**:不必要——这是为钉住的 ash 早于 Vulkan 1.4.313 头文件
  准备的退路,但 git 钉版已带有这些绑定。
- **在 `Device` 上加专用 `unified_image_layouts()` 访问器**:否决——与
  `buffer_float32_atomic_add` 不同,没有任何消费者,`optional_extension_enabled` 已足够。

## Consequences

- 在暴露该扩展及 feature 位的驱动上,`GENERAL` 成为几乎所有用途下规范保证最优的布局;
  其余驱动上行为与此前一字节不差(依然合法,只是可能非最优)。
- 对声明了该扩展名的驱动,设备创建多一次 `vkGetPhysicalDeviceFeatures2` 探测。
- 统一图像布局笔记曾先于代码声称按机会启用;本次变更使该说法成真。
- 本机已验证:编辑机驱动未暴露该扩展,设备创建按标准告警跳过,`moonfield-rhi` 全部
  47 个测试(含 `gpu_tests::headless_triangle`)通过——跳过路径已端到端验证;启用路径
  待提供该扩展的驱动出现。
