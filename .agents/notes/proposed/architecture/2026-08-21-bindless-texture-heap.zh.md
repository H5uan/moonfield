# Agent Note: Bindless texture heap (update-after-bind descriptor set)

Status: proposed

[English](2026-08-21-bindless-texture-heap.md)

## Problem

bindless 计算路径（GPU 指针模型）已交付：`GpuAllocation` 把 CPU 指针与设备
地址配对，计算内核通过单个 `GpuPtr` 接收根数据，barrier 只描述阶段。里程碑
范围里缺失、且任何图形管线之前都必需的是纹理模型：博客所说的“全局可索引
纹理描述符堆”——用户可见的纹理描述符数组，着色器用 32 位值索引，CPU（最终
还有 compute）可直接写入。

在本文档所基于的机器上评估了两条候选 Vulkan 路线：

- `VK_EXT_descriptor_heap`（2025）：博客模型最直接的对应物。已被否决：
  ash 0.38.0 没有它的绑定（只有 `descriptor_buffer`），本机 MoltenVK
  1.4.323（以及 lavapipe）不暴露该扩展——`vulkaninfo` 显示
  `VK_EXT_descriptor_heap` 支持数为 0。当驱动根本不枚举该扩展时，手写 FFI
  绑定也无济于事。
- update-after-bind 描述符集（`VK_EXT_descriptor_indexing`，Vulkan 1.2 核心
  特性）：即既有 note 已列为“Partially adopted”的保留模式路线。本方案采用
  它，因为本机已确认支持全部所需特性位（经 `vulkaninfo` 验证）：
  `descriptorIndexing`、`runtimeDescriptorArray`、
  `descriptorBindingSampledImageUpdateAfterBind`、
  `descriptorBindingVariableDescriptorCount`、`descriptorBindingPartiallyBound`、
  `shaderSampledImageArrayNonUniformIndexing`。

## Proposal

`moonfield-render/src/vulkan/bindless/` 下的 `texture_heap` 模块把博客的
纹理堆实现为一个大号 update-after-bind 描述符集：创建时定容，经
`vkUpdateDescriptorSets` 写入。公共接口复刻博客心智模型：槽位是 32 位索引，
堆存活于应用生命周期，着色器采样 `textureHeap[data.textureIndex]`。

### Textures 必须先存在

与 `gpu_alloc`（无需资源对象即可分配内存）不同，本里程碑的纹理是真实的
Vulkan image：既有 Vulkan 封面图像路径（`offscreen.rs`）已有 image /
image-view / sampler 创建原语。极简 `Texture` 值持有 `(vk::Image,
vk::ImageView, vk::Sampler)` 并在 drop 时释放。描述符堆部分只是在既有 image
之上做纯描述符记账，符合博客“纹理描述符创建需要一个薄 GPU 专用用户态 API”
的说法。

## Acceptance criteria

- [ ] `TextureHeap::new(device, capacity)` 创建一个 UAB 描述符池 + 一个
      描述符集，内含单个 `capacity` 个采样图像的 runtime-array 绑定；
      `capacity` 是 `u32`
- [ ] `TextureHeap::alloc_slot()` / `free_slot()` 从空闲表（bitmap 或
      `Vec`）发放从 0 开始的 32 位索引；`write(slot, texture)` 经
      `vkUpdateDescriptorSets` 写入 image-view+sampler 描述符
- [ ] 内核通过携带 `uint32 textureIndex` 的根结构体采样 `texture_heap[index]`；
      计算管线布局携带一个空 UAB 描述符集绑定 + 现有 push-constant 范围
- [ ] 无头集成测试（lavapipe CI + MoltenVK 本地）上传两块纯色纹理，经迷你
      compute 内核计算每槽平均值，读回并断言每槽值与纹理匹配
- [ ] clippy/fmt 干净；除 Vulkan 调用点外无新增 unsafe 面

## Risks

- MoltenVK 有每阶段 update-after-bind 上限（`vulkaninfo` 可见：
  `maxPerStageDescriptorUpdateAfterBindSampledImages` 为 1,000,000）；我们的
  capacity 必须落在机器实际限制之内。
- 管线布局把 UAB 绑定硬编码进 `ComputePipeline::new`（当前只有
  push-constant）。此后每个 compute 管线创建都必须传同一个堆布局，否则
  validation 期描述符查找失败。这支持“`ComputePipeline` 接收调用方的堆”
  而非“难以发现的全局变量”。
- 博客的“GPU 可写纹理堆”（compute 写描述符数据）**无法**用 UAB 描述符集
  实现——描述符仍只能经 `vkUpdateDescriptorSets` 由 CPU 写。记录为已知代价；
  一旦 ash 与驱动支持 `VK_EXT_descriptor_heap`，才能获得直接的 GPU 可写堆。
- 在 GPU 可能仍读取某槽位时更新它有竞态风险，需 `HAZARD_DESCRIPTORS`
  barrier；bindless barrier 模块已为该场景预留此 hazard 标志。

## Alternatives considered

- **`VK_EXT_descriptor_heap`**：理想方案，被否决（本机/CI 无驱动、无 ash
  绑定）。
- **`VK_EXT_descriptor_buffer`（ash 可用）**：描述符作为原始 GPU 内存
  块，最接近博客“描述符堆即内存”，并支持 GPU 写入。本里程碑否决：本机
  MoltenVK 的 descriptor buffer 支持未确认，且 UAB 路线的探测式验证已覆盖
  图形路径。
- **少量固定描述符集 + 快速绑定切换（retained-mode 模式）**：否决——
  每次绘制重新绑定描述符集正是博客要消除的代价；单个大 UAB 堆在其存活期内
  消除了它们。

## Consequences

- compute 与未来的图形管线可通过 CPU 映射的索引采样任意纹理；材质切换是
  对根结构体的一次 `uint32` 写入，而非描述符重绑——匹配博客的材质切换用例。
- RHI 在 `GpuPtr` 旁多了一个值类型：`TextureHandle`（`u32` newtype，
  `Copy`，可存进根结构体）。它不是 retained-mode 对象。
- `HAZARD_DESCRIPTORS` 在命令层变得可达：重写执行中内核读取的槽位后，
  `barrier(Stage, Stage, HAZARD_DESCRIPTORS)` 调用是合法的。
- 剩余图形管线里程碑可以用零新增绑定机制完成：像素经
  `texture_heap[data.textureIndex]` 采样纹理。