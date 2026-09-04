# Agent Note: Geometry through pointers — vertex pulling lands

Status: implemented

[English](2026-09-04-vertex-pulling.md)

## Problem

mesh 管线是仅剩的非 bindless 路径：`GpuMesh` 持有一对 `Buffer`，经固定功能
input assembler 绑定（每 draw `bind_vertex_buffers` / `bind_index_buffer`，
外加烙进管线的 per-mesh 顶点布局），几何放在 host-visible 内存
（`Memory::Default`）里由阻塞上传写入。RHI 本已是 buffer-device-address
到处的世界，顶点路径却为指针已经够得着的数据支付固定功能的仪式。

## Decision

- `DrawData` 长成每 draw 记录的终态：`{ model, color,
  positions: Ptr<float3>, indices: Ptr<uint>, index_count }`——一个 draw
  需要的一切，藏在一个指针背后。顶点着色器唯一的 stage 输入是
  `SV_VertexID`；它通过记录里的指针拉取两个数组
  （`vi = indices[vid]; position = positions[vi]`），draw 变为非索引
  （`draw(index_count, 1, 0, 0)`）。
- `GpuMesh` 是一对 GPU-only 的 `GpuAllocation`；几何经共享帧 uploader
  暂存（`upload_alloc`，每帧一次 flush、先于帧命令缓冲——egui 上传已经在
  用的同队列顺序）。
- 管线接受空顶点布局：不发射任何 binding/attribute 描述——拉取管线没有
  input assembler，正是 mesh-shader 管线本来的形状。
- `SV_VertexID` 反射出的 category 是 `None`、带语义名，varying-input 过滤
  天然把它排除在派生顶点布局之外——无需特判（`pulling_vertex_shape` 钉死
  这一点）。

## Alternatives considered

- **先做共享几何 arena 再拉取。** 拒绝此顺序：拉取自己就能消掉每 draw 的
  绑定（shader 拿指针），arena 只解决分配卫生——那是更晚的独立一步。
- **几何指针用独立 root 参数。** 拒绝：每 draw 一条记录才是 instancing
  与 indirect draw 将来消费的形状；uniform 结构体内嵌 `Ptr` 字段按 natural
  偏移布局（已验证），记录直接携带。

## Consequences

- `DrawMesh` 录制一个 draw 只需一次管线绑定、一次 8 字节 push、一次
  `draw`——没有顶点/索引绑定。`BufferUsage::VERTEX`/`INDEX` 词汇在 mesh
  路径上已无生产用户。
- 乱序 teardown（GPU 资源比 `RenderDevice` 活得久）在这项工作中崩了：
  退役环里未执行的 action 持有最后的 `Arc<Allocator>`，其析构经由已销毁
  的设备释放内存。`Device::drop` 现在在仍有 allocation Arc 时改为泄漏
  （设备与分配器，带 error 日志）而不是销毁；`Instance` 通过共享计数器
  追踪活设备，自己的 `Drop` 在有活设备时同样泄漏而不是围着活设备销毁。
  测试把 `RenderDevice` 插在最前，镜像真实插件顺序——守卫是机制，顺序是
  约定。
- GPU 端到端验证：opaque pass 像素测试原样通过（draw 以同样的变换数学
  拉取同样的几何）。
