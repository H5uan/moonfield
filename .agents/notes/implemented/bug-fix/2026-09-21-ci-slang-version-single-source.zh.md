# Agent Note: CI links the Slang version the bindings pin

Status: implemented

[English](2026-09-21-ci-slang-version-single-source.md)

## Problem

`test (windows-latest)` 自 2026-09-08 起间歇性失败：`moonfield-render-feature`
的 lib 测试二进制——workspace 运行中第一个创建 Slang `GlobalSession` 的二进制
（其 gpu_util 测试在 CPU 上编译 shader；moonfield-ml 的测试需要 Vulkan
device，在 runner 上跳过）——在全部测试通过后以 STATUS_ACCESS_VIOLATION
（0xc0000005）退出。在 windows runner 上直接运行该二进制 20 次：用 CI 提供
的 Slang 运行时崩溃 12 次；用绑定所钉的运行时一次未崩。

CI 的 `setup-slang` action 导出的 `SLANG_DIR` 指向 Slang 2026.12，而
`shader-slang-rs-sys` 的 build script 会优先采用 `SLANG_DIR`，弃用自己锁定
的下载 Slang 2026.16.1——即该 crate 手写 vtable 所钉定的版本。Slang 全局
状态的进程退出清理是上游 teardown bug（shader-slang-rs 的 CI 记录为
Windows 上 0xc0000005、macOS 上 SIGBUS；它自己的测试套件用永不 drop 的
缓存会话加 `--test-threads=1` 绕开该清理路径）。2026.12 运行时表现出该
bug；2026.16.1 没有。gpu_util 测试并行地创建并 drop 每测试一个
`GlobalSession`，正好穿过这条清理路径。

## Decision

CI 不再在任何地方设置 `SLANG_DIR`。sys build script 的 `SLANG_VERSION` 是
唯一的 Slang 版本源，它把预编译包下载到 cargo git checkout 里；编译作业
（clippy、release-check、test）缓存该下载，键为 `Cargo.lock`——lockfile 钉
住 checkout revision，后者钉住 `SLANG_VERSION`。`setup-slang` action 删除。

## Alternatives considered

- **把 action 的默认版本号升到 2026.16.1。** 否决：版本会住在两处且无机械
  关联——正是这次上线的漂移方式。
- **照搬 shader-slang-rs 的 `--test-threads=1`。** 否决：让整个 workspace
  套件串行化来掩盖上游 teardown bug；移除过期运行时才是移除触发条件。
- **让 `moonfield-rhi` 对齐 fork 的永不 drop 共享会话。** 暂缓：每测试
  创建并 drop 的模式在 2026.16.1 上 20/20 干净；若 Slang 回归再议。

## Consequences

- CI 不可能再链接到绑定未钉定的 Slang 运行时；跨两个仓库的版本漂移在结构
  上不可能发生。
- 每个编译作业在缓存未命中时下载 Slang 发布包（约 100 MB）。
- 对 flaky 的退出期 AV 本身不存在廉价的 CI 回归缝隙——每次 push 跑 20 轮
  windows 循环代价过高。单一版本源即防线；Slang teardown 若回归会以 CI
  失败的形式重现。
