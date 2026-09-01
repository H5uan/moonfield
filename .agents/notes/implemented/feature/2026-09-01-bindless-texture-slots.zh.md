# Agent Note: bindless texture slots

Status: implemented

[English](2026-09-01-bindless-texture-slots.md)

## Problem

`Texture::new` 创建的纹理无法参与 bindless 2.0 描述符堆：堆（commit `e3c3363`，`DescriptorHeap`）拥有槽位和 CPU 可见的描述符内存，但没有任何代码把纹理描述符写进去。bindless 着色器用 32 位 `TextureHandle` 索引纹理，因此纹理需要在创建时完成：分配槽位、上传像素、把 view 的描述符写进堆。

## Decision

`Texture` 增加可选 `slot: Option<TextureSlot>`：

- `TextureSlot` 持有 `{ handle: TextureHandle, heap: Arc<DescriptorHeap>, view_create_info: vk::ImageViewCreateInfo<'static> }`。create info 为*生命周期*而持有：堆的描述符写入编码的是它的指针（`ImageDescriptorInfoEXT.p_view`），因此它必须比槽位活得更久。
- `Texture::bindless(device, uploader, w, h, format, bytes)` 是主路径：创建 image + view、把上传排入帧上传器、分配槽位、写入描述符——一步完成，返回 `handle()` 即着色器侧索引的纹理。RGBA8 契约用字节长度校验守卫。
- `Texture::new` 原样保留（`slot: None`）作为 egui 互操作逃逸舱，仍经 `bind.rs` 的 set 绑定。
- `Drop` 先把槽位还给堆（bump 契约：已释放的槽位不再被引用），再按原有顺序销毁 view、image、allocation。
- 共享堆按需惰性构建：`Device::descriptor_heap()` 返回 `Arc<DescriptorHeap>`（OnceLock，与 `Device::uploader()` 相同模式），容量由新增的 `DESCRIPTOR_HEAP_IMAGE_CAPACITY` / `_SAMPLER_CAPACITY` 常量决定。

## Alternatives considered

- 槽位分配器留在 `Texture` 外部（调用方持有句柄）：drop 时会泄漏槽位，且破坏着色器契约想要"创建即绑定"的原子性。
- 存 view 句柄而非 create info：堆编码的是 create info 指针而非句柄，create info 必须与槽位共存亡。

## Consequences

- bindless 纹理自包含：创建完整就绪一个槽位，销毁完整归还它。
- 上传异步排入共享上传器；调用方仍在采样该句柄的帧之前提交（`end_frame`）。
- egui 的 `Texture::new` 路径零改动，由 `escape_hatch_has_no_slot` 验证。
- 下一步：把堆绑进管线（`cmd_bind_graphics`）——pipeline layout 集成属于渲染阶段工作。
