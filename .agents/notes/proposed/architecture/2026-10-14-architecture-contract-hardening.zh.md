# Agent Note: Architecture contract hardening

Status: proposed

[English](2026-10-14-architecture-contract-hardening.md)

## Problem

RHI、render-core、render-feature 和 editor 的划分提供了有效的所有权和依赖分离。
[Vulkan 枢纽](../../implemented/architecture/2026-10-13-rhi-vulkan-pivot.zh.md)、
[渲染记录上下文](../../implemented/architecture/2026-09-09-render-recording-context.zh.md)和
[共享设备生命周期](../../implemented/architecture/2026-09-13-shared-device-context-lifecycle.zh.md)
仍是基础。改进目标是调用者在这些模块之外必须维持的契约，而不是再次重写整个工作区。

静态检查发现六个方面需要处理：

- **生命周期和线程安全。** `World::park_main_world` 通过公开的安全方法保存借入
  World 的地址，却没有保留其生命周期。`Extract::fetch` 在内部解引用该地址；
  `App::render` 提供了预期作用域，但独立调用者可以在源 World 销毁后保留该资源。
  `Resource` 要求 `Send + Sync`，而 `CommandBuffer` 包含 `Cell<bool>`，记录操作
  依赖外部同步，却手动实现了这两个 trait。开展并发工作前需要审计这些契约的健全性。
- **场景失败一致性。** `ResolvedScene::apply` 在可能失败的模板构建完成前创建
  实体。`load_scene` 逐个解析并应用根节点。错误可能使本次失败操作创建的实体留在
  目标 World 中。
- **Shader 编译身份。** `PreparedShaders::needs_prepare` 只比较 pipeline 名、
  资产 id 和资产 revision。同一资产 revision 上的请求即使替换了入口、反射入口
  或 capabilities，也不会触发准备。`ShaderCacheKey` 还缺少导入文件的内容身份；
  实现可靠热重载前需要补齐依赖失效机制。
- **插件初始化职责。** `EditorPlugin` 通过
  `scene_io::load_pipeline_shaders` 加载 mesh、splat 和 egui 的 Shader 请求。
  `RenderFeaturePlugin` 插入默认的空存储和请求，可能覆盖调用者提供的资源。
  不使用编辑器的渲染应用需要自行复现功能初始化逻辑。
- **Gaussian 领域所有权。** trainer 从 `moonfield-render-feature` 导入
  `GaussianScene`，使 CPU 领域数据的消费者依赖框架层。多个公开的并行 Vec
  允许长度不一致；`pack_gaussians` 静默截断到必需属性数组中的最短长度。
- **编辑器操作职责。** UI 回调直接对 `World` 执行资产加载、场景 I/O 和 Transform
  编辑。撤销、后台导入和自动化可能因此需要分别实现同一种操作。

代码依据位于 `crates/` 下的 `moonfield-ecs/src/{world,resource}.rs`、
`moonfield-rhi/src/{backend/vulkan/command,shader/compile}.rs`、
`moonfield-render-core/src/extract.rs`、`moonfield-scene/src/{resolved,file}.rs`、
`moonfield-render-feature/src/{shader,plugin,splat/scene,splat/prepare}.rs` 和
`moonfield-editor/src/{lib,scene_io,ui}.rs`。
这些结论来自静态检查，不代表动态复现或测得的性能回退。

## Proposal

按以下顺序分别实施：

1. **P0 — 生命周期和线程契约。** 为不能健全满足 `Send + Sync` 的对象增加线程
   局部资源存储。审计手动线程 trait 实现，而不是一律移除。将 main-world parking
   收进受控的 extraction 作用域，统一安装、执行和移除资源，并覆盖栈展开。
   保留的裸指针入口必须显式暴露 unsafe 前提。共享访问无法安全同步时，优先使用
   独占记录接口。
2. **P1 — 场景提交语义。** 创建实体前解析并校验全部根节点。将资产解析和加载计划
   准备与 World 修改分开。记录本次操作拥有的实体并在失败时清理，明确成功加载的
   资产是否允许留在缓存。更强的原子提交需要限制模板的任意副作用，仅销毁根实体
   并不足够。
3. **P1 — Shader 身份。** 为编译请求增加 fingerprint，覆盖入口集合、反射入口、
   capabilities 和支持的编译选项。将源 revision 与准备产物 generation 分开，
   pipeline 根据后者重建。失败时保留上一份可用 pipeline。引入热重载时增加
   import 依赖失效机制。
