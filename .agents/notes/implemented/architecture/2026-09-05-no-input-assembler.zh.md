# Agent Note: The fixed-function vertex path is gone - pulling is the only vertex story

Status: implemented

[English](2026-09-05-no-input-assembler.md)

## Problem

mesh 管线改为指针拉取之后，固定功能顶点路径只剩死表面苟活：
`BufferUsage::VERTEX`/`INDEX` 词汇、`bind_vertex_buffers`/
`bind_index_buffer` 命令、`draw_indexed`（全部三个变体）、per-pipeline
的 `VertexBufferLayout` 构造、`Reflection::vertex_layout` 推导——唯一
残存的生产用户是 egui 后端，为指针已经够得着的数据支付整套仪式。

## Decision

- egui 与一切拉取化对齐：`vs_main(SV_VertexID, uniform EguiRoot root)`
  取 `root.vertices[root.indices[vid + root.index_base]]`。上传时把每个
  mesh 的本地索引改写为绝对顶点索引，draw 只携带自己的索引区间；root
  的静态前缀（32 字节：屏幕尺寸、开关、两个数组指针）每 pass 推一次，
  16 字节尾部（texture、sampler、index base）每 draw 推一次。帧槽持有
  host-visible 的 `GpuAllocation`（每帧全量重写），不再是 `Buffer`。
- RHI 删除整块固定功能表面：类型（`VertexBufferLayout`、
  `VertexAttribute`、`VertexFormat`、`IndexFormat`、
  `DrawIndexedIndirectArgs`）、命令（`bind_vertex_buffers`、
  `bind_index_buffer`、`draw_indexed` 及两个 indirect 变体）、
  `Reflection::vertex_layout`、所有 `GraphicsPipeline` 构造器的
  `vertex_layout` 参数。管线不再发射任何顶点输入描述——mesh-shader 的
  形状。
- `draw_indirect`/`draw_indirect_count`（非索引）保留；indirect-draw
  测试的索引段改为双记录非索引 `draw_indirect`，顺带覆盖了旧路径从未
  覆盖的 multi-draw 参数解析（draw count 与 stride）。

## Alternatives considered

- **为特殊管线保留固定功能路径。** 工作区里无人需要它；死表面让每个
  读者都得问"这里适用哪种顶点故事？"。
- **只拉顶点、保留索引缓冲绑定。** 半套机制；`index_base` 进 root 的
  非索引 draw 同样一个 draw 的数据量，且没有可绑错的状态。

## Consequences

- 这个引擎里一个 draw 恰好是：管线绑定、root 数据推送、`draw`。不
  存在可出错的绑定顶点状态，且所有管线的顶点输入状态永远相同。
- 顺带钉住一个反射 ABI 事实：push-data bank 跨 stage 共享，且每个
  stage 的入口签名就是 ABI——只在其中一个 stage 声明的 root 依然从
  offset 0 占据 bank，因此拥有自己 root 的 stage 必须先声明共享的
  前导 root（否则两个 stage 的放置会静默碰撞）。
  `bindless_graphics_heap_sampling` 的片元 stage 声明顶点 stage 的
  前导 `Ptr` 正是这个原因；`egui.slang` 与 `core_3d.slang` 本来就
  共享签名。
- 所有像素测试在真实 GPU 上原样通过：经 `first_vertex` 偏移的
  `SV_VertexID` 拉取精确复现旧的绘制。
