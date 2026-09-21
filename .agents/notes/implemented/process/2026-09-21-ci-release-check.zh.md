# Agent Note: CI checks the workspace without debug_assertions

Status: implemented

[English](2026-09-21-ci-release-check.md)

## Problem

CI 中两个编译 Rust 的作业——clippy 与 test——都在 dev profile 下运行，
`debug_assertions` 处于开启状态。只在 `debug_assertions` 关闭时才能编译
的代码（例如 panic 消息读取 `#[cfg(debug_assertions)]` 门控的字段）会让
`cargo build --release` 失败，而门禁保持绿色；
[ComponentMeta type_name 修复](../bug-fix/2026-09-21-componentmeta-debug-only-field-accessor.md)
就是这样进来的。编译整个 workspace 需要 libclang
（shader-slang-rs-sys 的 bindgen）与其 build script 下载的 Slang 包；各
作业缓存该下载且不设置 `SLANG_DIR`
（[CI 链接绑定所钉的 Slang 版本](../bug-fix/2026-09-21-ci-slang-version-single-source.md)）。

## Decision

新增 `release-check` 作业，在两个受支持的 runner（ubuntu-latest、
windows-latest）上运行 `cargo check --release --workspace --all-targets
--locked`，并沿用 clippy 作业的 libclang 与 Slang 前置。用 `check` 而非
`build`：不做代码生成与链接，所有 target 都在 `debug_assertions =
false` 下完成类型检查，而不支付 release 编译的成本。

## Alternatives considered

- **在 clippy 作业内加一个 release-check 步骤。** 否决：省去了前置的
  重复，但把 lint 与 profile 覆盖耦合进同一个作业状态，且 clippy 的
  缓存会混入 release 产物。
- **`cargo test --release`。** 否决：每次 push 都做完整的 release 代码
  生成，却没有额外的 cfg 覆盖收益；check 已经对所有 target 做了类型
  检查。

## Consequences

- 仅 release 可见的编译破坏会让 CI 在两个受支持的 target 上失败。
- 该作业只检查、不链接也不运行，优化构建的运行期行为仍未被测试。
