# Agent Note: Shared device context for GPU object lifetimes

Status: implemented

[English](2026-09-13-shared-device-context-lifecycle.md)

## Problem

此前只有 `GpuAllocation` 保住了设备的一部分(它的 `Arc<Mutex<Allocator>>`);
`Semaphore`/`Fence`、`Swapchain`、`CommandPool`、两种 pipeline 和
`ShaderModule` 只持裸 `ash::Device` 克隆。它们中任何一个活得比 `Device` 久,
都会对着已销毁的设备调 `destroy_*` —— 这是 Vulkan UB,而 `Device::drop` 里
基于 allocator 的泄漏守卫根本看不见。`Surface` 更是不持任何 instance 引用,
靠 render-core 的 `WindowSurfaceData` 字段顺序约定保命。在此之上,
`ash::Device + Arc<Mutex<Allocator>> + Arc<RetirementRing>` 这组字段三元组在
六个结构体里重复出现,而"create image → query requirements → allocate →
bind → create view"序列存在三份(texture、offscreen color、offscreen
depth)。

## Decision

wgpu 式共享所有权,设备拆成两层:

- `DeviceShared`(crate 内部)持有拆卸关键状态:`ash::Device` 句柄、
  `DeviceExtensionFunctions` loader、`RetirementRing`、
  `Arc<Mutex<Allocator>>`,以及一个 `Arc<InstanceShared>` 保活。它的
  `Drop` —— 由最后一个引用消失触发 —— 让 GPU idle、排空 ring、释放
  allocator,然后销毁设备。instance 的 Arc 在该函数体之后才析构,因此
  instance 按构造必然比设备长寿。
- `Device` 保留元数据与懒建单例(队列、队列族、heap 属性、
  uploader/descriptor-heap/shader/pipeline cache)。它的 `Drop` 只持久化
  pipeline cache 并提前释放单例 Arc。
- `DeviceContext`(crate 内部,`Clone`,`Deref` 到 `DeviceShared`)是每个
  GPU 对象保存、每个 crate 内部构造函数接收的唯一句柄,取代了逐字段的
  三元组。`Semaphore`、`Fence`、`TimestampQueryPool`、
  `CommandPool`/`CommandBuffer`、两种 pipeline、`ShaderModule`、
  `Swapchain`、`TextureView`、`FrameUploader`、`GpuBumpAllocator`、
  `GpuAllocation`、`Texture`、`OffscreenTarget`、`DepthBuffer` 和
  `HeapSlots` 都各持一个。
- `Instance` 包装 `Arc<InstanceShared>`;`Surface` 持有一个
  `Arc<InstanceShared>` 保活,字段顺序约定就此终结。
- `Image2d`(crate 内部,`vulkan/image.rs`)把三份 image 创建样板收敛为
  一次调用,返回 image + view + 自持有的 view create info + allocation。
- retirement action 仍然携带裸 `ash::Device` 克隆与 allocator Arc —— 刻意
  不用 `DeviceContext`:`ImageSlot` 此前携带 `Arc<DescriptorHeap>`,而 heap
  的后备 allocation 现在持有 `DeviceContext`,这会经由拥有该 action 的
  ring 闭合成强引用环(shared → ring → action → heap → allocation →
  shared),让设备永久泄漏。action 改为直接持有 heap 的
  `Arc<Mutex<SlotAllocator>>`;归还 slot 是纯 CPU 簿记,即使 heap 已销毁
  也依然合法。
- 泄漏守卫已删除:`Instance` 的 live-device 计数器和 `Device::drop` 的
  allocator `try_unwrap` 守卫守护的是共享所有权模型下无法表示的顺序。
  `DeviceShared::drop` 为 allocator 保留了一个防御性 `try_unwrap` 分支
  (宁可泄漏也不 use-after-destroy),但按构造不可达。

## Alternatives considered

- **万物共用一个 `Arc<DeviceInner>`(不拆 shared/外层)。** 懒建单例
  (uploader、descriptor heap)挂在设备上,会反持 inner —— 自环在
  uploader 初始化那一刻就让设备永久泄漏。两层拆分把拆卸关键状态之外的
  东西留在 Arc 外,`DeviceShared` 持有的任何东西都不会指回它。
- **在 Arc 模型之上保留泄漏守卫。** 对不可能状态做死检查会误导读者,让人
  以为顺序仍然承重;字段上的保活注释更好地承载了这个不变量。
- **单例用 Weak 反指设备。** 单例可以放进共享状态而不成环,但每次使用都
  变成可能失败的 upgrade;活得比设备久的 `FrameUploader` 将无法销毁自己的
  command buffer —— 把本次要消灭的 UB 类别又请了回来。
- **不做 `Image2d` 帮手。** 三份拷贝只在 usage 标志、aspect 和 allocation
  名字上有差异,收敛保持干净,因此采纳。

## Consequences

- "对象比设备长寿"不再是一类错误:逻辑设备在最后一个使用它的对象消失
  时才销毁,instance 比所有设备与 surface 都长寿。析构顺序约定
  (`RenderDevice` 字段顺序、`WindowSurfaceData` 字段顺序)退化为文档,
  不再承重。
- 公共 API 不变 —— 每个构造函数仍然接收 `&Device`/`&Instance`;下游 crate
  无需任何修改。边界门禁通过:`DeviceShared`/`DeviceContext`/
  `InstanceShared`/`Image2d` 全部是 crate 内部类型。
- 字段三元组恰好保留在一处,这是刻意的:`RetireAction::{Buffer, Image}`
  携带裸句柄加 allocator Arc,使 ring 内容永不强引用共享状态。
- 设备析构工作搬了家:pipeline cache 落盘与单例释放在 `Device::drop`;
  idle/drain/allocator/destroy 在 `DeviceShared::drop`,可能更晚(发生在
  最后一个引用析构所在的线程 —— 目前所有 Vulkan 对象都在主线程,实际
  是同一线程)。
- `gpu_tests` 全部原样通过,包括真实硬件上的 `headless_triangle`。
