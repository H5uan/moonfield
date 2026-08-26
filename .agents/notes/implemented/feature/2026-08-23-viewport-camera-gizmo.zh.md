# Agent Note: Viewport orbit camera and transform gizmo

Status: implemented

[English](2026-08-23-viewport-camera-gizmo.md)

## Problem

编辑器 viewport 此前是只读纹理：相机姿态由场景里碰巧生成的 `PrimaryCamera`
实体写死，移动实体只能在 inspector 里拖数值字段。既无法在场景中导航，也无法
直接操纵物体，viewport 只是预览而不是编辑界面。

## Decision

viewport 面板成为编辑器的交互界面，完全建立在现有的单线程渲染缝上——不改渲染
器，不动 Vulkan。

- `moonfield-editor/src/interaction.rs` 承载全部交互数学，均为可无头单测的纯
  函数：`OrbitCamera`（pivot/yaw/pitch/distance，pitch 与距离有钳制）、处理
  reverse-Z 与 Y 翻转的 `world_to_screen` / `screen_to_ray` 换算、gizmo 命中
  测试（8 像素屏幕空间阈值），以及平移、旋转、缩放的 `GizmoDrag` 状态机。
- viewport 相机由编辑器接管：`OrbitCamera` 从 `PrimaryCamera` 实体的
  `Transform` 初始化一次，之后每帧写回（`lib.rs` 的 `apply_orbit_camera`）。
  右键拖拽环绕，中键拖拽平移，滚轮向 pivot 推拉。
- gizmo 是画在 viewport 图像之上的屏幕空间覆盖层，用 egui 的 `Painter` 绘制：
  平移是轴向箭头，旋转是圆环，缩放是轴手柄加中心的统一缩放手柄，用 W/E/R
  切换（以 `egui_wants_keyboard_input` 守卫，文本框保留按键）。手柄沿实体的
  局部轴；悬停或拖动中的手柄高亮为黄色。手柄几何是纯屏幕空间的：轴手柄的
  端点沿投影后的 2D 轴方向放在固定像素长度处，圆环半径按最不受透视缩短
  影响的基方向估算——尺寸计算不经过世界长度，否则透视缩短会让它随距离
  变化（此前手柄会随实体移动而伸缩）。
- 拖动数学在拖动开始时冻结轴方向和原点。若对实时的 `GizmoFrame` 施加拖动会
  形成反馈：平移时原点随实体移动，旋转时轴随实体转动，增量将对着移动中的
  参照系测量。轴向平移额外对着一个拖动平面进行（包含该轴、朝向拖动
  射线），平面法线在拖动开始时冻结。
- 拖动算出世界空间 TRS；`world_trs_to_local` 经父级 `GlobalTransform` 仿射的
  逆将其换算回实体的本地 `Transform`，使 gizmo 编辑与层级正确复合。传播系统
  随后在同一帧刷新 `GlobalTransform`。

## Alternatives considered

- **在 3D scene pass 里渲染 gizmo。** 否决：这需要 Vulkan 渲染器里的线段/覆盖
  层管线、深度处理和拾取支持——对编辑器外壳来说改动面太大。2D egui 覆盖层把
  全部 gizmo 代码收在一个模块里，是标准的先行实现。
- **引入第三方 gizmo crate（如 transform-gizmo-egui）。** 否决：它会带来一个
  需要跟随工作区锚定的 egui 版本的依赖，而所需数学（平移与旋转的拖动
  平面求交、缩放的屏幕距离比）规模很小，自研可以完全单测。
- **轴向平移用两线最近点。** 在短暂上线后否决：当视线射线与拖动轴接近
  平行（轴手柄正对着相机）时，最近点参数发散，实体在拖动的第一个像素
  就飞出屏幕。拖动平面——包含该轴、朝向拖动射线、法线在拖动开始时
  冻结——只在完全平行时退化，而那种姿态下手柄在屏幕上只是一个点，
  根本不会开始拖动。
- **给编辑器单独一个相机实体，而不是驱动 `PrimaryCamera`。** 否决：两个相机
  来源需要同步规则和渲染选择机制。接管主相机姿态只是每帧一次写入，保持单一
  事实来源；代价记录在下方。

## Consequences

- 编辑器运行期间接管 viewport 相机：在 inspector 里改主相机的 `Transform`
  会在下一帧被覆写。
- gizmo 只有局部模式（手柄跟随实体旋转）；世界/局部切换和 viewport 内的点选
  拾取明确不在本次范围。
- 指针下没有 gizmo 手柄时，viewport 左键点击无任何效果——该入口预留给后续的
  点选功能。
- `interaction.rs` 把引擎的 reverse-Z 与 Y 翻转约定收敛在两个换算函数里；其
  之上的全部 gizmo 数学都在 egui 左上角原点的屏幕空间中进行。
- gizmo 管线使用（translation, rotation, scale）顺序，而 glam 的
  `to_scale_rotation_translation` 返回（scale, rotation, translation）；
  顺序重排在 `ui.rs` 的分解边界处显式进行。这里曾直接解构导致两端互换：
  实体的平移被写进了缩放字段，零分量使下一帧的分解退化，旋转变成
  NaN/inf——即"gizmo 一拖就消失"的故障。
