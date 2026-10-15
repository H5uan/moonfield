# Agent Note: Main-world parking scope and thread contracts

Status: implemented

[English](2026-10-14-main-world-parking-and-thread-contracts.md)

## Problem

[契约加固提案](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
指出了安全调用方可以绕过的生命周期和线程契约。闭包可以移出资源或目标 world，
因此仅在作用域退出时移除 parked 资源无法约束指针。原子录制标志也不能同步 Vulkan
命令录制、command pool 或 queue 操作。

## Decision

**parking 使用会失效的共享 token。** `MainWorld` 持有一个 `Arc`，
其中存储原子 world 指针与 parking 线程身份。`World::with_parked_main_world`
在清理目标之前使 token 失效，包括展开退出路径。移出的资源或 world 只能保留已失效
的 token。在已有 parked 资源的目标内嵌套 parking 会被拒绝，不替换原来源。
访问在解引用前检查有效性与线程归属；unsafe 引用访问器保留生命周期和别名前置条件。

`App::render` 在继续传播提取 panic 之前归还取出的主 world。提取接口仍兼容
普通资源和查询参数；移动 token 不会授予其他线程访问其来源的权限。

**GPU 操作绑定设备创建线程。** `DeviceShared` 记录该线程，在提供核心或
扩展入口、分配器及 command pool 之前检查线程归属。录制、提交、呈现、上传批次、
descriptor 写入与 retirement 清理遵循相同归属。`CommandBuffer::end` 在修改
原子标志之前检查，因此被拒绝的跨线程调用不会结束属主的录制。`Send + Sync`
允许 ECS 存储，不代表支持并发驱动访问。不可变句柄值和受互斥锁保护的 retirement
入队操作仍可共享；GPU 工作和资源析构属于属主线程。

**线程局部资源共享现有存储和析构顺序。** `Resources` 在单一映射表及 LIFO
析构列表中跟踪插入线程。线程局部访问器拒绝其他线程和不匹配的普通访问器。
不引入新的系统参数类别。

## Alternatives considered

**只清理目标存储。** 未采用：通用资源移除和目标移动都能绕过清理守卫。
使共享 token 失效能够独立于存储位置约束访问。

**只使用原子录制状态。** 未采用：标志保护的是 Rust 字段，而不是 command buffer
录制、command pool 操作或 queue 提交。属主线程检查覆盖这些驱动调用。

**全部 GPU 对象禁止 Send，并在渲染中使用线程局部参数。** 暂缓：这需要迁移
render world 资源获取和所有录制调用方。设备级归属检查无需增加参数类别，
即可约束当前单线程执行模型。

**单独的线程局部资源映射表。** 未采用：独立析构过程会破坏相关资源共享的
LIFO 析构顺序。

## Consequences

- 正常返回或展开退出后，移出的 parked token 均不可用；安全提取也不能在其他
  线程解引用它们。
- 其他线程上的 GPU 录制、pool 访问和 queue 操作在进入驱动前失败；直接使用
  扩展入口的路径同样受归属检查保护。
- 提取保留主 world 的变更时钟，app 在传播 panic 前归还其 world。
- 混合资源存储保留现有析构顺序；线程局部调用方使用 world 访问器，而不是
  新的系统参数接口。
