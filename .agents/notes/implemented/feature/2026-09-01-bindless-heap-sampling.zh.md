# Agent Note: bindless heap sampling (no pipeline descriptor layout)

Status: implemented

[English](2026-09-01-bindless-heap-sampling.md)

## Problem

描述符堆（此前提交）能装纹理描述符并绑定到命令缓冲，但没有管线在 GPU 上采样它。"shader bindings" 通道是否要求管线带描述符集布局（每堆形状一个），还是堆本身就能喂给着色器——这是悬而未决的问题。Slang 的 capability 路径（`ResourceDescriptorHeap` / `spvDescriptorHeapEXT`）看起来是免布局路线，但从未在真实驱动上验证过——此前的工具链验证跑在这台机器的驱动拿到 v2 实现之前。

## Decision

以实证为准并落地免布局路线：

- shader.rs：`Compiler` 增 capability 编译（`compile_*_with_capabilities`）；采样 kernel 传 `spvDescriptorHeapEXT`。编译器产出 `OpCapability UntypedPointersKHR` 且**无 DescriptorSet/Binding 装饰**——真正的描述符堆路径，管线无需 set layout、永不 bind descriptor sets。
- `DescriptorHeap::cmd_bind_graphics` 改名 `cmd_bind`：堆绑定是命令缓冲级、与绑点无关（一次调用服务图形与计算）。
- device.rs 要求 `VK_KHR_shader_untyped_pointers` 及其 feature；upload.rs 把图像释放到 `ALL_COMMANDS`（compute 也采样它）。
- 端到端测试 `descriptor_heap_sampling`：bindless 4x4 红纹理 → `cmd_bind` → compute kernel 采样 `ResourceDescriptorHeap[0]` / `SamplerDescriptorHeap[0]` → 读回断言纯红，跑在真实驱动上。

### 开 validation 时抓到的 bug

- device.rs 用 `let _ = features2.push(…)` 链式挂 `PhysicalDeviceFeatures2`——`push` 消费 `self`，整条 feature 链（bufferDeviceAddress、descriptorHeap、timeline……）**被丢弃**；设备从未请求这些 feature，驱动静默容忍。现改为绑定返回值。
- `DeviceCreateInfo` 用 `push(&mut features2)`；features2 带头部链后违反 push 的"无链 next"断言——改用 `extend` 合并。
- 注：本机 Khronos validation 层（SDK 1.4.335）比新结构旧（报 sType 1000135008 未知并崩溃），属于工具链版本差，非代码缺陷。无 validation 时全部 GPU 测试通过。

## Alternatives considered

- 要求描述符集布局（"shader bindings" 通道）：一旦 Slang capability 路径降级为 untyped 堆访问，经验证无必要。
- 让 `cmd_bind_graphics` 用于 compute：在规范"与绑点无关的绑定"语义下会误导。

## Consequences

- bindless 2.0 在真实硬件上端到端闭环：堆写入 → 绑定 → untyped shader 访问 → 采样读回，无描述符集布局、无 `vkCmdBindDescriptorSets`、根签名只是 BDA 指针。
- 运行时描述符堆 + BDA 指针配对与 no-graphics-API 蓝图完全一致。
- `VK_KHR_shader_untyped_pointers` 加入必需集；CI 机器缺失时照旧跳过 GPU 套件（它们本就过不了 descriptor_heap 要求）。
