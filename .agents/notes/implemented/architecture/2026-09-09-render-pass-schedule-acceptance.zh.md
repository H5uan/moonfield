# Agent Note: Render pass schedule acceptance

Status: implemented

[English](2026-09-09-render-pass-schedule-acceptance.md)

## Problem

[重构](../../proposed/architecture/2026-09-09-render-pass-schedule-redesign.zh.md)逐里程碑落地（schedules 入 world、extract schedule、集合链与逐 view schedule、render command、storage image）；最后一条验收要求证明这套机器服务得了 feature 作者：一个 pass 以"新文件 + 一次注册"加入，相对 opaque pass 有序，录制真实 GPU 工作——不碰 render-feature 核心。

## Decision

- `splat/sort_pass.rs` 就是这个 pass：`prepare_splat_sort` 在 `PrepareViews` 构建 radix 管线与配对缓冲（着色器源码暂为内嵌，GS 集成时改走资产管线）；`sort_splats` 在 `Core3d` 逐 view 运行、排在 `opaque_pass_3d` 之后，把排序录制进帧命令缓冲；`register_sort_pass` 就是全部注册面。
- 验收测试组合真实帧循环：`RenderPlugin` + `RenderFeaturePlugin` + `register_sort_pass` + 一台相机。一个约束为 `after(&opaque_pass_3d).before(&sort_splats)` 的探针系统证明顺序——除非 opaque pass 先于 sort，其约束不可满足——而种子化的配对集合经录制、帧循环提交的 dispatch 往返，与 CPU 稳定排序逐项相等。
- 落位评审通过：GS forward 链有家——投影、分桶、排序作为 `Core3d` 里 opaque pass 之后的逐 view 系统，产物在缓冲，blend 写 RGBA16F 中间图像，composite 把色调映射写进 view target——与 [GS 路线图](../../proposed/architecture/2026-09-07-gaussian-splatting-implementation-roadmap.zh.md) M3 措辞一致。

验收暴露的一个缺口，刻意保持现状：`in_set` 成员资格在注册时展开，事后向现有链条追加集合不会约束早先的成员。因此 splat 链锚在 opaque pass **系统**上（`.after(&opaque_pass_3d)`）而非一个 splat 集合；每集合末锚（Bevy 式整集合排序）是第二个 feature 需要插入 `Core3d` 链条时的重启条件。

## Alternatives considered

- **把重构 note 移入 `implemented/`。** 落选：implemented note 记录已交付现实（Decision/Consequences），不是带验收清单的提案；各里程碑 note 已承载实现记录，重构 note 留在 proposed 并勾满复选框。
- **`Core3d` 链条里的 splat 集合。** 落选：今天它要求链条一次性注册，成员交错仍要对着 opaque pass 系统排序；一条锚约束说了同样的事且零 ECS 改动。
- **在 opaque pass 上加标记证明顺序。** 落选：那要改 render-feature 核心，恰是验收禁止的；受约束的探针从外部证明同一条边。

## Consequences

- 重构的验收清单全部勾满；"加一个 pass = 一个文件 + 一次调用"端到端 GPU 验证。
- sort pass 的配对是合成的——GS M3 落地时由 splat 抽取替换，落位即本 note 评审的落位。
- editor 回归保持：editor 的 41 个测试与 workspace 套件原样通过，viewport/window/egui 路径不受 splat feature 影响（不调用就不注册任何东西）。
