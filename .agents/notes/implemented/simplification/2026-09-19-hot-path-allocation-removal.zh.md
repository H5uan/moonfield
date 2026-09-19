# Agent Note: Hot-path allocation removal in AssetServer and message updates

Status: implemented

[English](2026-09-19-hot-path-allocation-removal.md)

## Problem

两处按调用分配的分配点位于每帧或每次资产访问都会经过的路径上：

1. `AssetServer::load` 在探测缓存前用 `path.to_path_buf()` 构造缓存键
   `(TypeId, PathBuf)`，于是每次命中——也就是常见情形——都要克隆一次路径再
   丢弃。
2. `message_update_system` 每帧克隆整个 `MessageRegistry.updates` 函数指针
   向量，以便在对着 world 运行 update 函数之前释放 registry 的 resource
   借用。

## Decision

- 资产缓存改为嵌套 map：`HashMap<TypeId, HashMap<PathBuf, AssetId>>`，命中路径
  以借用的 `&Path` 探测（内层 map 的 `Borrow` 查找），只有在真正加载时才分配
  `PathBuf`。逻辑键仍是 `(TypeId, PathBuf)`：同一路径每种类型至多加载一次；缓存
  的 id 在 `Assets<T>` 存储中不再可解析时落到重新加载，其 insert 会替换掉过期
  条目。公开的 `load` 签名不变，调用方无需改动。
- `message_update_system` 按下标遍历 registry：每一步取 resource 借用、复制一个
  `fn(&mut World)`（`Copy` 指针），并在调用前释放借用。update 函数执行期间不持有
  对 `MessageRegistry` 的借用，因此 update 函数自身也可以访问 registry resource
  而不会发生借用冲突。registry 在整个运行期间始终留在 world 中。

## Alternatives considered

- **在扁平的 `(TypeId, PathBuf)` map 上用 raw-entry 或 `hashbrown` 查找。** 否决：
  该 crate 不依赖 `hashbrown`，而 std 的 raw-entry API 尚未稳定；嵌套 map 只用
  稳定版 std 就达到了同样的零分配命中路径，代价是每次查找多一跳 map。
- **把 `updates` 向量从 registry 中取出（或 `remove_resource` 掉整个 registry），
  运行后再放回。** 否决：对着 world 运行的 update 函数会在运行途中观察到空的或
  缺失的 `MessageRegistry`，而且交换多出一条必须恢复向量的失败路径。按下标遍历
  保持 registry 完整，每个函数都在短借用下读取。
- **在 registry 里存 `Rc<[fn(&mut World)]>` 快照。** 否决：resource 要求
  `Send + Sync`，这意味着要用 `Arc`，每帧一次引用计数自增而非克隆；按下标遍历
  两者都不需要。

## Consequences

- `AssetServer::load` 的缓存命中不再分配；只有真正的加载（或重新加载已移除的
  资产）才分配 `PathBuf` 键。
- `message_update_system` 每帧零分配；每个注册的消息类型每帧只付出一次 resource
  查找加一次函数指针复制。
- 在 update 运行期间注册的消息类型会在同一帧被纳入 buffer swap（注册目前只发生
  在 app 构建期，因此实际不可达）。
