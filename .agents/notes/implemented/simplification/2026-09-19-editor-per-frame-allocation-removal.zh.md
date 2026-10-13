# Agent Note: Editor per-frame allocation removal

Status: implemented

[English](2026-09-19-editor-per-frame-allocation-removal.md)

## Problem

编辑器每帧重复分配相同形状的内存：egui 上传为每个帧槽新建
顶点/索引/绘制 Vec，纹理增量逐像素调用 `Color32::to_array` 拷贝，层级面板为
每行存储一个拥有的 `String` 标签、并为悬停提示对每行各做一次 `format!`，
视口浮层把状态行构造成 `String`，检查器的 `prettify` 为每个字段分配一个
`String`，两个调试环境变量每帧通过 `std::env::var` 重读。在这些开销之外，
`prepare_egui_frame` 从 HashMap 最先产出的 `WindowSurfaces` 条目取 egui 管线的
颜色格式——只因为编辑器是单窗口才碰巧正确。

## Decision

- `egui_vk::FrameResources` 在现有 `mesh_draws` 之外持有持久的 CPU 暂存
  Vec（顶点、索引）；`update` 清空后复用，不再分配。
- 纹理上传用 `bytemuck::cast_slice` 把 `ColorImage.pixels` 直接转成
  `&[u8]`；编辑器的 `egui` 依赖启用 egui 的 `bytemuck` feature，使
  `Color32` 实现 `Pod`。零拷贝、无逐像素调用。
- `HierarchyEntry` 只存实体和深度；标签在绘制时解析（`entity_label` 借用
  `Name`，只有无名实体才格式化），悬停文本改用 `on_hover_ui`，只在悬停时
  构造；面板的行缓冲放在 `EditorMainState` 上，经 `collect_hierarchy_into`
  复用。
- 视口浮层的文本行全部是 `&'static str`：模式行按 `GizmoMode` 各有一条静态
  字符串，有选中项时选中提示行直接不出现。
- `EditorDebugEnv` 在 `EditorPlugin::build` 中一次性解析
  `MOONFIELD_EDITOR_AUTO_CLOSE` 和 `MOONFIELD_EDITOR_DUMP_VIEWPORT`，并插入
  两个世界；每帧的系统改读该资源。
- `prepare_egui_frame` 从 `WindowSurfaces::primary()`——即由 `PrimaryWindow`
  标记解析出的 surface——取交换链格式，不再取映射表的第一个条目。
- `registry::prettify` 写入由 `reflect_ui` 调用持有的暂存 `String`，在该层
  的所有字段间复用。

## Alternatives considered

- **跨帧池化每行的标签 `String`。** 否决：键控复用必须建立帧与帧之间的行
  身份映射；绘制时解析标签从根本上取消了存储问题，而有名字的实体——常见
  情况——直接借用。
- **缓存层级树、用变更检测触发重建。** 否决：只有每一次结构性修改都被追踪
  面板才保持正确；扁平重建足够简单，配合复用缓冲后开销已经很低。
- **`prettify` 返回 `Cow<str>` 并加借用快路径。** 否决：反射字段名都是
  snake_case，几乎每次调用都走 owned 分支，快路径是死代码；暂存缓冲让每个
  组件只复用一次分配。

## Consequences

- 稳态下编辑器每帧在 egui 上传、层级行、视口浮层和检查器字段标签上都不
  分配。
- 这两个环境变量是启动期配置：运行中修改不再生效。
- 多窗口时 egui 管线的格式确定性地跟随 `PrimaryWindow` 的 surface；没有任何
  surface 获取到图像的 tick 会跳过准备，而不是猜测。
- `HierarchyEntry` 移除了 `label` 字段；树的标签在测试中通过
  `entity_label` 验证。
