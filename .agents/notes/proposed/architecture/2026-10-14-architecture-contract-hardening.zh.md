# Agent Note: Architecture contract hardening

Status: proposed

[English](2026-10-14-architecture-contract-hardening.md)

## Problem

[Vulkan 枢纽](../../implemented/architecture/2026-10-13-rhi-vulkan-pivot.zh.md)、
[渲染记录上下文](../../implemented/architecture/2026-09-09-render-recording-context.zh.md)和
[共享设备生命周期](../../implemented/architecture/2026-09-13-shared-device-context-lifecycle.zh.md)
提供渲染基础。已落地契约分别由以下决策记录维护：

1. [Main-world parking 和线程访问](../../implemented/architecture/2026-10-14-main-world-parking-and-thread-contracts.zh.md)。
2. [场景提交](../../implemented/architecture/2026-10-14-scene-load-commit-semantics.zh.md)。
3. [Shader 请求身份](../../implemented/architecture/2026-10-14-shader-request-identity.zh.md)。
4. [功能 Shader 初始化](../../implemented/architecture/2026-10-14-feature-shader-ownership.zh.md)。
5. [Gaussian 领域](../../implemented/architecture/2026-10-14-shared-gaussian-domain.zh.md)。
6. [编辑器操作](../../implemented/architecture/2026-10-14-editor-action-interface.zh.md)。

待处理工作涉及这些接口之间的契约。以下 35 条审查记录保留原编号，供实现时跟踪：
34 项发现仍待处理，源码复核排除 R4。
P1 表示内存安全、无效 GPU 使用或核心计算错误；P2 表示具有具体触发条件的功能和
恢复缺陷。除非证据章节明确说明，源码观察不代表运行时复现。

### ECS borrowing and storage

