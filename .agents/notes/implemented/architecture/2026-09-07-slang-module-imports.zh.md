# Agent Note: Slang module imports resolve through the module-name path hint

Status: implemented

[English](2026-09-07-slang-module-imports.md)

## Problem

Gaussian Splatting 路线图的 M2 要把 `assets/shaders/gs/gaussian.slang` 做成共享库——训练内核与渲染 shader 共同 import 它的协方差/投影数学——而 workspace 此前没有任何 shader 用过 `import`。RHI 的源字符串编译传给 Slang 的是假文件名 hint（`{module_name}.slang`），资产编译的源没有目录上下文，import 无法解析。

## Decision

`compile_source*` 把 `module_name` 原样透传为 Slang 的文件路径 hint:是真实路径的模块名让源码从该文件所在目录 import 同目录模块;裸名则按进程工作目录解析。hint 文件**不需要存在**——只有它的目录需要——所以消费者可以用虚拟路径(`assets/shaders/gs/__gs_math_test.slang`)编译 wrapper 源码,磁盘上无 fixture。shader cache 的键本来就含模块名,import 对记忆化的参与是正确的。跨目录 import 不在本机制覆盖内;`SessionDesc::search_paths`(shader-slang-rs 已暴露)是消费者需要时的退路。

`gaussian.slang` 是库模块而非资产:管线 shader 是 import 它的资产,编译器的模块系统——而非资产层——拥有它的解析。文件头声明视空间约定(相机看向 +z、x 右、y 下——COLMAP/3DGS 形状);从引擎相机矩阵的换算发生在装配 `SplatView` 处,不在本文件。

验证是 `render-feature` 的 `gs_math` 测试:64 个种子化高斯经 GPU 跑 `cov3d`/`project`/`eval_color`,逐分量对比**独立的** glam 参考(glam 自己的四元数与矩阵运算,绝非 Slang 公式的转写),相对容差 1e-4。

## Alternatives considered

- **现在就上 session 搜索路径。** 落选:今天没有消费者需要跨目录 import;路径 hint 以零 API 表面覆盖同目录场景(gs 内核、测试 wrapper)。搜索路径留作退路。
- **转写式 CPU 参考(Slang 公式的 Rust 拷贝)。** 落选:任何转写错误它都会跟着"对上";glam 路径正是下方矩阵语义错误显形的原因。
- **纯标量数学(spike 风格)。** 落选:手展开的三角函数撑不起协方差/EWA;矩阵形式只需把语义修对一次。

## Consequences

- Slang 矩阵语义,探针验证、参考确认:`float3x3(v0, v1, v2)` 取行;`m[i][j]` 是 [行][列];矩阵之间的 `*` 是分量乘(HLSL)——矩阵积与矩阵×向量一样必须走 `mul()`。分量乘这个陷阱最险:类型全对、编译全过,`A·Aᵀ` 静默退化为 `A⊙Aᵀ`。
- `no_diff` 结构体参数(`SplatView`)在 `[Differentiable]` 函数中被接受;非 `public` 的结构体字段对 import 方不可见,共享结构因此显式携带 `public`。
- 被 import 的模块每次编译都从磁盘加载;rhi cache 按导入方源码做键,进程内只改库文件不会使导入方的缓存编译失效。当前没有消费者在运行时改 shader 文件,热重载按资产决策仍在范围外。
- rhi 的 import 探针(`source_import_resolves_through_module_name_path`)编译一个 import `gaussian` 并调用全部三个函数的 wrapper,机制保持受测。
