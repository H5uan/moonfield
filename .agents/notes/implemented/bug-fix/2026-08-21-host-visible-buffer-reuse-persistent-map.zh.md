# Agent Note: Host-visible buffers reuse gpu-allocator's persistent map

Status: implemented

[English](2026-08-21-host-visible-buffer-reuse-persistent-map.md)

## Problem

上传或读取 host-visible(`CpuToGpu` / `GpuToCpu`)buffer 在启动时失败,报
`VK_ERROR_MEMORY_MAP_FAILED: Memory is already mapped`。`Buffer` 的分配走设备共享的
gpu-allocator,后者把每块 host-visible 内存块保持持久映射,`mapped_ptr` 已指向分配区的确切位置
(offset 已内置)。旧的 `upload`/`read` 路径在同一个 `VkDeviceMemory` 上再次调用
`vkMapMemory`,MoltenVK 拒绝这种用法。CI 上的 lavapipe 抓不到,只有严格的 MoltenVK 会报。编辑器
的 `Viewport::new` 因此失败,viewport 从不渲染。

## Decision

`moonfield-render::vulkan::buffer::Buffer::upload_host_visible` 与 `read` 在
`mapped_ptr` 存在时复用该持久映射指针,仅当分配未持久映射时才退回手动 map/unmap。这落实了
host-visible buffer 一律通过 gpu-allocator 的持久映射写入的规则。

## Alternatives considered

- **先 `unmap_memory` 再 `map_memory`。** 拒绝:对 gpu-allocator 保持映射的块做 unmap 会破坏
  allocator 自身的映射契约。
- **弃用 `Buffer`,每次操作直接映射裸内存。** 拒绝:失去 gpu-allocator 提供的分配/offset 与
  生命周期管理。

## Consequences

- 启动不再失败;viewport 的立方体管线通过持久映射上传顶点与索引数据。
- 同一规则适用于 `read`(viewport dump、测试回读),它同样走 host-visible buffer。
