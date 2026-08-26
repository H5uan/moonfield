# Agent Note: VK_EXT_descriptor_heap enabled unconditionally

Status: implemented

[English](2026-08-21-enable-vk-ext-descriptor-heap.md)

## Problem

bindless 纹理堆提案在评估时曾因该扩展没有 ash 绑定、驱动不支持而否决了
`VK_EXT_descriptor_heap`。这两个约束都已消失:ash git 固定版本已包含生成的
`ext::descriptor_heap` 绑定(Vulkan-Headers 1.4.352,见[ash git master
笔记](2026-08-21-vulkan-1-4-via-ash-git-master.zh.md)),且目标平台现在是
枚举到该扩展的较新桌面驱动——当时记录为仅限 NVIDIA,但 AMD 的 Windows
专有驱动同样暴露该扩展(已在 RX 9070 XT、驱动 32.0.23034.4 上验证)。
启用该扩展是直接可写的 descriptor heap 模型的先决
条件,该模型用单一可索引堆取代每次 draw 的描述符集重绑定。

## Decision

- `moonfield-render` 在 `Device::from_physical_device` 中为每个逻辑设备
  请求 `VK_EXT_descriptor_heap`:扩展名写入 `DEVICE_EXTENSIONS`,与
  `VK_KHR_swapchain` 并列;`VkPhysicalDeviceDescriptorHeapFeaturesEXT`
  置位 `descriptorHeap` 后接入 `VkPhysicalDeviceFeatures2` 的 pNext 链。
- 无降级路径:未枚举该扩展的驱动在 `vkCreateDevice` 阶段失败。RHI 仅面向
  暴露该扩展的驱动(当前的 NVIDIA 与 AMD 专有驱动均可),不提供脱离堆仍能
  运行的代码路径。
- GPU 集成测试探测与 `Device::new` 相同选择的物理设备;扩展缺失时测试
  打印明确原因后跳过(`tests/common/mod.rs`),而不是把设备创建错误暴露成
  虚假的失败。CI 的 lavapipe 环境保持绿色。

## Alternatives considered

- **仅在枚举到时启用扩展、保留旧路径**:否决——RHI 只面向具备 descriptor
  heap 能力的平台;静默的双代码路径正是目标平台规则要排除的分叉。
- **GPU 测试加 `#[ignore]` 并移到自托管 runner**:暂缓——探测并跳过的
  辅助函数无需搬移测试即可保持 CI 绿色;之后接入自托管 runner 无需改代码。
- **等 Mesa 实现该扩展**:否决——lavapipe 没有公布支持计划;测试在那里跳过。

## Consequences

- 设备创建的成功与否取决于驱动是否支持该扩展;编辑器和 GPU 测试只在具备
  descriptor heap 能力的驱动上运行。
- `cargo test` 在 CI 上保持绿色:不支持的驱动打印跳过原因,支持的驱动跑
  完整套件(本机 AMD 驱动会真正跑全部 GPU 测试)。
- 纹理堆里程碑不再受阻:descriptor heap 可通过 `VK_BUFFER_USAGE_DESCRIPTOR_HEAP_EXT`
  缓冲区、`vkWriteResourceDescriptorsEXT` / `vkWriteSamplerDescriptorsEXT`
  与 `vkCmdBindResourceHeapEXT` 承载纹理模型;提案风险清单里记录的
  直接写入成本不再适用。
- 描述符写入从 `vkUpdateDescriptorSets` 变为直接写入堆;`HAZARD_DESCRIPTORS`
  的次序约束仍由 bindless 屏障模块负责。