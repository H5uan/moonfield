# Agent Note: glTF as the unified asset source format

Status: implemented

[English](2026-08-22-gltf-unified-asset-import.md)

## Problem

编辑器此前只能通过专用加载器读取 PLY splat 云，而且完全没有 mesh 资产
——viewport 把每个 `MeshRenderer` 实体都画成着色单位立方体，
`moonfield-render` 还为此保留着一个占位立方体 `scene::MeshRenderer`
（外加一个 serde 依赖），它唯一的职责就是这个占位。与此同时，
[场景保存/加载系统](../architecture/2026-08-21-bsn-style-scene-templates.zh.md)
已经采用 glTF 2.0 作为文本载体，于是工作区维护着两套互不相干的格式栈，
却仍然无法显示一个真实的 mesh。

## Decision

glTF 2.0（`.gltf`/`.glb`）是引擎唯一的资产来源格式，解析使用完整的
`gltf` crate（新增 workspace 依赖，features 为 `import` + `utils`）。
机制描述见 [docs/architecture.md](../../../../docs/architecture.md)。

- `moonfield-render-feature` 持有 `src/mesh/`：`Mesh` 资产（positions +
  indices 由私有字段加访问器持有，带预计算 AABB 和来源路径——与
  `SplatCloud` 同一形态）、`MeshHandle` 组件 newtype，以及
  `MeshRenderer` 组件（`#[reflect(ignore)]` 的 mesh 字段，可编辑的
  `color`）。`mesh/gltf.rs` 把文件中的全部 TRIANGLE 图元合并为一个
  mesh，索引加上顶点偏移，无索引图元合成顺序索引；POINTS 图元、节点
  变换和材质一律丢弃。
- `splat/io/gltf.rs` 取代已删除的 `ply.rs`：`KHR_gaussian_splatting`
  （Khronos RC）加载器，读取携带 `KHR_gaussian_splatting:*` 属性的
  POINTS 图元——仅支持 float componentType，kernel 必须为
  `"ellipse"`，不支持压缩子扩展（SPZ）。加载器把 glTF 渲染空间的值
  转换为 `GaussianScene` 保持的训练空间约定：scale → ln，opacity →
  logit，四元数 xyzw → wxyz，0 阶 SH 原样进入 `f_dc`，更高阶 SH
  转置进按通道分块的 `f_rest` 布局，缺失的阶补零。
  `SplatCloud::from_ply_*` 变为 `from_gltf_file`/`from_gltf_bytes`，
  `SplatLoadError` 现在是 `{Io, Gltf}`。由于 gltf-json 把未知的扩展
  语义映射为 `Checked::Invalid`，splat 加载器改用
  `Gltf::from_slice_without_validation` + `import_buffers` 解析，并从
  原始 JSON 读取属性映射；mesh 加载走带校验的 `gltf::import`。
- `moonfield-render` 获得深度支持——`OffscreenTarget::new_with_depth`
  （D32Sfloat）、`RenderPass::new_with_depth` 和
  `PipelineOptions.depth_test`（reverse-Z：清除值 0.0，比较
  `GREATER_OR_EQUAL`）——同时删除占位立方体 `scene::MeshRenderer`
  以及该 crate 的 serde 依赖。`tests/depth_occlusion.rs` 覆盖
  reverse-Z 路径。
- `moonfield-scene` 为编辑器的 `mesh_renderer` 注册项提供 roundtrip
  测试：一个路径字符串形式的自定义条目，其 load 钩子把
  `HandleTemplate<Mesh>::Path` 包进 `MeshRenderer` newtype。
- 编辑器用 `GltfLoader` 取代 `SplatCloudLoader`：它在文件字节中嗅探
  `"KHR_gaussian_splatting"` JSON 键，据此产出 `SplatCloud` 或
  `Mesh`；`load_asset`（原 `load_splat_cloud`）spawn 出以文件命名、
  携带对应组件的实体（`MeshRenderer` 使用 `DEFAULT_MESH_COLOR`）。
  viewport 通过 `AssetId → GpuMesh` 缓存把真实 mesh 画进带深度测试的
  目标，splat 的 AABB 占位继续用内部的单位立方体 mesh。旧
  `// DEBUG bypass mvp` shader 行掩盖的行/列主序不匹配也在根源上修
  复：Slang 默认按行主序打包 push-constant 矩阵，而 glam 的
  `to_cols_array()` 是列主序，因此 viewport shader 显式声明
  `column_major float4x4 mvp;`。

## Alternatives considered

- **splat 保留 PLY，glTF 只用于 mesh。** 否决：两种来源格式意味着两套
  加载器和两倍的故障面，而 glTF 用一个解析器加一次按内容嗅探的分发就
  覆盖两种资产类型。PLY 加载器是有意删除的；训练侧的互通日后由
  `KHR_gaussian_splatting` 导出器承担，而不是保留一个只进不出的第二
  格式。
- **手写 glTF 容器与 accessor 解码。** 否决：容器解析、外部 buffer 解
  析和 accessor 机制正是 `gltf` crate 已经实现并测试过的部分，重写毫
  无收益。唯一保留手工解码的是 splat 属性读取——这是 gltf-json 把未
  知扩展语义映射为 `Checked::Invalid` 所迫。
- **保留 glTF 场景图——节点变换、图元切分、材质。** 推迟：忠实的多图
  元、多材质导入需要 Material 系统和逐图元的绘制状态，而渲染器目前都
  没有。v1 把所有三角形图元合并为一个纯色的 `Mesh`；图元切分随
  Material 系统一同到来。

## Consequences

- PLY 文件在任何路径下都不再能加载；既有 splat 采集必须重新导出为
  `KHR_gaussian_splatting` glTF。
- 一个 glTF 文件产出一个 `Mesh`：图元边界、节点摆放和材质在导入时被
  抹平，因此依赖材质才有意义的文件在 Material 系统落地前都是纯色。
- splat 导入是严格的：非 float（量化）属性、非 ellipse 的 kernel、
  SPZ 风格的压缩子扩展都是显式报错，而不是静默降级。
- `mesh_renderer` 在场景文件中只是裸路径字符串；颜色不持久化——从场
  景加载回来的 mesh 一律使用 `DEFAULT_MESH_COLOR`。
- 开启深度的 pass 需要两个清除值（先颜色，后深度 0.0）；任何读取
  push-constant 矩阵的 Slang shader 都必须声明 `column_major`，才能与
  glam 的内存布局一致。
