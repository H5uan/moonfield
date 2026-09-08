# Agent Note: A deterministic GPU radix sort in gpu_util

Status: implemented

[English](2026-09-07-deterministic-radix-sort.md)

## Problem

Splatting 在 alpha 混合前按视空间深度排序高斯,而路线图的固定决策要求观看路径跨运行逐位一致。最简单的排序形状——原子散射——按原子到达顺序排列同 bin 元素,深度相等的高斯每次运行会以不同顺序混合。workspace 此前没有 GPU 排序。

## Decision

`gpu_util::RadixSort` 是 (u32 键, u32 值) 对上的 LSD 基数排序,编译自 `assets/shaders/util/radix_sort.slang` 资产:8 位一位、四个 pass、每 pass 三次 dispatch。histogram(每 256 元素 tile 一组)在 groupshared 内存里数 bin;scan(单工作组)以分块 Hillis-Steele 加跨块进位对扁平直方图做排他扫描;scatter(每 tile 一组)把每个元素放到其扫描基址加 wave 基址加 lane 前缀处。

确定性是结构性的:每个 scatter rank 都出自扫描偏移与 lane 序,绝不出自原子到达序。仅有的原子是计数原子,其结果与顺序无关。排序是稳定的——相等键保持输入序——重跑逐位复现输出。

直方图是 bin 主序(`hist[bin * groups + group]`):flat 扫描先排 bin,使每个 bin 的跨组基址连续。调用方传入自己的输入输出对;排序持有乒乓临时缓冲与直方图/偏移 scratch,输入只读一次,输出只写一次。一个 pass 内各 dispatch 之间的 barrier 由内部录制;无尾部 barrier——与其他工作的排序归调用方。

每个 entry 把参数收敛为单个 `uniform Params` root 结构体(`HistogramParams`、`ScanParams`、`ScatterParams`),宿主在 `gpu_util.rs` 里用 `#[repr(C)]` bytemuck POD 类型逐字段镜像:指针字段是 8 字节 GPU 地址,标量字段是内联 u32,尾部 `_pad: u32` 保证镜像结构体零填充、满足 Pod。RHI 在这里是完全无描述符集架构——descriptor heap、push data 与裸 GPU 指针——因此 `RadixSort` 每个 entry 只反射一次 root placement(`uniform_param("params")`),每次 dispatch 用一次 `push_data` 写整个 root blob,不再逐个推指针/标量。`#[cfg(test)]` 布局测试用与 `RadixSort::new` 相同的 `RootBinder` 查询编译同一份 shader,断言每个 Rust 镜像的总大小与各字段偏移等于反射出的 entry blob;shader 与宿主结构体漂移会在测试期失败,而不是腐蚀 GPU 内存。

## Alternatives considered

- **原子散射(每 bin 计数器 `atomicAdd`)。** 落选:bin 内顺序变成原子到达序——不确定,相等键跨运行混合顺序不同,观看路径失去精确性。
- **唯一复合键(深度, 索引)下的不稳定 pass。** 落选:即便键唯一,不稳定 LSD 基数也不正确——最后处理的 digit 并列时,不稳定 pass 丢弃了更早 digit 建立的顺序。
- **bitonic 网络。** 落选:splat 数量下每次视角变化 O(n log² n) 次比较交换;radix 是参考实现的形状。
- **组主序直方图布局(`hist[group * 256 + bin]`)。** 落选:flat 扫描先排组,每组的 bin 连续,每组散射进自己的输出段——组内各自有序、组间永不合并。bin 必须是外层维度;验收测试抓住了这一点。
- **构建期自动生成镜像结构体(如由 `struct_layout` 驱动的 proc macro 或源码生成)。** 落选:宿主在运行时同一次构建里编译 shader,RHI 已经整体落在 descriptor heap 与 push data 上;三个小型镜像结构体加反射布局测试让契约显式、并在本 crate 内即可编译。没有 Slang 依赖变化、没有架构迁移,布局测试免费兜住任何漂移。

## Consequences

- 验收测试排序四个用例——4096 乱序键、跨组尾、单元素、重度重复——与 Rust 稳定 `sort_by_key` **精确相等**对比,兼作确定性证明。
- 一路验证的 Slang 事实:`groupshared` 必须在全局作用域声明(HLSL 允许函数内);`InterlockedAdd` 作用于 groupshared 正常;`WaveGetLaneCount()` 需要 `spvGroupNonUniform`,RHI 的 `spirv_1_5` session 经隐式 profile 升级授予。
- scan 是单工作组实现;百万级数量下它是优化点(两级扫描),API 不变。
- Splatting 在调用点把 f32 深度映射为保序 u32 键;排序本身对键类型保持无关。
- 反射布局测试是 shader/`repr(C)` 镜像对的漂移防线:锁定单 root 参数形态、blob 总大小与每字段偏移,对照 Slang 的 entry 参数布局(std430)而非 `LayoutRules::Default`,两侧无法无声分歧。
