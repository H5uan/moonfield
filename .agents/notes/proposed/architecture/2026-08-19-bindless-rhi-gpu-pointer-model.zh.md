# Agent Note: Bindless RHI GPU pointer model

Status: proposed

[English](2026-08-19-bindless-rhi-gpu-pointer-model.md)

## Problem

引擎的 RHI 依赖保留模式的绑定对象——`BindGroup`、`BindGroupLayout`,以及通过描述符集声明着色器输入的管线。现代 GPU 暴露了 bindless 访问:着色器通过原始 64 位 GPU 地址(buffer device address)寻址数据,而非绑定的描述符集。围绕该模型设计的图形 API 可以整个去掉绑定抽象层:着色器根数据是每个着色器阶段的一个 GPU 指针,纹理是用户托管堆里的索引,barrier 描述 stage 到 stage 的依赖而不带资源列表。这一设计——由 Sebastian Aaltonen 在 [No Graphics API][no-gapi] 中提出——正是本项目想走的方向:它移除了当前横亘在场景代码与 GPU 之间的保留模式对象,也贴合高斯泼溅这类计算密集负载。

本 note 记录在 `moonfield-render` 内构建 bindless 计算路径的计划,并最终以它替换现有绑定模型。

[no-gapi]: https://www.sebastianaaltonen.com/blog/no-graphics-api

## Proposal

在 `moonfield-render/src/vulkan/` 下新增 `bindless` 模块,并以 `moonfield_render::bindless` 暴露。它是并行的 compute-first 路径;现有基于 `BindGroup`/`RenderPass` 的模块保持冻结,直到 bindless 路径覆盖图形管线,然后一次性删除保留模式。

### Memory: `gpu_alloc`

`gpu_alloc(device, bytes, align, memory)` 返回 `(cpu_ptr, gpu_ptr)` 对:CPU 指针可直接写入(UMA 或 ReBAR 映射堆),GPU 指针是可在着色器中使用的 buffer device address。`Memory` 有 `Default`(CPU 映射,常见情形)、`Gpu`(device-local,用于纹理与大 buffer)和 `ReadBack`。底层分配器仍是 `gpu-allocator`;内存在其上池化,由 bindless 层子分配。

GPU 指针是值类型 `GpuPtr(u64)`,不是句柄:它可以存入任意结构体、传给着色器、在 CPU 侧做算术调整——与 Loon GPU 的设计相同。Rust 通过所有权(`&`/struct)保留对象模型的安全性;不引入 `Handle<T>` 层。

### Root data

compute/vertex/fragment 着色器的根数据是每个阶段一个 `GpuPtr`。本地实验确认 Slang 工具链对 `Ptr<T, Access.Read>` 参数产出 `PhysicalStorageBuffer64` SPIR-V,并通过入口点 stage 的 push constant 传递根指针。bindless 命令层将指针作为 push constant 数据下发。

### Queue and synchronization

`queue`——`QueueType::{Graphics, Compute}`——是模块中的一等值,但初始里程碑将两者映射到同一物理队列;抽象保留,以便日后引入独立的 async-compute 队列而不破坏调用方。帧节奏使用 timeline semaphore,两帧在飞。

`barrier(before, after)` 映射为只有 stage 掩码的 Vulkan `MemoryBarrier2`——无资源列表。Hazard 标志(`HAZARD_DRAW_ARGUMENTS`,用于 GPU 侧生成绘制参数)留给后续里程碑。

博客中的 `gpuSignalAfter`/`gpuWaitBefore` 内存计数器在 API 中预留但本里程碑不实现;这些语义当前由 timeline semaphore 提供。

### 本里程碑范围(仅计算)

- `gpu_alloc` 返回 CPU/GPU 指针对。
- `compute pipeline` 与 `dispatch`,携带 `GpuPtr` 根指针。
- `dispatch_indirect` 从 GPU 内存读取启动参数。
- `cmd_memcpy` 用于 GPU→GPU 拷贝与回读。
- Pipeline desc 是可哈希 struct(shader 字节 + 特化常量),以便日后在不改公共 API 的前提下加入基于 `vkCreatePipelineCache` 的缓存。

明确不在范围内:图形绘制、render pass、纹理堆、特化常量缓存、以及每次绘制的 GPU 生成根数据(间接多绘制)。

## Alternatives considered

- **新 crate 从零编写。** 拒绝:保留模式模块留在 `moonfield-render`,编辑面已限制在 `src/vulkan/`;新 crate 会把相关 Vulkan 工作拆散到多个 crate。
- **引用 Loon GPU(C++ 实现的同一方案)。** 只采纳其设计:`GpuPtr` 作为值、着色器中无描述符集绑定、update-after-bind 纹理堆、timeline 帧节奏。不整体移植,因为 Metal 被拒绝,且引擎的 Rust 所有权模型改变了实现方式。
- **CPU 侧 `Handle<T>` 对象句柄。** 拒绝:Rust 所有权模型已在编译期证明生命周期;句柄表会把同样的错误推向运行时,还增加查找与锁。在 GPU 可寻址范围内的 `GpuPtr` 与纹理索引是值,不是句柄。
- **一个大的 update-after-bind 描述符集作为绑定。** 部分采纳:最终纹理模型使用 update-after-bind 描述符集。拒绝作为根数据路径:结构体内指针才是本设计的核心。

## Acceptance criteria

- `gpu_alloc` 返回可写的 CPU 指针和可用的 `GpuPtr`;CPU 写入对 GPU 可见。
- 带 `Ptr<T, Access>` 根参数的 Slang 计算着色器读取被下发的 GPU 地址处的数据,并写入结果 buffer。
- `dispatch` 启动 kernel;结果回读到 CPU 并校验预期值(CPU→GPU→CPU 闭环)。
- `barrier(Stage::Compute, Stage::Compute)` 在队列上运行,无资源列表。
- 两个在飞帧在一条 timeline semaphore 上排程。
- `tests/bindless_compute.rs` 在 MoltenVK(macOS 有驱动)与 lavapipe(CI)上双双通过,复用现有 Vulkan 存在性跳过模式。

## Risks

- Slang 的 `PhysicalStorageBuffer` 输出与 push constant 根指针是所固定工具链的绑定行为;升级 Slang 可能改变布局,需要一个编译期检查。
- MoltenVK 与 lavapipe 在部分描述符特性上限上不一致;计算路径只使用两者共享的特性集(如上述)。
- 过渡期内保留模式路径仍留在树中,因此 `cargo clippy` 不能因为冻结模块的调用方逐渐移走而对其告警。