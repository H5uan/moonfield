# Agent Note: Slang shaders organize into explicit modules

Status: implemented

[English](2026-09-24-slang-module-system.md)

## Problem

`assets/shaders/` 下的所有 shader 都是 legacy 单文件：没有 `module` 声明，
编译器把全部符号当作隐式 public（Slang 为前模块代码保留的兼容模式，用户
指南保留将其废弃的权利）。唯一的共享库 `gs/gaussian.slang` 已经在用
`public` 修饰符却不属于任何声明的模块；`editor_metadata.slang` 根本不是
shader 依赖——反射测试靠 Rust 侧把它的文本拼进探针源码
（`include_str!` + `concat!`），这是一种宿主侧不该拥有的共享机制。也没有
任何既定模式说明多文件模块或可导入库该怎么组织，随 shader 数量增长，
结构只能逐个即兴发挥。

## Decision

全部 shader 加入 Slang 模块系统（Slang 2026.16.1，`shader-slang-rs-sys`
已 pin），采用用户指南推荐的布局：顶层文件是可导入的模块主文件，实现
细节放子目录。

- `gaussian.slang`（自 `gs/` 上移）是 `gaussian` 模块的主文件：一句
  `module gaussian;` 声明加 `__include` 列表。实现拆为 `gs/types.slang`、
  `gs/projection.slang`、`gs/color.slang`，各以 `implementing gaussian;`
  开头。拆分线顺着模块自身的接缝——数据类型、投影数学（`cov3d`、
  `project`）、颜色求值——并把四元数转旋转矩阵的代码提取成模块内部的
  `quat_rotation` helper，让模块拥有一个真正的非公开符号。
- `editor_metadata.slang` 成为可导入的 `editor_metadata` 模块，属性结构体
  标 `public`——编辑器元数据属性类型是在**其他**模块的 shader 源码里使用
  的，`internal` 会把它挡在模块边界之内。反射测试改为编译一个
  `import editor_metadata;` 的探针，不再拼接文件文本。
- 入口 shader——`core_3d.slang`、`egui.slang`、`util/radix_sort.slang`、
  `ml/adam.slang`——各自声明模块（`module core_3d;` 等）并保持默认
  `internal` 可见性：它们被直接编译、从不被导入，没有东西需要跨过它们
  的边界。
- 两个编译测试钉住语义：一个探针经模块名路径提示导入 `gaussian`（如今
  解析到顶层主文件）；`internal_symbols_are_invisible_across_import`
  断言从导入方模块取 `quat_rotation` 无法编译——证明这些声明是承重的，
  不是装饰。

不添加 `#language slang 2026;` 指令：显式 `public` 修饰符已覆盖模块所需的
可见性，而启用 2026 的成员可见性默认值会改变当前没有代码依赖的语义。

## Alternatives considered

- **文件原地不动，只加 `module` 声明。** 放弃：gs/ml/util 目录会继续充当
  手工命名空间，而模块名又构成另一套命名空间，顶层目录也依然分不清哪些
  是可导入 API、哪些是入口实现。用户指南的约定（主文件在上、细节在下）
  只花两处测试路径的改动就消除了歧义。
- **`gaussian` 保持单文件声明，以后再拆。** 放弃：108 行的体量拆分很便宜，
  而多文件模式（`module` + `__include` + `implementing`）需要趁任何模块
  真正膨胀之前在仓库里立下模板——否则第一次真正的拆分又会即兴发明结构。
  拆分还需要一个内部 helper（`quat_rotation`），这恰是模块系统要表达的
  访问控制面。
- **`editor_metadata` 继续用 `include_str!` 拼接。** 放弃：它把模块依赖
  复制到 Rust 源码里，而不是在依赖所在的 Slang 侧声明；而且任何真正想带
  `[EditorColor]` 式元数据的 shader 都没法复用它，除非宿主再拼更多文本。
  虚拟路径导入的模式已被 gaussian 探针测试验证过。
- **`editor_metadata` 的属性结构体用 `internal` 导入。** 放弃：属性结构体
  是从导入方模块的源码里引用的，这正是 `internal` 所禁止的；`public` 才
  是正确的修饰符，反射测试验证了属性仍能跨导入 surfaced。

## Consequences

- legacy 模块模式退出：未声明的符号从此是 `internal`，未来一次意外的跨
  模块引用会在编译期失败，而不是静默解析成功。
  `internal_symbols_are_invisible_across_import` 测试让这份契约保持可观测。
- 预处理状态不跨 `import`/`__include` 边界传播。今天这没有代价——
  `defines` 变体只喂给被直接编译的入口模块——但未来想要宏驱动行为的模块
  必须自带配置，不能继承导入方的宏。
- 内存中的 wrapper shader（测试、未来的训练 kernel wrapper）必须以一个
  把自己放到被导入模块旁边的模块名编译；虚拟路径约定
  （`assets/shaders/__*_probe.slang`）从此成为既定做法，并随
  `gaussian.slang` 一同迁移。
- `gaussian` 模块的公开面恰好是它的 `public` 符号：三个数据类型、
  `cov3d`、`project`、`eval_color`。其余（目前是 `quat_rotation`）都是
  模块可以自由重组、不触及导入方的实现细节。
- SPIR-V 产物不变：拆分后 gs_math 测试对参考实现的数值断言仍然逐位通过，
  这是一次没有任何代码生成漂移的纯结构改动。
