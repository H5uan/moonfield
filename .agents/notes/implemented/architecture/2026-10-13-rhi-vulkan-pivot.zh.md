# Agent Note: RHI Vulkan pivot

Status: implemented

[English](2026-10-13-rhi-vulkan-pivot.md)

## Problem

私有模块布局在两条 cfg 门控的 re-export 列表之后维护两个按平台选择的后端(非 Apple
目标 `vulkan`,Apple 目标 `metal`)。每次 RHI 改动都要为第二条臂付税:新 API 要么补
Metal 实现、要么加能力门控,两条 re-export 列表彼此漂移,而 CI(ubuntu/windows)
从不编译 Metal 臂——它的契约只能在一台 Mac 上验证,而 editor 在那里编不过(bindless
通道是 Metal 提案推迟的阶段性工作)。

## Decision

Vulkan 是 RHI 的 pivot API——唯一的后端,也是公共词汇表所围绕的语义:

- `backend/metal/` 删除;`backend/vulkan/` 是唯一的私有后端,没有平台选择。facade
  只承载一条 re-export 列表,`shader_target()` 返回 `Spirv`。
- 原生栈(ash/ash-window/gpu-allocator)在 `Cargo.toml` 里无条件声明;objc2 栈随
  Metal 臂一起消失。
- 让第二后端保持便宜的接口纪律不变:词汇表模块持有语义,facade 的 re-export 列表
  是全部公共表面,`verify_rhi_boundary.py` 机械禁止后端类型泄漏其中,契约测试套件
  (`tests/contract_*.rs`)钉住任何后来后端必须遵守的公共行为。
- 后来的第二后端按 Godot 的 RenderingDevice 后端趋同其 Vulkan 形接口的方式趋同本
  pivot。最可能的一个是 D3D12:它直接映射到 descriptor-heap 的 bindless 词汇
  (`VK_EXT_descriptor_heap` 的设计就是 D3D12 形状的),stage 到 resource-state 的
  翻译留在后端内部;屏障保持在 `Stage` 词汇上,render pass 描述保持轻量 desc,而非
  Vulkan render pass 对象。
- Slang 的 `ShaderTarget` 词汇保留 Metal 目标供离线编译;后端的运行时目标是
  `Spirv`。

## Alternatives considered

**保留 Metal 臂休眠。** 落选:平台 cfg 让 CI 看不见这条臂,它只会静默腐烂——没有
构建它、没有验证它,契约在底下漂移。冻结的后端回来时仍是重写,与删除同结局,中间
还多付一份税。

**两条臂都保留并完成 Metal bindless 通道。** 落选:推迟的阶段性工作不服务任何消费
者;editor 是唯一的二进制,支持目标是 Windows 和 Linux。

## Consequences

- macOS 移出支持目标,直到那里再次落地一个后端。Git 历史与 Metal notes 保留实现知
  识;Vulkan 加载器(MoltenVK)原则上可以到达 Apple 硬件。
- 每次 RHI 改动只编译一个后端;新 API 的能力门控税与双 re-export 列表消失。
- sub-crate 时代的转发层 `backend/vulkan/shader/` 随之删除;后端代码直接从
  `crate::shader` 导入共享 shader 词汇。
