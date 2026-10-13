# Agent Note: Physical device queries migrated to the v2 API family

Status: implemented

[English](2026-08-24-physical-device-query-v2.md)

## Problem

RHI 仍通过 Vulkan 1.0 时代的查询入口与物理设备对话,而实例本身按
Vulkan 1.4 创建(见[ash git master 笔记](2026-08-21-vulkan-1-4-via-ash-git-master.zh.md)):
`vkGetPhysicalDeviceProperties2` 与 `vkGetPhysicalDeviceQueueFamilyProperties2`
自 1.1 起就是核心函数,表面能力与格式的 KHR `2` 版查询也已存在。1.0
版本只返回基础结构体——没有 pNext 链——因此任何 Vulkan 1.2/1.3/1.4
扩展属性(如 `PhysicalDeviceVulkan13Properties`)或扩展表面能力都无法通过
它们获取;同一查询保留两种写法也会诱使新代码继续复制旧形式。

## Decision

- 删除 `Instance::physical_device_properties`,调用方改用
  `physical_device_properties2(&self, device, out: &mut PhysicalDeviceProperties2)`,
  传入输出结构体,扩展结构可通过其 pNext 指针挂链。选卡逻辑
  (`Device::new`、`RenderPlugin`、`tests/common/mod.rs`)读取
  `out.properties.device_*`。
- 删除 `Instance::queue_family_properties`;`queue_family_properties2`
  返回 `Vec<QueueFamilyProperties2>`(ash master 绑定是 len 查询加切片填充),
  消费方读取 `.queue_family_properties` 取标志位。
- `Surface::capabilities` / `formats` 改用 `vkGetPhysicalDeviceSurfaceCapabilities2KHR` /
  `vkGetPhysicalDeviceSurfaceFormats2KHR`,返回
  `SurfaceCapabilities2KHR` / `Vec<SurfaceFormat2KHR>`(基础数据位于
  `.surface_capabilities` / `.surface_format`)。共享实例在三个平台的
  表面扩展列表中都启用了 `VK_KHR_get_surface_capabilities2`,`Surface`
  在表面 loader 旁加载 `get_surface_capabilities2` loader。
- 没有 `2` 版本的查询保持不变:`vkGetPhysicalDeviceSurfacePresentModesKHR`
  与 `vkGetPhysicalDeviceSurfaceSupportKHR`。

## Alternatives considered

- **保留 1.0 包装,2 系列并行新增**:否决——同一查询存在两种写法,"新
  API"规则就无法执行,调用方会继续拷贝旧形式。
- **只迁移核心查询,表面查询不动**:否决——表面 `2` 版查询恰恰是能从
  pNext 获益的一方(如 `SurfaceProtectedCapabilitiesKHR`),迁移成本只是
  一个实例扩展加一个 loader。
- **保留 v1 的便利返回风格(`fn() -> T`),只换底层调用**:不可能——ash
  master 里 2 系列返回 `()`,填充调用方提供的结构体。

## Consequences

- RHI 中所有物理设备能力/格式查询都走 `2` 系列入口,以后挂扩展结构
  无需再更换查询函数。
- 调用点多一层嵌套(`props.device_type` → `props.properties.device_type`,
  `f.format` → `f.surface_format.format`);swapchain 选格式处一次性绑定
  `caps = capabilities.surface_capabilities`。
- 包含表面扩展的实例现在额外启用 `VK_KHR_get_surface_capabilities2`;
  headless 实例不变。
- 无实际行为变化——选 GPU 与设备名日志读取的字段与之前完全相同,
  只是经由 2 系列的查询形态。