# Agent Note: Architecture contract hardening

Status: proposed

[中文](2026-10-14-architecture-contract-hardening.zh.md)

## Problem

The [Vulkan pivot](../../implemented/architecture/2026-10-13-rhi-vulkan-pivot.md),
[render recording context](../../implemented/architecture/2026-09-09-render-recording-context.md),
and [shared device lifecycle](../../implemented/architecture/2026-09-13-shared-device-context-lifecycle.md)
provide the rendering foundations. Shipped contracts have separate owners:

1. [Main-world parking and thread access](../../implemented/architecture/2026-10-14-main-world-parking-and-thread-contracts.md).
2. [Scene commits](../../implemented/architecture/2026-10-14-scene-load-commit-semantics.md).
3. [Shader request identity](../../implemented/architecture/2026-10-14-shader-request-identity.md).
4. [Feature shader initialization](../../implemented/architecture/2026-10-14-feature-shader-ownership.md).
5. [The Gaussian domain](../../implemented/architecture/2026-10-14-shared-gaussian-domain.md).
6. [Editor actions](../../implemented/architecture/2026-10-14-editor-action-interface.md).

The remaining work concerns contracts between those interfaces. The following
35 review records retain their identifiers for implementation tracking:
34 findings remain open; source reinspection excludes R4. P1
denotes memory safety, invalid GPU use, or core computation errors; P2 denotes
functional and recovery defects with specific triggers. A source observation
is not a runtime reproduction unless the evidence section says so.

### ECS borrowing and storage

