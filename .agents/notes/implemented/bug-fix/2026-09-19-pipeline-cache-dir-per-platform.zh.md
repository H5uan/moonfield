# Agent Note: Per-platform pipeline cache directory

Status: implemented

[English](2026-09-19-pipeline-cache-dir-per-platform.md)

## Problem

`pipeline_cache_path`（crates/moonfield-rhi/src/vulkan/device.rs）原来从
`XDG_CACHE_HOME` 解析 Vulkan 管线缓存的位置，回退到 `HOME/.cache`，再往后是
`unwrap_or_default()`。Windows 上这两个变量通常都未设置，于是根目录为空，路径
`moonfield/pipeline_cache.bin` 变成相对进程工作目录：缓存落在编辑器恰好启动的目录
（仓库根目录就积攒了一个这样的文件），多次运行之间从不共享，还会污染进程运行过的
任何目录。

## Decision

缓存根目录按平台解析，只用 `std::env`（工作区并未依赖 `dirs`）：

- Windows：`%LOCALAPPDATA%`，回退到 `%APPDATA%`。
- 其他平台：`$XDG_CACHE_HOME`，回退到 `~/.cache`——行为不变。
- 两个平台共同：没有可用的缓存根时回退到临时目录（`std::env::temp_dir`），它总是
  绝对路径，因此路径不会再悄悄变成相对工作目录。

最终结果总是 `<root>/moonfield/pipeline_cache.bin`。路径之上的所有失败模式本来就
是非致命的，并保持如此：文件缺失或损坏就以空缓存启动，驱动拒绝的旧数据回退为冷
缓存，`Device::drop` 里写回失败只记警告——缓存未命中永远不会让设备创建失败。

## Alternatives considered

- **依赖 `dirs` crate。** 否决：它今天不在依赖图里，而工作区支持的两个平台
  （Windows 与 Linux）只需要上面这几个环境变量查询；为三次 `std::env::var_os`
  调用引入一个依赖是多余的供应链边。
- **回退锚定到可执行文件或资源目录。** 否决：安装后的二进制目录常常只读，且"可
  执行文件旁边"把按用户、按驱动划分的缓存耦合到安装布局；临时目录是唯一保证可
  写且绝对的位置。
- **没有根目录可解析时禁用缓存。** 否决：静默丢弃缓存会让每次运行都付出管线编
  译时间，而这个配置错误已被临时目录回退覆盖。

## Consequences

- Windows 上缓存位于 `%LOCALAPPDATA%\moonfield\pipeline_cache.bin`，跨多次运行和
  不同工作目录共享；驱动级管线复用现在在 Windows 上生效。
- 早期运行写在各处工作目录下的缓存文件成为孤儿；代码不做清理（每个只有几 KB，
  且其归属目录一般无法得知）。
- 临时目录回退意味着在配置极简的系统上缓存读写可能落在临时目录；其内容按构造
  本来就是可丢弃的。
