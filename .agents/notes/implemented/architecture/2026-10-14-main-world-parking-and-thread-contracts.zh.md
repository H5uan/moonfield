# Agent Note: Main-world parking scope and thread contracts

Status: implemented

[English](2026-10-14-main-world-parking-and-thread-contracts.md)

## Problem

[架构契约加固提案](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
的第一项覆盖三处健全性缺口：`World::park_main_world` 通过公开的安全方法存储借用来的
world 地址，调用方可以让 `MainWorld` 资源比其来源活得更久（而 `App::render` 在 panic
中途 drop 取出的主 world，恰好会在 render world 里留下这样的悬垂指针）；RHI 的
`CommandBuffer` 把 `Cell<bool>` 录制标志与手写的 `Send + Sync` 实现搭配使用，两个共享
引用并发调用 `end` 就是真实的数据竞争；`Resource` 的 `Send + Sync + 'static` 约束则让
无法健全共享的 GPU 对象在 `World` 中无处安放。

## Decision

**parking 是作用域，而不是一对调用。** `World::with_parked_main_world` 是唯一的 parking
入口：它插入 `MainWorld` 资源、运行闭包，并由一个 drop guard 在包括 unwind 在内的所有
退出路径上移除该资源，因此 `MainWorld` 内部的裸指针永远不会比借用来源 world 的调用活
得更久。`park_main_world`/`unpark_main_world` 是私有的。`App::render` 用
`catch_unwind`/`resume_unwind` 包裹提取：panic 仍然会传播，但只在作用域完成 unpark、
取出的主 world 归还给 app 之后——panic 的提取器留下的是可恢复的 app。
`MainWorld`::`world`/`world_mut` 保留其 unsafe 前置条件——它们正是提案要求暴露的裸指针
入口。

**`CommandBuffer` 自动实现 `Send + Sync`；共享状态修改是原子的。** `ended` 标志是
`AtomicBool`：`end` 先用 swap 认领结束操作再调用 `vkEndCommandBuffer`，因此并发的重复
end 是空操作，永远不构成竞争。`Cell` 移除后，所有字段都是 `Send + Sync`，手写实现随之
删除。结构体文档承载录制契约：录制方法取 `&self`，因为帧循环把共享引用交给单线程调度
运行的 render 系统；跨线程共享*录制中*的 buffer 违反 Vulkan 的宿主同步要求，属于调用方
违约。

**线程局部资源是带线程钉扎的存储，不是新的 world。** `Resources` 仍是一张映射表加一条
LIFO drop 顺序；`thread_pins` 表标记通过 `World::insert_thread_local_resource` 插入的
类型（任何 `'static` 类型）并记录插入线程。线程局部访问器在非属主线程访问时 panic，在
普通/线程局部访问器混用时同样 panic。钉扎是面向未来的——`World` 目前既非 `Send` 也非
`Sync`——它把未来的跨线程访问变成 panic 而不是数据竞争。

其余手写实现经审计后保留：`Reflection`（对自有 Slang 对象的只读共享查询）、
`ShaderCache`（所有会话访问都在其互斥锁内）、`HostPtr`（单写者分配契约）、
`RawHandleWrapper`（句柄仅用于 surface 创建）都在实现处携带各自的论证。

## Alternatives considered

- **把 parking 标为 `unsafe` 而非作用域化。** 未采用：作用域直接消除了失效模式
  （parked 指针比其来源活得久），而不是要求每个调用方避免它，并让 `App::render` 不含
  unsafe。
- **`CommandBuffer` 的排他（`&mut self`）录制方法。** 暂缓：录制经由 `RenderContext`
  交给 render 系统的共享引用进行，排他化会波及 render-core、render-feature、编辑器和
  训练器，换来的保证单线程调度已经提供。改为记录审计后的契约；原子标志消除了唯一真实
  的竞争。
- **为线程局部资源单建一张映射表。** 未采用：第二个存储会有自己的 drop 过程，当线程
  局部 GPU 对象依赖更早创建的普通资源时会破坏 LIFO 保证。
- **线程局部系统参数。** 暂缓：还没有消费者在系统中取用线程局部资源；在没有消费者的
  情况下添加参数支持只是未经使用的框架表面。

## Consequences

- 安全的提取调用不再可能读到已销毁的主 world：资源不会比
  `with_parked_main_world` 活得更久，`App::render` 在 panic 恢复传播前归还主 world
  （由 `moonfield-ecs` 和 `moonfield-app` 的测试覆盖）。
- 混合存储下 ECS 资源的 drop 顺序不变；线程局部资源先于它所依赖的、更早创建的资源
  drop，与普通资源一致。
- `CommandBuffer` 的 `Send + Sync` 现在是由编译器检查的字段属性；某个字段失去这两个
  trait 时，在 owner 把 buffer 存入资源的位置直接编译失败，而不是削弱一条手写承诺。
- 线程局部资源没有系统参数访问；需要它们的系统直接取 `&mut World`（或使用 `World`
  访问器）。