- **E1 / P1：** [直接组件读取](../../../../crates/moonfield-ecs/src/world.rs#L843)
  绕过列借用保护，而 `SystemState<Query<&mut T>>::get(&World)` 允许对同一个值
  取得可变访问。
- **E2 / P1：** [QueryGetGuard](../../../../crates/moonfield-ecs/src/query.rs#L202)
  通过 Deref 暴露可复制的 `&'w T`。该引用可在释放列借用的 guard 销毁后继续存在，
  使后续可变访问与它冲突。
- **E3 / P1：** [Bundle 插入](../../../../crates/moonfield-ecs/src/world.rs#L1134)
  对组件元数据去重，却不对值回调去重。重复的 T 会两次析构旧 T，并覆盖一个传入值。
- **E4 / P1：** [ZST 列](../../../../crates/moonfield-ecs/src/archetype.rs#L615)
  为要求更高对齐的组件使用仅按 1 字节对齐的悬空指针。
- **E5 / P1：** [Archetype 清理](../../../../crates/moonfield-ecs/src/archetype.rs#L288)
  和[组件替换](../../../../crates/moonfield-ecs/src/world.rs#L1134)在调用用户 Drop
  时仍保留活跃所有权元数据。栈展开后，已析构值可能再次被析构。
- **E7 / P2：** [Tuple fetch 获取](../../../../crates/moonfield-ecs/src/query.rs#L459)
  在后续借用 panic 时，不会释放已取得的列借用。

### Entity identity and scope restoration

- **E8 / P2：** [实体清理](../../../../crates/moonfield-ecs/src/entities.rs#L520)
  重置 generation 元数据，使旧 Entity 能够标识一个新实体。
- **E9 / P2：** [alloc_at](../../../../crates/moonfield-ecs/src/entities.rs#L394)
  选择槽位时没有一致校验或采用传入的 generation；spawn_at 可能跳过 hook，或产生
  无法通过传入句柄访问的实体。
- **E10 / P2：** [保留实体的存在性判断](../../../../crates/moonfield-ecs/src/entities.rs#L503)
  在 pending 分支忽略 generation，接受过期句柄。
- **E13 / P2：** [resource_scope](../../../../crates/moonfield-ecs/src/world.rs#L597)
  在 panic 时丢失借出的资源。正常的移除和重插又向资源析构顺序追加记录，改变依赖
  对象的析构顺序。
- **E14 / P2：** [Schedule 执行](../../../../crates/moonfield-ecs/src/world.rs#L736)
  在运行前移除 schedule，仅在正常返回时恢复；捕获系统或 command panic 后，
  对应 label 不再有可运行的 schedule。

### Hierarchy mutation and propagation

- **E6 / P1：** [Transform 传播](../../../../crates/moonfield-ecs/src/hierarchy.rs#L190)
  在全部变化祖先完成更新前，将父 affine 放入队列。过期的 pending 项可以覆盖已经
  正确重算的后代。
- **E11 / P2：** [ChildOf](../../../../crates/moonfield-ecs/src/hierarchy.rs#L28)
  允许普通的可变字段访问绕过关系 hook，使 Children 和循环检查与存储的父关系不一致。
- **E12 / P2：** [循环校验](../../../../crates/moonfield-ecs/src/hierarchy.rs#L113)
  在插入新关系并移除旧关系后运行。被拒绝的链接可能继续存储，旧反向链接也没有恢复。
- **E15 / P2：** [Children 删除](../../../../crates/moonfield-ecs/src/hierarchy.rs#L72)
  使用 swap_remove，与按插入顺序遍历的契约冲突。

### Vulkan image contracts

- **R1 / P1：** [附件转换](../../../../crates/moonfield-rhi/src/backend/vulkan/formats.rs#L138)
  把 PRESENT_SRC_KHR 传给 dynamic rendering。Vulkan 1.4 同样禁止这种附件布局：
  `VUID-VkRenderingAttachmentInfo-imageView-06145`。
  [Dynamic rendering 契约](../../implemented/architecture/2026-08-24-dynamic-rendering-replaces-render-pass.zh.md)
  要求显式转换；声明附件布局不会记录 initial 或 final barrier。
- **R2 / P1：** [Offscreen 创建](../../../../crates/moonfield-rhi/src/backend/vulkan/offscreen.rs#L168)
  和 resize 只初始化颜色布局；新深度图像仍处于 UNDEFINED，渲染却声明 GENERAL。
- **R11 / P2：** [Swapchain 格式报告](../../../../crates/moonfield-rhi/src/backend/vulkan/swapchain.rs#L568)
  将 SRGB view 表示为 UNORM，使 pipeline 附件格式与实际 view 格式不同。
- **R12 / P2：** [Offscreen 回读](../../../../crates/moonfield-rhi/src/backend/vulkan/offscreen.rs#L322)
  将 fragment 读取声明为 source scope，没有覆盖内容生产者的写入。
- **R13 / P2：** [Texture 回读](../../../../crates/moonfield-rhi/src/backend/vulkan/texture.rs#L175)
  可用于创建时没有 TRANSFER_SRC 的 storage 和 bindless 纹理。
- **R14 / P2：** [Texture 上传](../../../../crates/moonfield-rhi/src/backend/vulkan/texture.rs#L387)
  转发未经检查的字节长度和复制区域；过短的 staging 数据及越界目标能进入图像复制记录。

[附件有效用法规则](https://docs.vulkan.org/refpages/latest/refpages/source/VkRenderingAttachmentInfo.html)
独立于[统一图像布局](https://docs.vulkan.org/features/latest/features/proposals/VK_KHR_unified_image_layouts.html)
生效。呈现时保持 PRESENT 合法；渲染需要显式进入和退出可用布局。

### GPU ownership and completion

- **R3 / P1：** [退休槽](../../../../crates/moonfield-rhi/src/backend/vulkan/retire.rs#L158)
  每次 begin 都会清空。重试被中止的帧复用同一槽，可能在较新在途引用完成前释放资源。
- **R4 / P1：** 经源码复核排除。
  [FrameContext::drop](../../../../crates/moonfield-render-core/src/window.rs#L379)
  在字段析构前调用
  [Device::wait_idle](../../../../crates/moonfield-rhi/src/backend/vulkan/device.rs#L1061)。
  缺少等待的判断不作为待修缺陷；关闭路径的 GPU 覆盖仍属于验证要求。
- **R5 / P1：** [呈现信号量](../../../../crates/moonfield-rhi/src/backend/vulkan/swapchain.rs#L149)
  属于帧槽，而帧槽复用只等待 graphics 完成，不能证明呈现等待已经消费该信号量。
- **R6 / P1：** [WindowSurfaceData](../../../../crates/moonfield-render-core/src/window.rs#L423)
  在退休 swapchain 之前析构 Surface，违反
  `VUID-vkDestroySurfaceKHR-surface-01266`。
- **R9 / P2：** [TextureView](../../../../crates/moonfield-rhi/src/backend/vulkan/view.rs#L16)
  及其 clone 维持设备存活，却不保留图像/view 的拥有者或携带其借用生命周期。
- **R10 / P2：** [Surface 构造](../../../../crates/moonfield-rhi/src/backend/vulkan/swapchain.rs#L57)
  不保留原生窗口，也不将返回的 Surface 绑定到窗口生命周期。
- **F1 / P1：** [Splat 相机上传](../../../../crates/moonfield-render-feature/src/splat/forward_pass.rs#L659)
  改写同一个持久化 allocation，而前序帧可能仍在读取它。

[呈现信号量指南](https://docs.vulkan.org/guide/latest/swapchain_semaphore_reuse.html)
区分 graphics 完成与呈现等待消费。
[Surface 销毁](https://docs.vulkan.org/refpages/latest/refpages/source/vkDestroySurfaceKHR.html)
要求先销毁为该 surface 创建的每个 swapchain。

### Descriptor heap ranges

- **R7 / P1：** 公开的[buffer 写入](../../../../crates/moonfield-rhi/src/backend/vulkan/descriptor_heap.rs#L369)
  和[sampler 写入](../../../../crates/moonfield-rhi/src/backend/vulkan/descriptor_heap.rs#L419)
  接受可公开构造的 handle，并生成未经检查的 host 输出范围。crate 私有的图像
  descriptor 写入使用相同的未检查偏移模式。
- **R8 / P1：** [Heap 绑定](../../../../crates/moonfield-rhi/src/backend/vulkan/descriptor_heap.rs#L456)
  在偏移零保留空间，应用槽也从那里开始；可用容量没有排除驱动的保留区域。

[vkCmdBindResourceHeapEXT](https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdBindResourceHeapEXT.html)
禁止应用在命令缓冲绑定保留区域时访问该范围。

### Asset and prepared-cache identity

- **A1 / P2：** [Assets](../../../../crates/moonfield-asset/src/lib.rs#L182)
  仅按 index/generation 标识值。
  [AssetServer](../../../../crates/moonfield-asset/src/server.rs#L112)可将另一 store
  中的不相关资产作为路径缓存命中返回。
- **F2 / P2：** [Splat 准备](../../../../crates/moonfield-render-feature/src/splat/prepare.rs#L248)
  跳过新的空 revision，却不移除旧 prepared cloud。渲染仍按 AssetId 消费原 GPU 数据。

### Window runner control

- **W1 / P2：** [普通帧和 modal 帧](../../../../crates/moonfield-winit/src/lib.rs#L620)
  丢弃 App::update 的退出结果，只响应 WindowControl。
- **W2 / P2：** [Reactive 调度](../../../../crates/moonfield-winit/src/lib.rs#L570)
  将 Duration::MAX 加到 Instant 上，但配置承诺该值代表无限等待。

### Editor history identity

- **UI1 / P2：** [Redo](../../../../crates/moonfield-editor/src/action.rs#L300)
  只更新创建条目的逆操作。导入、移动、撤销两次、重做两次会以新 generation 重建实体，
  随后的移动仍引用已销毁的 Entity，因而失败。

## Evidence

E1/E2 的纯安全 API 示例通过编译；没有执行未定义行为，也没有使用 Miri 检查。
E3 记录旧值的 Drop 为 `[7, 7]`。E8 复现旧新句柄相同，过期查询返回新值。
E6 在 200 个构造的 World 中失败：根和子节点的 local x 改为 20 和 4，
孙节点 local x 为 3，根位于另一 archetype，预期 global x 为 27。
A1 复现没有额外 loader 调用的不相关缓存命中。W2 复现 checked_add 返回 None，
普通 Instant 加法 panic。UI1 由历史操作及 allocator generation 变化的追踪确认。
当前 Drop 实现与 R4 缺少析构等待的判断相矛盾；该排除依据源码，不代表 GPU 测试通过。

复用源码的隔离测试入口通过 133 个 ECS 测试、22 个 App 测试、
18 个 Time core/real/virtual 测试和 1 个 Winit 配置测试。
这些结果不代表锁定依赖的 workspace 测试通过：Slang 依赖的 imgui 子模块无法离线检出。
GPU 观察依据代码和所引用的规范，不声称已在合格 Vulkan 环境执行验证。

## Proposal

按以下顺序分别实施：

- **ECS 访问和存储。** 让直接访问、Query 迭代和实体 guard 使用同一套可执行的
  借用规则。将引用有效性绑定到保护它的借用，或要求独占 World 访问；在接口设计时
  选择机制。在 hook 或所有权转移前拒绝重复 bundle 类型，满足 ZST 指针对齐，
  并在用户 Drop 前记录析构进度。tuple 的部分获取失败必须正确展开。
- **World 身份和层级。** 在 clear、reservation 和 spawn_at 中继续拒绝过期句柄。
  栈展开时恢复资源和 schedule，保留其依赖顺序。关系变更在提交前经过校验并维护
  反向链接。使用祖先最终状态传播，保留公开约定的兄弟顺序。
- **GPU 完成和所有权。** 按已完成的提交值退休资源，abort 和重复 begin 不作为
  完成事件。析构前等待，保留图像/view 和窗口拥有者，在 Surface 前销毁全部
  swapchain。按 acquire 的 image 或明确的呈现完成条件复用呈现信号量，并为受
  保护的每个帧槽分配相机记录。
- **图像和 descriptor 有效性。** 记录显式的渲染/呈现转换及深度初始化转换。
  保留实际附件格式，为回读追踪内容生产者的写入，校验 usage 和上传范围。
  检查 descriptor 输出范围及槽归属，分配不重叠且正确对齐的驱动保留区域。
  保留 untyped shader heap 访问。
- **缓存、runner 和历史行为。** 将 handle 和路径缓存绑定到资产 store，或编码
  store 身份。明确表示空 prepared revision。两种 runner 路径均响应 AppExit，
  将无限 reactive 等待映射到 ControlFlow::Wait。为编辑器创建的对象赋予逻辑身份，
  重放时重映射依赖的 action 和 inverse，覆盖嵌套 batch 及加载场景的后代。
- **验证和决策记录。** 将以下反例加入其所属测试套件。在合格硬件上使用现有的
  [严格 GPU 契约模式](../../implemented/testing/2026-10-14-strict-gpu-contract-mode.zh.md)
  执行 GPU 路径。修正渲染布局理由；运行时修复满足验收条件前，不将其记录为已完成。

[Splat forward 决策](../../implemented/architecture/2026-10-14-splat-forward-pass.zh.md)
维护颜色和 kernel 约定。保留这些约定、同步资产加载、编辑器的同帧反馈及 RHI
公开 API 的后端边界。

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

- **E1/E2/E7：** 在引用发生冲突前拒绝不兼容的安全访问。guard 产生的引用不能在
  借用释放后继续有效；捕获部分获取失败后，已取得的列均可再次借用。覆盖 tuple、
  Option、直接访问、迭代及实体查询。
- **E3/E4/E5：** 拒绝重复 bundle 时不改变实体，也不触发修改 hook。高对齐 ZST
  的 query/Drop 指针有效。捕获 Drop panic 后不重复析构值，不将已析构值暴露为
  活跃值。覆盖插入、替换、clear 和 World 析构。
- **E8/E9/E10：** 旧 Entity 在 clear、复用和 reservation 后仍无效。
  非默认或过期的 spawn_at generation 在修改前被拒绝，或按明确的恢复契约一致采用。
- **E13/E14：** 捕获 resource、system 和 command panic 后，保留作用域中的
  资源或 schedule 及其依赖顺序。运行期间注册的系统仍遵守 schedule 合并契约。
- **E6/E11/E12/E15：** 每条受支持的 reparent 路径均维护两个方向的链接。
  拒绝循环后恢复原图。上述三层结构同时变化时，在不同 archetype 顺序下均得到
  global x=27；删除中间子节点后保留兄弟插入顺序。
- **R1/R2/R11：** 窗口、颜色和深度目标的首次使用及 resize 均满足 Vulkan
  附件布局和格式规则。覆盖真实 SRGB surface 格式，不将其报告为 UNORM，
  也不为它创建 UNORM pipeline。
- **R12/R13/R14：** 回读以合法 usage 观察颜色和 compute 写入。在记录复制前
  拒绝过短数据、负 offset、extent 溢出和越界上传区域。
- **R3/R4/R6：** 退休对象在全部可能的提交完成前保持存活，覆盖失败帧重试。
  提交后关闭及 swapchain 退休期间关闭，均按合法顺序释放命令缓冲和 swapchain。
- **R5：** 多帧槽仅在呈现等待已消费后复用信号量。resize 和退休满足相同完成规则。
- **R9/R10/F1：** view 和 surface 保留所需拥有者，或强制其生命周期。
  多帧在途时移动相机，不修改另一帧仍在读取的 allocation。
- **R7/R8：** 伪造、外来、已释放和越界 handle 均不进入驱动输出指针。
  使用 checked heap 算术及非零驱动保留区域，得到不重叠且正确对齐的应用范围。
- **A1/F2：** 跨 store 的 handle 和路径缓存命中不能解析为不相关值。
  将可见 cloud 替换为空 revision 后移除其绘制数据；消费 pipeline 观察 prepared revision。
- **W1/W2：** AppExit 终止普通帧和 Windows modal 帧路径。
  Duration::MAX 及其他不可表示的 deadline 进入无限等待，不发生 panic。
- **UI1：** 导入/加载、移动、撤销两次、重做两次在反复循环中将移动应用到重建对象。
  覆盖加载的后代及嵌套 batch；重放失败保留历史，不留下部分修改。
- 现有公开契约测试、Agent Note 配对和 RHI 边界检查保持通过。
  严格 GPU 测试实际执行受影响路径，不将不受支持的硬件计为测试通过。

## Risks

借用改动影响 SystemParam、tuple fetch、变更检测及迭代器返回的引用。
需要修复完整的引用/借用契约；只检查一个 getter 不能关闭其他逃逸路径。

可安全展开的所有权转移必须容忍用户 Drop 和 hook，不能使已析构值可见。
资源和 schedule 恢复必须保留现有注册和析构规则。

保留拥有者不能单独证明 GPU 完成，呈现完成也不同于 graphics 完成。
新的退休机制必须覆盖 abort、resize、关闭和独立保留的 view，避免产生 Arc 循环。

Store 身份及编辑器逻辑身份可能改变 handle 布局、缓存键和重放记录。
重放场景模板仍遵守场景提交决策限定的副作用保证；实体重映射不会为任意资源或
文件写入提供通用回滚。

严格 GPU 验证需要持续维护硬件和驱动。CPU 测试和文档检查通过不能证明 GPU
有效性、性能或不存在未定义行为。
