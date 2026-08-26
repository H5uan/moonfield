# Agent Note: GPU allocator destroyed before the logical device

Status: implemented

[English](2026-08-21-allocator-destroyed-before-device.md)

## Problem

关闭编辑器窗口在 macOS 上段错误。崩溃发生在 `Allocator::drop` 期间经 `vkFreeMemory`
进入的 `MVKDeviceMemory::~MVKDeviceMemory`。`Device` 以 `Arc<Mutex<Allocator>>` 持有共享的
gpu-allocator;`Device::drop` 先调用 `vkDestroyDevice`,随后 `allocator` 字段才被 drop——
于是它的内存块(`vkFreeMemory` / `vkUnmapMemory`)在逻辑设备已销毁后才被释放。MoltenVK
在这次调用中解引用已释放的 Objective-C 对象,从而崩溃。

## Decision

`Device::allocator` 现在是 `Option<Arc<Mutex<Allocator>>>`。`Device::drop` 取出 allocator,
当它仍持有最后一个 `Arc` 时(每个 `Buffer`/image 资源都先于其所属 device drop,并释放自己的
allocator clone),在设备句柄仍有效时先销毁它,再调用 `vkDestroyDevice`。`allocator()` 访问器做
unwrap;它仅在设备 drop 期间为 `None`。

## Alternatives considered

- **重排 `Device` 字段,让 allocator 先于 `device` drop。** 拒绝:`vkDestroyDevice` 在
  `Drop::drop` body 里执行,先于任何字段 drop,字段顺序帮不上忙。
- **让 buffers 先于 device 释放。** 拒绝:拥有内存块的是 allocator 而非单个 buffer,device 才是
  allocator 的阻塞持有者;在设备销毁时释放 allocator 是自然的单一释放点。

## Consequences

- 关闭编辑器干净退出(状态 0),不再段错误。
- allocator 的 `try_unwrap` 要求所有资源的 allocator `Arc` 都已先 drop;若有泄漏资源仍持有
  clone,`try_unwrap` 失败,allocator 随之泄漏(安全:不会对已销毁设备释放内存)。
