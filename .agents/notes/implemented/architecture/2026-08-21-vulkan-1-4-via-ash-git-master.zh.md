# Agent Note: Vulkan 1.4 via ash git master

Status: implemented

[English](2026-08-21-vulkan-1-4-via-ash-git-master.md)

## Problem

crates.io 上已发布的 ash 最高版本为 `0.38.0+1.3.281`(基于 Vulkan-Headers
1.3.281 生成),早于 Vulkan 1.4 规范(2025-08)。绑定中缺少 `API_VERSION_1_4`
和 1.4 的结构定义(`VkPhysicalDeviceVulkan14Features`/`Properties`),RHI
无法请求或查询 1.4 实例。

## Decision

- 根 `Cargo.toml` 通过 `[patch.crates-io]` 把三个 vulkan crate 从 git 源打补丁:
  `ash` 与 `ash-window`(两者同仓)锁定 ash 仓库 commit `f4c2ca3`
  (`0.38.0+1.4.352`,Vulkan-Headers 1.4.352);`gpu-allocator` 锁定
  Traverse-Research/gpu-allocator 的 `ash-next` 分支 commit `6a68a5b`。
  各 crate 的清单仍保留发布版的 semver 约束(`ash = "0.38"` 等);补丁版本
  沿用 `0.38.x` 前缀,因此图中所有 registry 需求——包括传递依赖——都解析到
  补丁源。
- `moonfield-render` 的调用点迁移到 ash master API,上游有三处机械性破坏:
  扩展加载器 `Instance::new`/`Device::new` 改为 `load`;pNext builder 方法
  `push_next` 改为 `TaggedStructure::push` trait 方法,导入方式为
  `use ash::vk::{self, TaggedStructure as _}`;`ash_window::create_surface`
  替换为 `SurfaceFactory::new(...).create_surface(...)`。
- 实例请求 1.4 API 级别:
  `ApplicationInfo::api_version(vk::API_VERSION_1_4)`。

## Alternatives considered

- **留在已发布的 ash,本地补齐 1.4 定义。** 拒绝:手写 1.4 结构的 FFI 定义会
  重复生成代码,实例版本也只能以裸数字常量形式出现,上游无法识别。
- **等待 ash 下一个正式版本。** 拒绝:ash 大约每年发布一次,而 Vulkan 1.4
  的支持现在就需要。
- **vendor 三个 crate 的 fork。** 拒绝:把三份上游代码拷进仓库会重复承担
  上游的维护负担,并掩盖上游的演进脉络。

## Consequences

- RHI 以 Vulkan 1.4 创建实例,1.4 核心类型可用于设备特性/属性查询。
- ash master 不是发布版:在锁定的 commit 上源码与 `0.38` semver 前缀保持
  兼容,但未来 master 的变更可能在下一次发布前再次破坏 API;这类漂移在
  进入时在此消化。
- `rev` 锁定保证构建可复现;`Cargo.lock` 记录 git 源。
- 不支持 Vulkan 1.4 的驱动会导致 `create_instance` 失败;驱动版本探测
  (`entry::try_enumerate_instance_version`)尚未接入。