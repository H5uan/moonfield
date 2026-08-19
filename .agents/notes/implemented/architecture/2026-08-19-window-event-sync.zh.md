# Agent Note: Window event synchronization

Status: implemented

[English](2026-08-19-window-event-sync.md)

## Problem

窗口状态在两个方向上需要同步:winit 上报 resize/DPI/focus 变化,游戏/编辑器代码修改标题、光标模式等窗口属性。若无统一模型,每个客户端都要维护一份"窗口长什么样"的拷贝,组件与原生窗口之间会累积漂移,请求队列也会与其驱动的组件状态竞争。

## Decision

窗口是 ECS 实体,`moonfield-window` 中的 `Window` 组件是逻辑窗口状态的唯一事实来源。

- **winit→ECS 即时写入**:winit 的 `window_event` 当场把 resize/DPI/focus 写回 `Window` 组件。
- **ECS→winit 每帧差分一次**:`App::update` 之后,`sync_windows`(`windows.rs`)把实时 `Window` 字段与 `CachedWindow` 组件做 diff,`diff_window` 返回 `WindowDiff`(标题、光标模式),由后端应用到原生窗口。这是 `CachedWindow` diff 模式(无变更检测)。
- `WinitWindows`(resource)维护 `Entity ↔ WindowId` 映射;主窗口实体在 `resumed` 中生成,若存在预创建的 `Window` 实体会收养它。
- **没有 `WindowRequests` 通道** —— 直接改组件。
- 生命周期事件(`close_requested`/`resized`/`focus_*`/`scale_factor_changed`)走独立通道 `WindowEvents` world resource,因此即使消费方不逐帧轮询也不会漏事件。
- 退出策略镜像 `auto_accept_quit` 约定:`CloseRequested` 默认退出;`WindowControl::set_auto_exit_on_close(false)` 接管控制,之后 `WindowControl::request_exit()` 退出。

## Alternatives considered

- **`WindowRequests` 通道(变更队列)。** 拒绝:这会产生能与组件分化的第二个事实来源,引入提交点,且容易被无意绕过。改组件 + 每帧一次 diff 保持单一事实,每帧成本有界。
- **每帧全量推送所有字段。** 拒绝:重复应用未变化的值(光标模式、标题)每帧产生 winit 调用,也让测试看不到 diff;逐字段 diff 保持表面有意。
- **用变更检测跟踪变化。** 拒绝:组件与编辑器共享并被多个系统读取;`CachedWindow` diff 比把 `Window` 变更耦合进 ECS change-tick 机制更简单,diff 本身就是可测试单元。

## Consequences

- 任何改变窗口状态的人都必须修改 `Window` 组件——没有别的门,错误用法一眼可见。
- diff 每帧运行一次,因此突发快速变更会塌缩为最终值:这是有意为之,但测试光标/标题突发行为时要记住。
- `WindowEvents` 必须由消费方(编辑器或 app)每帧排空,否则条目会积压。