# Agent Note: pinned nightly toolchain via rust-toolchain.toml

Status: implemented

[English](2026-08-22-pinned-nightly-toolchain.md)

## Problem

CI 之前按 job 各自用 `dtolnay/rust-toolchain@stable` 选编译器,而本地开发
已统一到某个特定 nightly(rustc 1.100.0-nightly)。没有任何机制把 CI 和
本地构建钉在同一个编译器上;Dependabot 只管理 cargo 和 github-actions 两
个生态,没有覆盖 `rust-toolchain.toml` 的生态,工具链完全无人管理。

## Decision

仓库根目录新增 `rust-toolchain.toml`,钉住一个**带日期**的 nightly ——
`channel = "nightly-2026-08-22"`,并携带 `rustfmt`、`clippy`、
`rust-analyzer` 和 `rust-src` 组件。language server 使用与构建相同的编译器和
标准库源码，因此仓库内的 LSP 启动不依赖另行管理的全局安装。该日期
经 dist channel 存档
(`static.rust-lang.org/dist/<date>/channel-rust-nightly.toml`)核实,
恰好对应 rustc 1.100.0-nightly `c656540d6`。rustup 在本地自动识别该文
件;CI 的每个 Rust job 用一步裸 `rustup show` 安装它(rustup 读取文件、下载
带日期的工具链及其组件)。独立 CI job 执行 `rust-analyzer --version`，确认
工具链可提供该可执行文件；汇总 CI 结果也包含此 job。
`dtolnay/rust-toolchain` 曾首先尝试,但它**不
读** `rust-toolchain.toml`——那里 `toolchain` 是必填输入。工具链滚动由
`.github/workflows/nightly-bump.yml` 自动化:每周 cron 从 channel 清单解
出最新 nightly 日期、改写 channel,并用 `peter-evans/create-pull-request`
开 PR,让 CI 在合入前于全部平台验证新工具链。另外 `dependabot.yml` 新增
`egui*` 分组,未来 egui 栈的 bump 会合为单个 PR(见
[egui 0.36 迁移记录](2026-08-22-egui-stack-0-36.zh.md))。

## Alternatives considered

- **滚动 `channel = "nightly"`。** 否决:CI 结果会随当天 nightly 漂移,不
  可复现,回归无法归因。
- **保持按 job 的 `@stable`。** 否决:开发已统一到钉住的 nightly,本地/CI
  编译器分裂正是要修掉的问题。
- **交给 Dependabot 管理工具链。** 不可行:Dependabot 没有
  `rust-toolchain.toml` 对应的生态;bump workflow 补的就是这个缺口。

- **在钉住的工具链之外安装 rust-analyzer。** 否决：编辑器可能因此使用与仓库
  编译器不同的 language server 和标准库源码。在 `rust-toolchain.toml` 中声明
  两个组件，可让本地开发和 CI 共用同一个有版本约束的来源。

## Consequences

- 本地机器在下一次 `cargo` 调用时自动选用(并一次性下载)该日期的
  nightly;仓库内 `rustup default` 不再有影响。
- nightly 更新以 CI 门禁 PR 的形式到达;遇到坏 nightly 关掉对应 bump PR
  即可跳过。缺失 `rust-analyzer` 或 `rust-src` 构件也会使 PR 在合入前失败。
- 带日期的 dist 存档长期保留,钉住的日期始终可安装。
