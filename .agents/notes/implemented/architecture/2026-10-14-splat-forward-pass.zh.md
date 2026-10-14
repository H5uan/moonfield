# Agent Note: Splat forward pass

Status: implemented

[English](2026-10-14-splat-forward-pass.md)

## Problem

[Gaussian splatting 路线图](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.md)的 M3 需要基于 tile 的前向 pass；此前 splat 相关代码完全不接触 GPU：splat 模块只有场景数据、加载器和一个光栅化占位——没有上传路径、没有抽取、没有 kernel——splat 云无法出现在任何一帧里。

## Decision

前向是一串按视口录制在 `Core3d` schedule 中不透明 pass 之后的 compute dispatch（见[渲染 pass schedule 重设计](2026-09-09-render-pass-schedule-redesign.md)），随后合成进视口目标。kernel 位于 `assets/shaders/gs/forward.slang` 与 `gs/composite.slang`；Rust 侧是 `render-feature` 的 `splat::{forward, prepare, forward_pass}` 模块。五个决策决定了它的形状：

- **tile 分桶是原子追加，没有前缀和。** `duplicate_keys` 对每个覆盖的 tile 从全局计数器取一个槽位（`InterlockedAdd`），把（tile、depth、Gaussian）三元组写入该槽。超过条目容量（N×16）的部分丢弃并计数——可观测，不致命。
- **(tile, depth) 复合序是对全部条目容量做两次稳定 u32 `RadixSort`。** `clear` kernel 每帧用 `0xFFFFFFFF` 哨兵填充 tile 键并清零计数器，因此两次排序与 `tile_ranges` 都按容量运行，追加的条目数从不离开 GPU——没有帧中回读。两次排序之间的 `gather` entry 按深度排序的槽位置换重读 tile 键与 Gaussian id。
- **composite 是 compute pass，不是图形 pass。** `OffscreenTarget` 带 `STORAGE` usage 并暴露 `storage_handle()`；kernel 通过描述符堆的 storage 槽位读取 splat 纹素与视口目标自身的纹素，写回 `dst * (1 - alpha) + reinhard(color)`——同一线程内对同一纹素先读后写，没有反馈拷贝（堆槽形状见 [rhi storage image](2026-09-09-rhi-storage-image.md)）。每视口工件上的 `recorded` 标志使 composite 对每次 blend 恰好执行一次，过期的 splat 图像永远不会进入视口目标。
- **kernel 为 M4 训练器做好了共享准备。** 每个 entry 接收一个 `uniform` 参数 blob——指针字段加内联标量——作用于调用方持有的 buffer，因此训练器的裸 `CommandBuffer` 面可以录制同样的 dispatch；M3 只接通渲染器一侧（经由[渲染录制上下文](2026-09-09-render-recording-context.md)的 compute 门，其自动 `COMPUTE → COMPUTE` barrier 串行化每一环）。
- **COLMAP ↔ moonfield 相机约定翻转集中在一处。** `forward_pass::splat_view` 给世界→视图旋转左乘 `diag(1, -1, -1)`，并从投影矩阵的余切项推导焦距；gs 模块全程保持 +z 朝前、y 朝下的约定。

实现过程中钉住的两条契约教训：

- **`Ptr<T>` 指向体使用紧凑的自然布局**（`float3` 为 12 字节、4 对齐）——而不是 `Reflection::struct_layout` 默认规则所报告的样子（16 对齐、std430 风格）。rhi 新增了 `Reflection::field_pointee_layout`，经由指针字段反射出真正烘进指针运算的布局；`splat::forward` 中的记录镜像（`Gaussian3D`、`SplatView`、`SplatRecord`）以它为准钉住布局（指针约定见[bindless rhi GPU 指针模型](2026-08-19-bindless-rhi-gpu-pointer-model.md)）。
- **`eval_color` 原样返回 `sh_dc`**——`0.282095·c + 0.5` 的 SH 偏置在前向路径中没有实现，因此 DC 项直接存放可显示的 RGB（合成资产就是这么存的）。是否实现该偏置由 M4 决定；若实现，`assets/models/synth_splats.glb` 与 `gs_forward` 测试的颜色需要重新生成。

## Alternatives considered

- **把 `RadixSort` 扩展到 u64 键。** 否决：要为省下几次 pass 去改动 `radix_sort.slang` 和全部布局测试；两次稳定 u32 排序对 shader 零改动，代价只是多四次 scatter。
- **参考实现的逐 Gaussian 前缀和分桶。** 否决：需要把扫描 pass 从 `RadixSort` 里拆出来独立使用；原子追加加排序以少一族 kernel 达到同样的逐 tile 顺序。
- **图形 composite（全屏三角形加 alpha 混合状态）。** 否决：对视口目标边采样边渲染是反馈回路，而拷贝一份目标的代价比 compute 读改写更高。
- **直接 blend 进视口目标。** 否决：目标是 8 位且没有色调映射的落点——[渲染 pass schedule 重设计](2026-09-09-render-pass-schedule-redesign.md)已经否决过。

## Consequences

- 视口路径每帧支付工件写入带宽（记录、条目与逐像素透射率），这是路线图共享前向决策的代价；透射率 buffer 是 M4 反向的输入。
- 条目容量为每视口 N×16；覆盖 tile 超出份额的 splat 会丢弃条目并增加溢出计数，而不会破坏排序。
- 编辑器默认特性包含 `splat`，`cargo run` 即可渲染 splat 云；合成参考资产由 `cargo run -p moonfield-render-feature --example gen_splat_asset --features splat` 生成（示例会把产物经 `load_splat_gltf` 回读校验）。
- M3 验收达成：`tests/gs_forward.rs` 含两个带像素断言的 GPU 测试（向 RGBA16F 图像的深度序混合；在已知清屏色之上向视口目标的色调映射合成），编辑器视口能渲染 `assets/models/synth_splats.glb`——已通过 `MOONFIELD_EDITOR_DUMP_VIEWPORT` 转储冒烟验证，画面中能看到位置着色的点云覆盖在茶壶上。
