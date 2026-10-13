# Agent Note: Recurring lookup and allocation costs in ECS resources, hooks, entities, and the fixed loop

Status: implemented

[English](2026-09-19-ecs-recurring-lookup-costs.md)

## Problem

一次对 `moonfield-ecs` 与 `moonfield-time` 的审计在每帧或每实体的路径上发现四项
反复开销：

1. 资源存储（`Resources`）与调度存储（`Schedules`）用默认 SipHash hasher 的
   `HashMap<TypeId, …>` 做键，于是每次 `Res<T>`/`ResMut<T>` 获取、每次调度运行都要
   对 `TypeId` 做一遍 SipHash。
2. `World::fire_hook` 在每个组件事件上做两次注册表查找（take，再 restore），即使
   根本没有注册任何 hook。
3. `Entities::contains` 对 `pending` 的保留尾段做线性扫描，`Entities::alloc_at` 对
   整个列表做线性扫描以便把 id 从空闲列表中换出。`alloc_at` 在每次 `Commands::spawn`
   应用时运行（保留 id 必不在空闲列表中，常见情形是一次完整 miss 扫描），
   `contains` 在关系（relationship）的 insert hook 中每次链接都会运行。
4. `run_fixed_main_schedule` 在每个定步上重新 insert 泛型 `Time` 资源，而
   `Resources::insert` 会新建 `RefCell::new(Box::new(..))`——每步一次堆分配，外加
   每次都在 LIFO drop 顺序表里添一条。

## Decision

- `Resources` 与 `Schedules` 改用 `TypeIdMap`——本 crate 已有的 `TypeId` 键恒等哈希
  映射（`TypeId` 本身唯一，hasher 直接转发 id 位而不再混淆）。
- `fire_hook` 在 hook 注册表为空时提前返回，没有 hook 的 world（或组件类型）不再付出
  两次查找。
- `Entities` 维护一个稠密侧索引 `pending_pos: Vec<u32>`，把实体 id 映射到它在
  `pending` 中的位置，不在则为 `u32::MAX`。reserve 只移动 `free_cursor`，从不重排
  `pending`，因此位置在 reserve/flush 周期中保持有效；所有增删 `pending` 条目的位置
  （`free`、`alloc`、`alloc_at`、`flush`、`set_freelist`、`clear`）同步更新该索引。
  `contains` 变为一次索引加载后与 `free_cursor` 比较，`alloc_at` 直接按索引
  swap-remove。
- 定步循环把每步的快照经泛型 `Time` 的 `RefCell` 借用写回现有资源
  （`*generic = snapshot`），而不是替换资源；仅当资源缺失时才 insert。循环结束后恢复
  到 virtual 时钟的写入同理。

## Alternatives considered

- **用 `HashMap<u32, u32>` 做 `pending` 的侧索引。** 否决：实体 id 本身就是 `meta`
  的稠密下标，`Vec` 无需任何哈希即可 O(1) 查找，代价是在 `meta` 每项 8 字节旁多花
  每 id 4 字节。
- **保持 `pending` 有序并二分查找。** 否决：`alloc`/`free` 把 `pending` 当栈用，
  id 复用顺序在发放出的 generation 上是可观察的；排序会改变该顺序。
- **把 hook 注册表本身也换成 `TypeIdMap`。** 未采用：注册表为空的快速路径已经消除
  了无 hook 情形的查找开销，而注册表只在有 hook 后才会被查询——SipHash 键的 map
  可以留给将来因其他理由改动 `world.rs` hook 存储的变更顺带处理。

## Consequences

- 资源获取与调度查找不再做 SipHash 计算；行为不变（资源的 LIFO drop 顺序由插入顺序
  列表而非 map 驱动，保持不变）。
- 未注册任何 hook 的 world 在每个组件事件上零开销；一旦有 hook 注册，查找仍走
  SipHash 键的注册表 map。
- `contains` 与 `alloc_at` 对 `pending` 规模为 O(1)；`alloc` 与 `free` 各多付一次
  数组写入。
- 稳态定步循环每步零分配。时钟的可观察契约不变：fixed 调度运行期间泛型 `Time` 镜像
  `Time<Fixed>`，之后恢复到 `Time<Virtual>`；泛型 `Time` 资源缺失时仍在第一步创建。
- 四条路径均由 `moonfield-ecs` 与 `moonfield-time` 的现有测试覆盖（实体 reserve/flush
  往返、hook 触发顺序、资源读写往返、带泛型时钟断言的定步步数测试）。
