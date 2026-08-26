# Agent Note: Vulkan RHI boundary

Status: implemented

[English](2026-08-19-vulkan-rhi-boundary.md)

## Problem

渲染器必须让场景与渲染器 crate 表达自己的工作而无需依赖 `ash` 类型,同时所有驱动调用留在同一个 crate 内。两个约定让这条接缝承重:引擎的 clip 空间是 Y-up + reverse-Z(Vulkan 是 Y-down),调用方绝不能需要知道 viewport 翻转发生在哪里。没有显式边界,裸 `Vk*` 句柄与坐标翻转就会泄漏进场景代码,之后难以收拾。

## Decision

`moonfield-rhi` 是唯一链接 `ash` 的 crate,所有 Vulkan 专属代码位于 `src/vulkan/`(device、swapchain、pipeline、command、sync、offscreen、shader)。它暴露的表面使用自己的词汇:

- 公开资源描述——`Format`、`BufferUsage`、`VertexBufferLayout`——声明在 `src/types.rs`,绝不使用裸 `ash` 类型。
- 引擎 clip 约定是 **Y-up + reverse-Z**;任何 Vulkan viewport 调整发生在这条边界(`vulkan::*`),而不是场景或渲染器代码。
- 所有 Vulkan 对象住在主线程;目前没有任何东西跨线程 `Send`。对象按创建逆序、显式 drop 顺序销毁。
- Shader:后端在运行时编译 Slang→SPIR-V(`vulkan/shader.rs`),`ShaderModule::from_spirv` 直接加载字节码;一次离线 `slangc -target spirv` 编译也可通过 `include_bytes!` 产出内嵌字节。
- `cargo test -p moonfield-rhi --test headless_triangle` 在 lavapipe 上无头运行;Windows/macOS 无 Vulkan 驱动时优雅跳过。

## Alternatives considered

- **跨 crate 暴露裸 `ash` 类型。** 拒绝:每个消费方都会依赖 `ash` 和 Vulkan 生命周期规则;`types.rs` 让表面可测试、可替换。
- **给每个 Vulkan 对象包一层完整对象模型。** 拒绝:逐对象抽象层只增加层次,不增加安全性;只导出资源描述词汇,其余都留在 `vulkan/` 之后。
- **让 clip 空间改用 Vulkan 原生(Y-down)。** 拒绝:引擎数学层(reverse-Z、Y-up)与相机/渲染代码一致;在边界一处调整 viewport 比到处翻转约定便宜。
- **每个 shader 都离线编译。** 拒绝:运行时编译满足迭代,也让后端拥有工具链;离线内嵌保留给交付。

## Consequences

- 切换后端(或无需 GPU 测试)都在 `types.rs` 之后进行,场景代码零改动。
- 单线程所有权今天简单安全,但意味着 GPU 工作与 ECS 更新无法重叠;这一点推迟到命令队列交接落地之后。
- `shader-slang-sys` 构建与运行时都需要 Slang(`SLANG_DIR` 或 `VULKAN_SDK`);CI 的 `setup-slang` action 固定版本。