- **E1 / P1:** [Direct component reads](../../../../crates/moonfield-ecs/src/world.rs#L843)
  bypass column leases while `SystemState<Query<&mut T>>::get(&World)` permits
  mutable access to the same value.
- **E2 / P1:** [QueryGetGuard](../../../../crates/moonfield-ecs/src/query.rs#L202)
  exposes a copyable `&'w T` through `Deref`. That reference can outlive the guard
  that releases its column lease, allowing conflicting mutable access.
- **E3 / P1:** [Bundle insertion](../../../../crates/moonfield-ecs/src/world.rs#L1134)
  deduplicates component metadata without deduplicating value callbacks.
  Repeated T entries drop the old T twice and overwrite an incoming value.
- **E4 / P1:** [ZST columns](../../../../crates/moonfield-ecs/src/archetype.rs#L615)
  use an alignment-1 dangling pointer for components with higher alignment.
- **E5 / P1:** [Archetype cleanup](../../../../crates/moonfield-ecs/src/archetype.rs#L288)
  and [replacement](../../../../crates/moonfield-ecs/src/world.rs#L1134) retain
  live ownership metadata across user Drop calls. Unwind can leave already
  destroyed values eligible for another destruction.
- **E7 / P2:** [Tuple fetch acquisition](../../../../crates/moonfield-ecs/src/query.rs#L459)
  does not release earlier column leases if a later acquisition panics.

### Entity identity and scope restoration

- **E8 / P2:** [Entity clearing](../../../../crates/moonfield-ecs/src/entities.rs#L520)
  resets generation metadata, allowing an old Entity to identify a new entity.
- **E9 / P2:** [alloc_at](../../../../crates/moonfield-ecs/src/entities.rs#L394)
  selects a slot without consistently validating or adopting the supplied
  generation; `spawn_at` can skip hooks or produce an inaccessible entity.
- **E10 / P2:** [Reserved-entity membership](../../../../crates/moonfield-ecs/src/entities.rs#L503)
  ignores generation in its pending branch and accepts stale handles.
- **E13 / P2:** [resource_scope](../../../../crates/moonfield-ecs/src/world.rs#L597)
  loses the borrowed resource on panic. Normal removal and reinsertion append
  another entry to the resource teardown order, changing dependency ordering.
- **E14 / P2:** [Schedule execution](../../../../crates/moonfield-ecs/src/world.rs#L736)
  removes a schedule before running it and restores it only on normal return;
  caught system or command panics leave its label with no running schedule.

### Hierarchy mutation and propagation

- **E6 / P1:** [Transform propagation](../../../../crates/moonfield-ecs/src/hierarchy.rs#L190)
  queues parent affines before all changed ancestors are final. A stale
  pending item can overwrite a correctly recomputed descendant.
- **E11 / P2:** [ChildOf](../../../../crates/moonfield-ecs/src/hierarchy.rs#L28)
  permits ordinary mutable field access that bypasses relationship hooks,
  leaving Children and cycle checks inconsistent with the stored parent.
- **E12 / P2:** [Cycle validation](../../../../crates/moonfield-ecs/src/hierarchy.rs#L113)
  runs after inserting the new relationship and discarding the old one.
  A rejected link can remain stored without restoring the old reverse link.
- **E15 / P2:** [Children removal](../../../../crates/moonfield-ecs/src/hierarchy.rs#L72)
  uses `swap_remove`, contradicting its insertion-order traversal contract.

### Vulkan image contracts

- **R1 / P1:** [Attachment conversion](../../../../crates/moonfield-rhi/src/backend/vulkan/formats.rs#L138)
  supplies `PRESENT_SRC_KHR` to dynamic rendering. Vulkan 1.4 also prohibits
  this attachment layout: `VUID-VkRenderingAttachmentInfo-imageView-06145`.
  The [dynamic-rendering contract](../../implemented/architecture/2026-08-24-dynamic-rendering-replaces-render-pass.md)
  requires explicit transitions; the attachment layout declaration does not
  record initial or final barriers.
- **R2 / P1:** [Offscreen creation](../../../../crates/moonfield-rhi/src/backend/vulkan/offscreen.rs#L168)
  and resize initialize only color layout; newly created depth images remain
  UNDEFINED while rendering declares GENERAL.
- **R11 / P2:** [Swapchain format reporting](../../../../crates/moonfield-rhi/src/backend/vulkan/swapchain.rs#L568)
  represents SRGB views as UNORM, making the pipeline attachment format differ
  from the actual view format.
- **R12 / P2:** [Offscreen readback](../../../../crates/moonfield-rhi/src/backend/vulkan/offscreen.rs#L322)
  names fragment reads as its source scope rather than the producer writes.
- **R13 / P2:** [Texture readback](../../../../crates/moonfield-rhi/src/backend/vulkan/texture.rs#L175)
  is available on storage and bindless textures constructed without TRANSFER_SRC.
- **R14 / P2:** [Texture upload](../../../../crates/moonfield-rhi/src/backend/vulkan/texture.rs#L387)
  forwards unchecked byte lengths and copy regions; short staging data and
  out-of-bounds destinations can reach recorded image copies.

The [attachment valid-usage rule](https://docs.vulkan.org/refpages/latest/refpages/source/VkRenderingAttachmentInfo.html)
applies independently of [unified image layouts](https://docs.vulkan.org/features/latest/features/proposals/VK_KHR_unified_image_layouts.html).
Keeping PRESENT for presentation remains valid; rendering needs explicit
transitions into and out of its usable layout.

### GPU ownership and completion

- **R3 / P1:** [Retirement slots](../../../../crates/moonfield-rhi/src/backend/vulkan/retire.rs#L158)
  drain on every begin. Retrying an aborted frame reuses the same slot and can
  retire resources before newer in-flight references complete.
- **R4 / P1:** Excluded by source reinspection.
  [FrameContext::drop](../../../../crates/moonfield-render-core/src/window.rs#L379)
  calls [Device::wait_idle](../../../../crates/moonfield-rhi/src/backend/vulkan/device.rs#L1061)
  before its fields are destroyed. The missing-wait allegation is not an open
  defect; shutdown GPU coverage remains a verification requirement.
- **R5 / P1:** [Present semaphores](../../../../crates/moonfield-rhi/src/backend/vulkan/swapchain.rs#L149)
  belong to frame slots whose reuse waits only for graphics completion.
  That does not prove the presentation wait consumed the semaphore.
- **R6 / P1:** [WindowSurfaceData](../../../../crates/moonfield-render-core/src/window.rs#L423)
  drops Surface before its retired swapchains, contrary to
  `VUID-vkDestroySurfaceKHR-surface-01266`.
- **R9 / P2:** [TextureView](../../../../crates/moonfield-rhi/src/backend/vulkan/view.rs#L16)
  and its clones keep the device alive without retaining the image/view owner
  or carrying its borrow lifetime.
- **R10 / P2:** [Surface construction](../../../../crates/moonfield-rhi/src/backend/vulkan/swapchain.rs#L57)
  does not retain the native window or bind the returned Surface to its lifetime.
- **F1 / P1:** [Splat camera uploads](../../../../crates/moonfield-render-feature/src/splat/forward_pass.rs#L659)
  rewrite one persistent allocation while previous frames may still read it.

The [present semaphore guide](https://docs.vulkan.org/guide/latest/swapchain_semaphore_reuse.html)
distinguishes graphics completion from consumption of a present wait.
[Surface destruction](https://docs.vulkan.org/refpages/latest/refpages/source/vkDestroySurfaceKHR.html)
requires destruction of every swapchain created for that surface.

### Descriptor heap ranges

- **R7 / P1:** Public [buffer writes](../../../../crates/moonfield-rhi/src/backend/vulkan/descriptor_heap.rs#L369)
  and [sampler writes](../../../../crates/moonfield-rhi/src/backend/vulkan/descriptor_heap.rs#L419)
  accept publicly constructible handles and form unchecked host output ranges.
  Crate-private image descriptor writes use the same unchecked offset pattern.
- **R8 / P1:** [Heap binding](../../../../crates/moonfield-rhi/src/backend/vulkan/descriptor_heap.rs#L456)
  reserves space at offset zero while application slots also start there;
  usable capacity does not exclude the implementation reservation.

[vkCmdBindResourceHeapEXT](https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdBindResourceHeapEXT.html)
forbids application access to the reserved range while command buffers bind it.

### Asset and prepared-cache identity

- **A1 / P2:** [Assets](../../../../crates/moonfield-asset/src/lib.rs#L182)
  identifies values only by index/generation. [AssetServer](../../../../crates/moonfield-asset/src/server.rs#L112)
  can return an unrelated asset from another store as a path-cache hit.
- **F2 / P2:** [Splat preparation](../../../../crates/moonfield-render-feature/src/splat/prepare.rs#L248)
  skips a new empty revision without removing the old prepared cloud.
  Rendering continues to consume the previous GPU data by AssetId.

### Window runner control

- **W1 / P2:** [Ordinary and modal frames](../../../../crates/moonfield-winit/src/lib.rs#L620)
  discard `App::update()`'s exit result and only honor WindowControl.
- **W2 / P2:** [Reactive scheduling](../../../../crates/moonfield-winit/src/lib.rs#L570)
  adds Duration::MAX to Instant although the configuration promises indefinite
  waiting for that value.

### Editor history identity

- **UI1 / P2:** [Redo](../../../../crates/moonfield-editor/src/action.rs#L300)
  refreshes only the creation entry's inverse. Import, move, undo twice,
  and redo twice recreates the entity with a new generation, then fails the
  dependent movement because it still names the destroyed Entity.

## Evidence

Safe-only examples for E1/E2 compile; they have not been executed as undefined
behavior or checked with Miri. E3 records the existing value's Drop as `[7, 7]`.
E8 reproduces identical old/new handles and a stale lookup returning the new
value. E6 fails in 200 constructed worlds when root and child local x become
20 and 4, the grandchild local x is 3, and the root occupies another archetype;
the expected global x is 27. A1 reproduces an unrelated cache hit without an
additional loader call. W2 reproduces `checked_add` returning None and ordinary
Instant addition panicking. UI1 is established by tracing history and allocator
generation changes. R4's teardown-wait allegation is contradicted by the
current Drop implementation; this exclusion is source-based, not a GPU pass.

Isolated source-based test entry points pass 133 ECS tests, 22 App tests,
18 Time core/real/virtual tests, and one Winit configuration test. They do not
establish passage of the locked workspace suite: the Slang dependency's imgui
submodule cannot be checked out offline. GPU observations are based on code and
the cited specification; no qualifying Vulkan execution is claimed.

## Proposal

Implement independent changes in this order:

- **ECS access and storage.** Make direct access, Query iteration, and entity
   guards share one enforceable borrowing rule. Bind reference validity to the
   protecting lease or require exclusive World access; choose the mechanism
   during interface design. Reject duplicate bundle types before hooks or
   ownership transfer, align ZST pointers, and track destruction progress before
   invoking user Drop. Partial tuple acquisition must unwind cleanly.
- **World identity and hierarchy.** Preserve stale-handle rejection through
   clear, reservation, and spawn_at. Restore resources and schedules on unwind
   without changing their dependency order. Route relationship changes through
   validation and reverse-link maintenance before committing them. Propagate
   from final ancestor state and preserve the documented sibling order.
- **GPU completion and ownership.** Retire against completed submission values;
   abort and repeated begin are not completion events. Wait before teardown,
   retain image/view and window owners, and destroy every swapchain before its
   Surface. Reuse present semaphores by acquired image or explicit present
   completion, and allocate camera records per protected frame slot.
- **Image and descriptor validity.** Record explicit rendering/presentation and
   depth initialization transitions. Preserve actual attachment formats, track
   producer writes for readback, and validate usage flags and upload ranges.
   Validate descriptor output ranges and slot ownership; allocate a disjoint,
   aligned implementation reservation. Preserve untyped shader heap access.
- **Cache, runner, and history behavior.** Bind handles and path caches to the
   owning asset store or encode store identity. Represent empty prepared
   revisions explicitly. Honor AppExit in both runner paths and map unbounded
   reactive waits to ControlFlow::Wait. Give creation-owned editor objects
   logical identities and remap dependent actions and inverses on replay,
   including nested batches and loaded descendants.
- **Verification and decision records.** Add the counterexamples below to the
   owning suites. Exercise GPU paths under the existing
   [strict GPU contract mode](../../implemented/testing/2026-10-14-strict-gpu-contract-mode.md)
   on qualifying hardware. Correct the rendering-layout rationale without
   claiming runtime fixes until those fixes satisfy their acceptance criteria.

The [splat forward decision](../../implemented/architecture/2026-10-14-splat-forward-pass.md)
owns its color and kernel conventions. Preserve those conventions, synchronous
asset loading, same-frame editor feedback, and the RHI public backend boundary.

## Alternatives considered

**Rewrite the workspace around another backend, a full render graph, or a
parallel ECS.** These do not resolve the identified lifetime, failure, and
initialization contracts. They add migration work before the existing
interfaces are safe and independently testable.

**Split large source files without changing interfaces.** File organization can
improve navigation, but it leaves UI-side world mutation, feature startup
knowledge, and caller-side synchronization requirements intact. Split along the
operation and ownership interfaces above instead of line counts alone.

**Introduce asynchronous loading first.** Synchronous loading is a deliberate
simplification. Moving it to workers before defining action results, scene
commit semantics, and thread-local resources makes existing contracts harder
to enforce. Establish those interfaces first.

## Acceptance criteria

- **E1/E2/E7:** Conflicting safe accesses are rejected before references overlap.
  Guard-derived references cannot survive lease release; catching partial
  acquisition failure leaves every acquired column borrowable. Cover tuple and
  Option composition, direct access, iteration, and entity lookup.
- **E3/E4/E5:** Duplicate bundles are rejected without changing the entity or
  firing mutation hooks. High-alignment ZST query/Drop pointers are valid.
  Caught Drop panics never destroy a value twice or expose a destroyed value as
  live. Exercise insertion, replacement, clear, and World destruction.
- **E8/E9/E10:** Old Entity values stay invalid after clear, reuse, and
  reservation. A non-default or stale spawn_at generation is either rejected
  before mutation or adopted consistently under a documented restore contract.
- **E13/E14:** Caught resource, system, and command panics preserve the scoped
  resource or schedule and its dependency order. Mid-run system registrations
  still obey the schedule merge contract.
- **E6/E11/E12/E15:** Every supported reparent path maintains both link
  directions. Rejected cycles restore the original graph. Simultaneous ancestor
  changes produce global x=27 in the described three-level case under archetype
  permutations; removing a middle child preserves sibling insertion order.
- **R1/R2/R11:** First use and resize of window, color, and depth targets satisfy
  Vulkan attachment-layout and format rules. Test an actual SRGB surface format
  without reporting or creating its pipeline as UNORM.
- **R12/R13/R14:** Readback observes color and compute writes with valid image
  usage. Short data, negative offsets, extent overflow, and out-of-range upload
  regions are rejected before copy recording.
- **R3/R4/R6:** Retired objects remain alive until all possible submissions
  complete, including failed-frame retry. Close after submit and close during
  swapchain retirement release command buffers and swapchains in valid order.
- **R5:** Multiple frame slots reuse present semaphores only after their waits
  are consumed. Resize and retirement satisfy the same completion rule.
- **R9/R10/F1:** Views and surfaces retain their required owners or enforce
  their lifetimes. Moving a camera with multiple frames in flight does not
  modify an allocation still read by another frame.
- **R7/R8:** Forged, foreign, freed, and out-of-range handles never reach a
  driver output pointer. Checked heap arithmetic and nonzero implementation
  reservations produce disjoint, correctly aligned application ranges.
- **A1/F2:** Cross-store handles and path-cache hits cannot resolve unrelated
  values. Replacing a visible cloud with an empty revision removes its draw
  data; consuming pipelines observe the prepared revision.
- **W1/W2:** AppExit terminates ordinary and Windows modal frame paths.
  Duration::MAX and other unrepresentable deadlines enter indefinite waiting
  without panicking.
- **UI1:** Import/load, move, undo twice, and redo twice applies the movement
  to the recreated object through repeated cycles. Include loaded descendants
  and nested batches; failed replay preserves history without partial mutation.
- Existing public contract tests, Agent Note pairing, and the RHI boundary
  check remain clean. Strict GPU tests execute the affected paths rather than
  counting unsupported hardware as a passing test.

## Risks

Borrowing changes affect SystemParam, tuple fetches, change detection, and
references returned by iterators. Repair the complete reference/lease contract;
checking one getter cannot close escape paths elsewhere.

Panic-safe ownership transfer must tolerate user Drop and hook code without
making destroyed values visible. Resource and schedule restoration must preserve
their existing registration and teardown rules.

Owner retention alone does not prove GPU completion. Present completion is also
distinct from graphics completion. A new retirement mechanism must cover abort,
resize, shutdown, and independently retained views without creating Arc cycles.

Store identity and editor logical identity may alter handle layout, cache keys,
and replay records. Replaying scene templates still has the bounded side-effect
guarantee owned by the scene-commit decision; entity remapping does not create
general rollback for arbitrary resources or file writes.

Strict GPU validation needs maintained hardware and drivers. CPU passing tests
and documentation checks do not establish GPU validity, performance, or the
absence of undefined behavior.
