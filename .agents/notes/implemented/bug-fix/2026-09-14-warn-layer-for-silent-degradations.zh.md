# Agent Note: Warn layer for silent degradations

Status: implemented

[English](2026-09-14-warn-layer-for-silent-degradations.md)

## Problem

框架内部的可恢复失败全是静默的：插入指向已死实体的关系被无声丢弃；schedule 约束引用没有注册系统的 label 时不可见地退回注册序（拼错或系统改名）；mesh 无 source path 的 `MeshRenderer` 让整个实体跳过场景保存；引用缺失资产的 mesh renderer 在渲染、UI、日志三个面同时消失。编辑器的 Load/Save 状态把判定结果拍平成文本，再用 `contains("failed")` 嗅探反推。

## Decision

- 框架内部的可恢复失败走 log 层告警——moonfield-ecs 直用 `tracing`（框架层之下，见 [log 分层边界](../architecture/2026-09-05-log-crate-layering-boundary.zh.md)），其上走 moonfield-log 的再导出。点位：被丢弃的关系、schedule 解析中被忽略的 label（每次重建告警一次，而非每 run）、无法落盘的 `MeshRenderer`（保存钩子处 `warn_once`）、引用缺失资产的 mesh renderer（extract 根因处带计数 `warn_once`——draw 侧的跳过是同一根因的下游，保持静默）。
- 编辑器 Load/Save 状态结构化携带判定：`theme::Status`（Success/Failure）取代纯文本 message，`status_color` 的文本嗅探删除。错误文本保持文本——判定已结构化后，`load_with_server` 的 `AssetError` → `String` 只是展示用。

## Alternatives considered

- **Result 系统参数（参考实现 0.20 形态：`IntoResult` + `BevyError`/`Severity` + `ErrorContext` + fallback error handler）。** 当前零消费者——所有可失败路径（资产加载、场景存取）都是服务层代码，没有系统返回 `Result`。推迟到第一个 fallible 系统出现（ML 训练步、资产加载系统化、GPU 错误上报），届时按完整形态一次做全，不零碎地做。
- **world 级诊断资源收集告警。** 与所有 crate 已共享的 log 层重复。
- **载入侧字段重置告警（mesh 颜色、camera order）。** 那是系统性的、非异常的：字段保真属于场景条目粒度问题，不是告警。

## Consequences

- moonfield-ecs 新增直连 `tracing` 依赖，与其他框架层之下的 leaf 一致。
- `warn_once` 让每帧点位（保存钩子探测、mesh 提取）不刷屏；schedule 告警只在约束注册变化时触发。
- 上述每个静默降级现在都只差一行日志；降级行为本身没有任何改变。
