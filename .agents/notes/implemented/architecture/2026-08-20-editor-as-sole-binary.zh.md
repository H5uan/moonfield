# Agent Note: The editor is the workspace's only binary

Status: implemented

[English](2026-08-20-editor-as-sole-binary.md)

## Problem

两个可运行目标争夺入口角色:`cargo run` 构建的是 `moonfield` binary crate——一个只打印 FPS、从不加载编辑器的 demo——而产品入口却放在 `crates/moonfield-editor/examples/editor.rs`。以项目命名的 crate 不是产品,编辑器只能通过 example 目标启动。

## Decision

`moonfield-editor` 是 workspace 唯一的 binary crate:原 `examples/editor.rs` 移到 `src/main.rs`(binary 取 package 名),`moonfield` crate 删除——它的 demo main 覆盖不了任何编辑器 binary 和测试套件未覆盖的东西。根 `Cargo.toml` 设置 `default-members = ["crates/moonfield-editor"]`,裸 `cargo run` 直接启动编辑器。

## Alternatives considered

- **保留 `moonfield` 作为只组合 `EditorPlugin` 的薄 binary。** 拒绝:一个全部内容就是一段插件组合 main 的 crate 是没有主人的间接层;binary 属于编辑器 crate。
- **把 demo main 挪到其他 crate 的 `examples/`。** 拒绝:它只用到 `LogPlugin` + `TimePlugin` + `print_fps`,编辑器 binary 已全部覆盖;保留它只是多出一个无人维护的可运行入口。

## Consequences

- 在 workspace 根目录 `cargo run` 构建并启动编辑器;不存在非编辑器的 binary 目标。
- 编辑器 binary 持有启动 demo 场景(相机 + 父子方块),同时作为 `MOONFIELD_EDITOR_AUTO_CLOSE` 冒烟测试的载体。
- `EditorPlugin` 仍是纯 plugin——binary 只负责组合插件,把编辑器嵌入其他 app 依然可行。
- 被删除的 crate 是只有 binary 的叶子:workspace 内没有任何依赖,也不暴露库接口。
