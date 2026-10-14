# Agent Note: Shader request identity and artifact generations

Status: implemented

[English](2026-10-14-shader-request-identity.md)

## Problem

[架构契约加固提案](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
的第三项：`PreparedShaders::needs_prepare` 只比较 pipeline 名、asset id 和 asset 修订号。
在同一 asset 修订号下替换请求的入口点集合、反射入口或 capability 会静默复用旧产物；而
pipeline 以源码修订号决定是否重建，导致 shader 变更与请求变更在 pipeline 状态上都无法与
"无变化"区分。

## Decision

**请求携带指纹。** `PipelineShader::fingerprint()` 对反射入口和入口集合（名称与
capability）求哈希——即请求中所有会改变编译产物的部分。`PreparedShaders` 的槽位记录
编译时的指纹，`needs_prepare` 接收整个请求：asset 变更、源码修订号前进、请求变更都会重
新触发准备。

**pipeline 观察产物代际号，而不是源码修订号。** `PreparedShaders` 在每次编译成功时发放
单调递增的 `generation`，失败时不发放。`PreparedShader::generation()` 取代 `revision()`
成为所有消费方（core-3d 与 splat 各 pass、egui pipeline）的过期判断依据。因此一次失败的
重编译不可能扰动正在运行的 pipeline：没有新代际号发放，`get` 不再提供该 pipeline 的产
物，消费方继续运行用上一代产物构建的 pipeline。

提案在热重载名下提到的 `ShaderCacheKey` 不含 import 文件内容的问题保持开放——热重载尚
不存在；import 依赖失效随它一起落地。

## Alternatives considered

- **以 (修订号, 指纹) 对作为 pipeline 重建键。** 未采用：每个消费方都要存并比较两者，
  且失败编译的槽位会推进修订号（用于抑制逐帧重试），对任何不检查结果的消费方都读作"有
  新产物"。单个单调计数器把"出错时保留"规则折进了同一次比较。
- **把源码哈希进指纹。** 未采用：源码身份已由 asset 修订号承载；再哈希一次会把请求身份
  与 asset 存储已经在跟踪的内容耦合起来。

## Consequences

- 不改源码、只改 shader 请求会触发准备与 pipeline 重建；由
  `moonfield-render-feature::shader` 的测试覆盖。
- 一次改坏的 shader 不再触及任何 pipeline 状态：最后可用的 pipeline 一直运行到某次编译
  成功并发放新代际号。
- `AssetRevision` 已从所有 pipeline 身份判断中移除；它仍是 `needs_prepare` 和提取路径使
  用的 asset 存储概念。
