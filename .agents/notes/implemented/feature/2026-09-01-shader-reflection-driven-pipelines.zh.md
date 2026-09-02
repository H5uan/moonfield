# Agent Note: shader-reflection-driven pipelines

Status: implemented

[English](2026-09-01-shader-reflection-driven-pipelines.md)

## Problem

bindless 描述符堆管线在每个层面都硬编码了 shader 管线形态。`GraphicsPipeline::new_with_options` 固定构造恰好两个 stage（VERTEX + FRAGMENT）、入口名写死 `"main"`；`ComputePipeline::new` 固定一个 COMPUTE stage 加 `"main"`；顶点布局（`VertexBufferLayout`）在每个调用点手写；每 draw 的根数据用 `bytemuck::bytes_of` 对着"Layout must match X in shader"注释同步的结构手工组装。这一切在运行/编译期都没有任何校验——shader 的 `[shader("...")]` 或结构体布局一旦变化，宿主侧会静默保持过期状态。

而 Slang 反射恰好提供了闭环所需的一切：每个入口的 stage、其根参数及字节位置、以及顶点输入布局。

## Decision

让 shader 从"指向管线的某个东西"变成唯一真理来源，其余全部从 Slang 反射读取：

- `CompiledShader` 现在携带发射出的 SPIR-V 入口名（从 `OpEntryPoint` 解析——发射名与源名不同），管线不再写死 `"main"`。`ShaderModule::from_compiled` 记录 stage + entry；管线构造校验模块的 stage 与 stage 列表匹配，并拒绝无 stage 信息的模块。
- `GraphicsPipeline::new_with_stages(device, formats, depth, &[ShaderStageDesc], layout)` 从任意 stage 列表构建管线（`ShaderStageDesc` 只是一个模块）；双 stage 构造器是其特例。mesh/tess 管线只是更长的列表，无需新构造器。
- `Compiler` 新增 `compile_*_with_options`（capabilities + 预处理器 `macro_define` 对）与 `ShaderCache`（按 path/source + entry + caps + defines 记忆化），shader 变体只编译一次并共享。
- `Reflection::vertex_layout(entry)` 从顶点入口的 varying 输入推导 `VertexBufferLayout`（struct 字段展开；紧凑、4 字节对齐打包，与既有 `PodVertex` 约定一致）。
- `Reflection::root_parameters(entry)` + `RootBinder` 构建 push-data blob：`Ptr<T>` root 携带 GPU 地址，`uniform` root 携带内联字节，全部落在反射报告的偏移上。core_3d 与 egui 现在都经它录制 draw。
- `Reflection::compute_thread_group_size(entry)`（numthreads），以及 `struct_rust_source`（输出带偏移的 `#[repr(C)]` 骨架）与 `field_user_attributes` 作为编辑器元数据接缝。
- workspace 升级到 2024 edition（启用 `if let ... && ...` 链；既有嵌套 `if` 站点同步改写）。

## Alternatives considered

- 用 Slang 反射的 `name_override()` 取入口名：错误——它报告的是源级 override；锁定的 shader-slang-rs rev 对普通命名入口返回 `None`，而发射出的 SPIR-V 是 `main`。
- 从发射出的 SPIR-V execution model 解析 stage 而非反射的 `EntryPoint::stage()`：反射已暴露它；SPIR-V 解析只用于入口名。

## Consequences

- stage/entry/顶点布局/根布局契约现在在管线构造时被机器校验：`[shader(...)]` 标注不匹配、缺 stage 信息、结构体尺寸漂移都会大声失败（egui 在管线构建时用反射断言 `EguiRoot` 尺寸）。
- shader 编辑不再需要为常见形态（双 stage 图形、单 compute）同步改宿主代码；新 stage 只是数据。
- GPU 管线测试原样通过（两条主管线现在都从反射构造），5 个新 RHI 单测覆盖缓存记忆化、变体 defines、顶点布局推导、root blob 与多 stage（compute+graphics）文件。
- 已知限制：在锁定的 shader-slang-rs rev + SPIR-V target 下 `field_user_attributes` 返回空（probe 验证）；API 形态保留并宽松断言，Slang 升级时会暴露出来。