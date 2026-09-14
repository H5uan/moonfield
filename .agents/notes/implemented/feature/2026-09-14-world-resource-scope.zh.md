# Agent Note: World resource scope

Status: implemented

[English](2026-09-14-world-resource-scope.md)

## Problem

`AssetServer::load` 需要同时持有 `&mut AssetServer` 和 `&mut Assets<T>`，而世界的资源存储对每种资源只外借一次。每个调用方都在手写同一个变通——`remove_resource`、使用、`insert_resource`——分布在 `HandleTemplate::build`、编辑器的场景加载和 UI 的场景 Load 按钮；错误路径上任何提前返回只要漏掉放回，资源就无声地从世界中消失。

## Decision

- `World::resource_scope<R, U>(&mut self, f: impl FnOnce(&mut World, &mut R)
  -> U)` 把 `R` 临时移出世界的资源存储，以 (world, resource) 运行 `f`，然后放回。放回在 `f` 返回之后的每条路径上执行，因此提前 `return` 不可能让资源留在世界之外；在 `f` 内插入的新 `R` 会被替换。资源不存在时 panic；`try_resource_scope` 是 `Option` 形态。
- 被 scoped 的资源在 `f` 期间不在世界里，因此 `f` 可以一边持有 `&mut R` 一边通过 world 可变地使用其他资源——正是变通所要解决的双 `&mut` 形状。
- 三处调用方（`load_with_server`、`HandleTemplate::build`、场景 Load 按钮）各收敛为一层 scope；手写的取出/使用/放回及其错误路径注释全部删除。

## Alternatives considered

- **`AssetServer` 内部可变（排队加载、稍后插入）。** 参考实现的 `AssetServer::load` 只取 `&self`，因为加载是异步的、资产数据稍后才落入 `Assets<T>`，双借用根本不出现。moonfield 的加载按决策是同步的（[bsn-style scene templates](../architecture/2026-08-21-bsn-style-scene-templates.zh.md)），借用是固有的，scope 才是答案。
- **`ParamSet` 式的系统参数。** 那是解决系统签名内部冲突的工具；受影响的调用点都是持有 `&mut World` 的服务层代码，一层 scope 足够。

## Consequences

- 嵌套 scope 自然组合 N 个资源的访问。
- 编辑器 `EditorMainState` 的 slot 取出/放回是另一种形状（状态整帧阶段在外，而非双借用），保持显式块不变。
