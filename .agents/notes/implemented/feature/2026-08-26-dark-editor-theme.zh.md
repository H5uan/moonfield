# Agent Note: dark editor theme

Status: implemented

[English](2026-08-26-dark-editor-theme.md)

## Problem

编辑器此前用 egui 的默认深色主题渲染：接近纯黑的配色、默认间距、6px 控件圆角、白色文字。所有面板、tab 条和控件看起来都像默认的 egui 示例。编辑器需要一套连贯的深色调色板——深蓝灰表面（`WINDOW_BG` `#1F1F24`、`PANEL_BG` `#2A2A2E`、`INPUT_BG` `#36373B`）、强调蓝（`ACCENT_BLUE` `#206EC8`）、语义化状态色、紧凑间距和 2/4px 圆角。

布局保持 UE 风格（见 [UE5 风格编辑器布局](2026-08-23-ue5-style-layout.zh.md)）；本次范围仅限颜色、密度和 chrome。

## Decision

新增 `theme.rs` 作为调色板和组合的唯一所有者。它提供：

- `theme::install(&egui::Context)` — 在 `EditorMainState::new` 中调用 `set_style_of(Theme::Dark, …)`，整个编辑器经 egui-dock 的 `Style::from_egui` 桥自动继承（tab 条、tab 内容区和覆盖层颜色自动从 `extreme_bg_color` / `window_fill` 派生）。
- `theme::visuals()` / `style()` — 调色板和紧凑间距（`item_spacing` 6×4、`button_padding` 6×3、2px 控件圆角、4px 窗口/菜单边距）。
- `theme::status_color(&str)` — Content 面板的 Load/Save 结果消息按结果着色：消息不含 `failed` 时用 `TEXT_SUCCESS` 绿，含 `failed` 时用 `TEXT_ERROR` 红（原先是不分语义的灰色 `ui.small`）。

Viewport 覆盖层提示文字从 `Color32::from_white_alpha(160)` 改为 `theme::TEXT_SECONDARY`，跟随主题。

## Alternatives considered

- **在 `ui.rs::show` 里手工构造 egui_dock `Style`。** 已拒绝：未显式设置样式时 `DockArea` 已通过 `Style::from_egui` 从 egui 样式派生，逐面板设样式会把调色板复制到第二个家。`theme.rs` 保持唯一所有者，映射交给 egui-dock 的派生逻辑。
- **带字体和图标的"专业主题"。** 本次已拒绝：字体明确不在范围内，图标属于工具栏 chrome 工作，不属于调色板。

## Consequences

- 编辑器仍使用 egui 内置字体；文字大小不变。
- Gizmo 轴色保持行业标准红绿蓝——主题不重绘 gizmo 管线。
- 调色板是一组常量，不可用户配置；设置界面属于独立功能。