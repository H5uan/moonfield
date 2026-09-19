# Agent Note: Explicit primary window resolution

Status: implemented

[English](2026-09-13-explicit-primary-window-resolution.md)

## Problem

窗口帧循环中有两处绕过了 `moonfield-window` 定义、`moonfield-winit` spawn
的 `PrimaryWindow` marker 组件：

- `extract_windows` 只查询 `(&Window, &RawHandleWrapper)`，在世界边界处丢弃了
  marker。
- `WindowSurfaces::primary()` 通过猜测解析 `PrimaryWindow` 逻辑渲染目标：
  取在飞表面中 main entity 位值最小者。这一猜测隐式依赖一条未声明的不变量
  ——窗口后端恰好先 spawn 主窗口——任何以其他顺序 spawn 窗口的后端都会把
  画面渲染到错误的窗口，且没有任何诊断信息。

## Decision

窗口身份改为端到端显式传递：

- `ExtractedWindow` 新增 `primary: bool` 字段；`extract_windows` 通过查询中
  的 `Option<&PrimaryWindow>` 填充它，与 `extract_cameras` 处理
  `PrimaryCamera` 的模式相同。
- `WindowSurfaceData` 保存 `primary` 标志，由 `create_window_surfaces` 每帧
  从抽取到的窗口刷新（持久化 map 条目自身无法读取每帧重建的组件）。
- `WindowSurfaces::primary()` 改为经由纯函数 `resolve_primary`，对在飞的
  `(entity, is_primary)` 候选做解析，并固定了边界行为：
  - 恰好一个带标记的候选：无论实体顺序如何，它胜出；
  - 没有带标记的候选（后端从不 spawn `PrimaryWindow`）：回退到历史上
    「最小实体」的猜测，使这类后端仍能正常渲染；
  - 多个带标记的候选（违反「恰好一个」约定）：`warn_once!` 警告，并确定
    性地取标记候选中实体最小者。
- `resolve_primary` 对全部四种情形（空、标记胜出、回退、违约）都有单元
  测试，实体由原始位值构造，无需 GPU。

## Alternatives considered

- **每次调用 `primary()` 时从 `ExtractedWindow` 组件重新解析。** `primary()`
  只拿 `&self`，无法访问 world；把组件传进去需要改签名和三处调用点，相比
  每帧刷新一次的标志位没有收益。
- **无标记候选时返回 `None`。** 更严格，但会把「后端未打标记」的情形从
  「可能渲染到错误窗口」变成「窗口完全没有输出」；回退保留了猜测仍是唯一
  机制时的既有行为。
- **多个标记窗口时 panic。** marker 约定确实是恰好一个，但渲染循环对内容侧
  的错误应当降级而不是中止；`warn_once!` 既暴露问题又不打断帧循环。

## Consequences

- 渲染目标 `RenderTarget::PrimaryWindow` 现在跟随 marker，而不是 spawn 顺序；
  未声明的不变量被移除。
- `ExtractedWindow` 是公共结构体且新增了公共字段；它仅有的构造点都在
  render-core 内部（`extract_windows` 与 `create_window_surfaces` 中的克隆），
  因此下游 crate 无需改动。
- `WindowSurfaces::primary()` 签名未变；editor 与 render-feature 的调用点
  原样编译。
