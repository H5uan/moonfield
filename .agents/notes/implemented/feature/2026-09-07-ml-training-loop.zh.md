# Agent Note: The moonfield-ml training loop runs on the public RHI API

Status: implemented

[English](2026-09-07-ml-training-loop.md)

## Problem

[Gaussian Splatting 路线图](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.md)的 M1 是训练回路:`moonfield-ml` 只有 `Trainer`/`TrainingMethod`/`Adam` 的 trait 骨架,没有内核、没有提交循环,也没有任何东西证明完整栈——原子梯度累积、经资产加载的优化器内核、宿主循环——只经 RHI 公共类型跑通。

## Decision

`Trainer` 跑同步循环:一个命令缓冲,每步重录(`begin` → `TrainingMethod::record_step` → `end` → `Device::submit_and_wait`),loss 在第 1 步、每 `report_every` 步、最后一步经 `readback_loss` 读回。命令池是纯保活字段,与 `FrameContext` 同一惯例:命令缓冲在 `Drop` 里经池句柄释放自身,池必须活得比它长。

`Adam` 接收已加载的 `Shader` 资产,经设备 `ShaderCache` 从其源文本编译(`compile_source` / `compile_source_reflection`,均按源文本键)——训练内核与任何管线 shader 一样消费资产层([2026-09-06-shader-as-asset](../architecture/2026-09-06-shader-as-asset.md));路径选择归应用装配。内核的全部边带信息是一个 `Ptr<AdamConfig>` 根参数:24 字节的 `#[repr(C)]` 镜像(step、count、四个超参——六个 4 字节标量,两侧同自然布局),宿主每次派发经持久映射推进 `step`。

M1 方法(`tests/gaussian_fit.rs`)把 rhi spike 的 2D 问题经该循环重拟合:backward 用 `__atomic_add` 把每个像素的贡献直接累加进每高斯梯度槽——这是 Slang→SPIR-V 编译下可调用的拼法(路线图的固定决策)——spike 的 per-(pixel, Gaussian) 记录与归约 pass 已删除。梯度缓冲在每个录制步开头由宿主清零;同步循环使此刻 GPU 空闲,生产方法(M4)按路线图改为内核侧清零。

## Alternatives considered

- **Adam 内核走文件路径编译(`ShaderCache::compile_file*` 加烘焙目录)。** 落选:它复活了 shader-as-asset 决策刚删掉的 `CARGO_MANIFEST_DIR` 查找,且路径键的缓存看不见同路径的源码变更。
- **超参用标量 uniform 根参数。** 落选:一个内核两种 root-data 机制;结构体指针让全部内核输入都是指针,并把步数计数折进同一 24 字节。
- **spike 的记录加归约反向。** 落选:记录量随 像素 × 高斯 × 参数 增长——场景规模下不可行,路线图因此选了原子。

## Consequences

- `cargo test -p moonfield-ml` 端到端证明回路:Adam 内核对上两步手算,且梯度逐步不同(0.9,再 0.806770——梯度不同使"动量被清零"与"步数卡住"各自可观测);Gaussian-fit 验收以纯公共类型驱动 600 迭代,终/初 loss 比 ≤ 0.2。
- **Slang 2026.14.1 对 autodiff 加原子组合错误编译。** lockfile 在 shader-slang-rs 374b76c 时,验收跑完迭代后崩溃——SIGSEGV,位于命令缓冲拆除时驱动内部的 `free_command_buffers`。lockfile 现解析到 e71b713(Slang 2026.16.1),同一份源码编译运行正确;ml 需要 slang ≥ 2026.16.1。
- 训练结果是统计值,不逐位可复现(原子交错);回归判据是阈值。
- 步数计数每次派发由宿主经持久映射写入——同步循环下安全;重叠提交是路线图 backlog。
