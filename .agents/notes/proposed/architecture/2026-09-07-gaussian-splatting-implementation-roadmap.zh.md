# Agent Note: Gaussian Splatting implementation roadmap

Status: proposed

[English](2026-09-07-gaussian-splatting-implementation-roadmap.md)

## Problem

训练栈([2026-09-04-slang-autodiff-gaussian-training](../../implemented/architecture/2026-09-04-slang-autodiff-gaussian-training.md))与 crate 划分([2026-09-04-ml-training-crate](../../implemented/architecture/2026-09-04-ml-training-crate.md))已成定论,实现本身尚未排期:`moonfield-ml` 只有 trait 骨架、没有内核,`render-feature` 的 splat 光栅化器与计算工具仍是占位,五个设计点悬而未决——梯度累积、前向内核在观看与训练之间的共享、深度排序、SH 训练节奏、验收数据。

## Proposal

固定决策:

- **梯度累积用 buffer float32 原子操作**(`VK_EXT_shader_atomic_float`;开发机的 T1000 暴露 `shaderBufferFloat32AtomicAdd`)。Slang 经 `__atomic_add` 内建发出 `OpAtomicFAddEXT`——GLSL 风格的 `atomicAdd` 对 Slang→SPIR-V 编译不可见——已由 `gpu_tests::float_atomics` 端到端验证。内核每步清零梯度缓冲。训练结果不再逐位可复现:训练回路的回归判据改为统计值(loss/PSNR 阈值),观看路径保持精确。
- **一套可微的 tile 前向同时服务观看与训练。** 它把每个 tile 排序后的高斯列表与每像素透射率写入缓冲;`render-feature` 把它录成 `Core3d` schedule 里的逐 view 系统([render pass schedule redesign](2026-09-09-render-pass-schedule-redesign.md)),`moonfield-ml` 在每个训练步里录同一组分发。观看路径为中间产物的写出付带宽。
- **深度排序用 `render-feature::gpu_util` 的 GPU radix sort**,连同排序与 tile 分桶所需的前缀和与展开 pass。不存在 CPU 排序路径。
- **训练只拟合 0 阶 SH(DC)**;`sh_rest` 保持零填充。更高阶及其渐进解锁进 backlog。
- **验收在开发机上跑公开 COLMAP 场景**(Tandt_db 的 truck);CI 没有 GPU,这些测试跳过。阈值从首个稳定的本地基线减去裕量钉定——不取文献峰值——并记录在本 note 里。

里程碑,每个以可运行的证明收尾:

- **M1 —— `moonfield-ml` 训练回路跑在 RHI 公共 API 上。** `Trainer::run` 走 `submit_and_wait`,Adam 内核(`assets/shaders/ml/adam.slang`),一个最小 `TrainingMethod` 以 `__atomic_add` 反向重拟合 `gaussian_fit` 目标。RHI 的可选扩展表加入 `VK_EXT_shader_atomic_float` 与 `buffer_float32_atomic_add` 能力查询。
- **M2 —— 共享 splat 数学与计算工具。** `assets/shaders/gs/gaussian.slang` 补上 3D 协方差、EWA 投影、DC 颜色(全部 `[Differentiable]`);`gpu_util` 补上 radix sort 及其 scan。删除 `rasterize.rs` 与 `gpu_util.rs` 里的里程碑编号注释;序列由本 note 持有。
- **M3 —— tile 前向。** 投影、tile 分桶、radix sort、alpha 混合写入 RGBA16F 图像,中间产物落缓冲;composite pass 做 tonemap 写入 view target([render pass schedule redesign](2026-09-09-render-pass-schedule-redesign.md))。`render-feature` 将其录成 `Core3d` schedule 里的逐 view 系统(先离屏测试,后编辑器视口);`moonfield-ml` 每步录同一前向。
- **M4 —— 3D 训练回路。** 在 tile 列表上反向(`[BackwardDerivative]` 包裹全局内存副作用)、按属性 SoA 的 Adam、L1 loss、`readback_loss`。数据集视图经 COLMAP 加载器进入(扩展到二进制模型),外加图像解码器(新依赖),分辨率按 4 GB 预算封顶。
- **M5 —— 密度控制与 checkpoint。** 位置梯度累积、clone/split/prune 与 SoA 重建、checkpoint 往返、`KHR_gaussian_splatting` 导出、编辑器重载。

Backlog,明确不在范围内:1–3 阶 SH 与解锁节奏、D-SSIM loss 项、2DGS / Stoch3DGS 内核族、ReSTIR 集成、编辑器训练面板、timeline 重叠提交、PINHOLE/SIMPLE_PINHOLE 之外的相机模型。

## Alternatives considered

- **warp/block 梯度归约(确定性、无扩展)。** 落选:为了让只有回归测试消费的性质保活而把反向内核复杂化,不值得;原子操作与参考实现的形状一致。若 M1 探针表明 Slang 在本驱动上发不出 buffer float 原子操作,此备选重开。
- **spike 的 per-(pixel, Gaussian) 梯度缓冲加归约 pass。** 落选:内存随 像素 × 高斯 × 参数 增长,场景规模下不可行;它只活在 rhi 的 `gaussian_fit` 测试里。
- **单独写一套不可微的观看光栅化器。** 落选:同一算法两套内核族必然漂移;共享前向的代价只是观看路径多付中间产物带宽。
- **CPU 深度排序。** 落选:每个训练步都会引入一次 GPU→CPU 同步,把 CPU 塞回单设备设计本要移除的训练循环。
- **从第一步就训全部 SH 阶。** 落选:位置还在大幅移动时,视角相关项没有可拟合的信号——参考实现因此渐进解锁各阶;只训 DC 让 M4 收缩到其单视角验收能度量的范围。
- **只用合成场景验收。** 落选:程序生成的目标通过了,COLMAP 摄取、图像解码、相机数学仍未被检验;公开场景的代价只是本地运行——CI 本来就不跑 GPU 测试。

## Acceptance criteria

- M1:`cargo test -p moonfield-ml` 仅经公共 API 把 `gaussian_fit` 目标重拟合到终/初 loss 比 ≤ 0.2。
- M2:GPU 探针让乱序键经 radix sort 往返;协方差与投影在容差内对上 CPU 参考。
- M3:存在离屏渲染测试,编辑器视口渲染一份参考 `KHR_gaussian_splatting` glTF。
- M4:`cargo run -p moonfield-ml --example train` 把 truck 单视角过拟合到钉定的 loss 比。
- M5:truck 在封顶分辨率下训到钉定的 DC-only PSNR;导出 → 编辑器重载往返。

## Risks

- truck 的相机模型必须是加载器支持的 PINHOLE/SIMPLE_PINHOLE;不匹配则 M4 变大。
- T1000 的 4 GB 封顶图像分辨率、高斯数量与 tile 列表大小;基线与阈值在封顶分辨率下定义。
- 二进制 COLMAP 解析与图像解码器是 M4 的新表面;任一项都可能超出预估。
- 单视角过拟合(M4)可能带着只有多视角训练才暴露的缺陷通过;M5 的运行才是闸门。
