# Agent Note: UE5-style editor layout

Status: implemented

[English](2026-08-23-ue5-style-layout.md)

## Problem

编辑器的初始 dock 布局是通用的三栏排列（左层级、右检查器、中 viewport），不
符合场景编辑器用户期待的形态：viewport 是主要工作面（现在承载了相机操控和
transform gizmo，见[viewport camera gizmo](2026-08-23-viewport-camera-gizmo.zh.md)），
却与侧面板均分窗口；资产/场景的文件操作挤在层级树顶部。

## Decision

初始布局改为 UE5 的编辑器外壳，用 `ui.rs::initial_dock_state` 里现有的
egui_dock split 构建：

- **Viewport** 占据中心最大面积。
- **Outliner**（层级，改名）在右上，**Details**（检查器，改名）在其下——约
  22% 宽的右列均分。
- **Content Browser**（新增 `Tab::Content`）占据 viewport 下方的底部条带
  （约 28% 高度），接管原先位于层级树顶部的资产加载和场景保存/加载行。
  层级面板现在只有实体树。

只有默认布局和标签页标题变化；用户运行时重排不受影响，面板逻辑没有在模块间
移动。

## Alternatives considered

- **保留通用三栏布局。** 否决：viewport 是编辑器的重心；让它占据最大面积、
  把实体编辑集中在右侧，符合引擎目标用户的心智模型。
- **现在就做真正的 content browser（目录列表、缩略图）。** 暂缓：它依赖原生
  文件对话框和资产枚举，两者都是已知欠账。底部面板今天承接现有的手输路径行，
  以后承接真正的浏览器。
- **把 `Tab` 枚举变体改成 UE5 术语（`Outliner`、`Details`）。** 否决：纯粹的
  标识符改名；只改显示的标题。

## Consequences

- 用户看到 UE5 面板名（Outliner / Details / Content Browser），代码中保留
  `Hierarchy` / `Inspector` 变体名。
- 资产与场景文件操作从层级面板移到 Content Browser；层级面板去掉了顶部的
  操作行，只显示实体树。
- 该布局只是初始状态——egui_dock 仍允许用户在运行时随意重排。