4. **P1 — 功能初始化。** 各渲染功能使用应用提供的资产配置声明并注册自身 Shader
   需求，编辑器仅拥有 egui 请求。初始化缺失资源时保留显式配置，并在组合完成后
   校验必需插件依赖和调度锚点。
5. **P2 — 共享 Gaussian 领域。** 将 CPU Gaussian 数据和约定提取到 renderer 与
   trainer 共享的模块，使其不依赖 app/render-core。根据最终依赖图决定 crate
   封装。在构造和更新接口校验属性等长、数值及表示约定；packing 返回错误，而不是
   静默丢弃记录。[Splat forward 决策](../../implemented/architecture/2026-10-14-splat-forward-pass.zh.md)
   负责说明当前颜色和 kernel 约定。
6. **P2 — 编辑器操作。** UI、快捷键和自动化通过同一个 `EditorAction` 执行接口
   导入资产、加载场景和编辑 Transform。执行模块拥有校验、结果和撤销记录，UI
   负责展示。添加后台任务前先建立该接口，渲染帧交接仍独立于操作队列。

在 GPU 测量证明缩小同步范围有收益前，保留保守的记录 barrier。普通 CI 继续容忍
不受支持的硬件，在合格 runner 上增加严格 GPU 契约模式：设备初始化失败必须使
该任务失败，不能通过返回来将跳过测试计为通过。现有公开契约测试是起点，不要求
新增 mock RHI。

## Alternatives considered

**围绕另一个 backend、完整 render graph 或并行 ECS 重写工作区。** 这些方案不
解决已识别的生命周期、失败处理和初始化契约，却会在现有接口变得安全且可独立测试
之前增加迁移工作。

**仅拆分大型源文件，不改变接口。** 文件组织可以改善导航，但 UI 直接修改 World、
功能启动知识和调用者同步责任仍然存在。应按上述操作和所有权接口拆分，而不是只按
行数拆分。

**先引入异步加载。** 同步加载是有意的简化。在定义操作结果、场景提交语义和线程
局部资源前将其移到工作线程，会使现有契约更难执行。应先建立这些接口。

## Acceptance criteria

- 安全的 extraction 调用不能读取已经销毁的源 World。extraction 栈展开后不留下
  parked 指针，应用的 main world 仍可恢复。
- 线程局部 GPU 资源不需要不成立的 `Send + Sync` 承诺；记录访问和 pool 同步具有
  明确且经过审计的契约。
- 后续根节点非法、资产缺失和子模板失败均不留下本次操作拥有的实体。测试明确
  资产缓存保留规则及允许的非实体副作用。
- 修改 Shader 请求而不修改源码也会触发准备和 pipeline 重建。准备失败时保留可用
  pipeline。热重载实现后，仅修改 import 文件也能使产物失效。
- 最小 mesh 或 splat 应用无需 `EditorPlugin` 即可初始化已声明的渲染 Shader；
  预配置资产存储在插件组合后仍被保留。必需依赖和锚点缺失时给出可操作的诊断。
- renderer 和 trainer 使用同一套经过校验的 Gaussian 领域接口；属性数组长度
  不一致时拒绝数据，不做截断。
- UI 和无窗口测试执行相同的编辑器操作，Transform 编辑可撤销，导入及加载具有
  明确结果。
- 严格 GPU 任务在受支持硬件上执行公开契约和代表性 mesh/splat 测试；不受支持
  的设备使该任务明确失败。

## Risks

线程局部资源会影响 system param 获取和访问检查。存储改动需要同时审计别名和栈
展开行为；仅增加资源类别无法保证 extraction 安全。

场景模板可以修改资源或无关实体。在提交接口限制这些副作用前，实体清理只能提供
有限的失败保证。Shader generation 的变化也必须传递给全部消费 pipeline，而不
只是编译缓存。

Gaussian 拆分必须保留导入、渲染和训练之间的参数布局及颜色约定。编辑器操作队列
必须保留同一帧内 Transform 传播的顺序。这两项改动都不应在缺少具体消费者时增加
额外的通用框架。

严格 GPU 任务需要持续维护硬件和驱动。本提案不依据静态检查作出吞吐量、帧时间、
覆盖率或 GPU 校验通过的承诺。
