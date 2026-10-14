# Agent Note: Architecture contract hardening

Status: proposed

[中文](2026-10-14-architecture-contract-hardening.zh.md)

## Problem

The RHI, render-core, render-feature, and editor split provides useful ownership
and dependency separation. The [Vulkan pivot](../../implemented/architecture/2026-10-13-rhi-vulkan-pivot.md),
[render recording context](../../implemented/architecture/2026-09-09-render-recording-context.md),
and [shared device lifecycle](../../implemented/architecture/2026-09-13-shared-device-context-lifecycle.md)
remain the foundations. The improvement target is the contracts callers must
uphold outside those modules, rather than another workspace-wide rewrite.

Static inspection identifies six areas:

- **Lifetime and thread safety.** `World::park_main_world` stores a borrowed
  world's address through a public safe method without retaining its lifetime.
  `Extract::fetch` dereferences it internally; `App::render` supplies the intended
  scope, but independent callers can retain the resource after its source dies.
  `Resource` requires `Send + Sync`, and `CommandBuffer` manually implements both
  despite containing a `Cell<bool>` and relying on externally synchronized
  recording. These contracts need a soundness audit before concurrency work.
- **Scene failure consistency.** `ResolvedScene::apply` spawns entities before
  fallible template construction completes. `load_scene` resolves and applies
  roots one at a time. An error can leave entities from the failed operation in
  the destination world.
- **Shader compilation identity.** `PreparedShaders::needs_prepare` compares
  only the pipeline name, asset id, and asset revision. Replacing a request's
  entry points, reflection entry, or capabilities for the same asset revision
  does not trigger preparation. `ShaderCacheKey` also omits imported file
  contents; dependency invalidation is needed before reliable hot reload.
- **Plugin initialization ownership.** `EditorPlugin` loads mesh, splat, and
  egui shader requests through `scene_io::load_pipeline_shaders`.
  `RenderFeaturePlugin` initializes empty stores and requests by inserting
  defaults, which can overwrite caller-provided resources. Rendering without
  the editor requires application code to reproduce feature initialization.
- **Gaussian domain ownership.** The trainer imports `GaussianScene` from
  `moonfield-render-feature`, bringing framework dependencies into a consumer
  of CPU domain data. Its parallel public vectors allow mismatched lengths;
  `pack_gaussians` silently truncates to the shortest required attribute array.
- **Editor operation ownership.** UI callbacks perform asset loading, scene
  I/O, and transform edits directly against `World`. Undo, background import,
  and automation would otherwise need separate implementations of those actions.

Evidence lives in `moonfield-ecs/src/{world,resource}.rs`,
`moonfield-rhi/src/{backend/vulkan/command,shader/compile}.rs`,
`moonfield-render-core/src/extract.rs`, `moonfield-scene/src/{resolved,file}.rs`,
`moonfield-render-feature/src/{shader,plugin,splat/scene,splat/prepare}.rs`, and
`moonfield-editor/src/{lib,scene_io,ui}.rs`, all under `crates/`.
These observations are static findings, not dynamic reproductions or measured
performance regressions.

## Proposal

Implement independent changes in this order:

1. **P0 — lifetime and thread contracts.** Add thread-local resource storage for
   objects that cannot soundly satisfy `Send + Sync`. Audit manual thread-trait
   implementations rather than removing them indiscriminately. Contain
   main-world parking in a controlled extraction scope that installs, runs, and
   removes the resource, including on unwind. Any retained raw-pointer entry
   must expose its unsafe preconditions. Prefer exclusive recording interfaces
   where shared access cannot be synchronized safely.
2. **P1 — scene commit semantics.** Parse and validate all roots before spawning.
   Separate asset resolution and load-plan preparation from world mutation.
   Track operation-owned entities and clean them up on failure; define whether
   successfully loaded assets may remain cached. A stronger atomic commit
   requires restricting arbitrary template side effects, not just despawning
   the root.
3. **P1 — shader identity.** Give each compile request a fingerprint covering
   its entry set, reflection entry, capabilities, and supported compile options.
   Separate source revision from prepared-artifact generation, and make pipeline
   rebuilds observe the latter. Preserve the last usable pipeline on failure.
   Add import dependency invalidation when introducing hot reload.
4. **P1 — feature initialization.** Each rendering feature declares and
   registers its own shader needs using application-supplied asset configuration.
   The editor owns only egui's requests. Initialize absent resources without
   replacing explicit configuration, and validate required plugin dependencies
   and schedule anchors after composition.
5. **P2 — shared Gaussian domain.** Extract CPU Gaussian data and conventions
   into a module shared by the renderer and trainer without app/render-core
   dependencies. Choose crate packaging from the resulting dependency graph.
   Validate equal attribute lengths, numeric values, and representation
   conventions at construction/update interfaces; packing returns an error
   rather than silently discarding records. The [splat forward decision](../../implemented/architecture/2026-10-14-splat-forward-pass.md)
   owns the current color and kernel conventions.
6. **P2 — editor actions.** Route UI, keyboard, and automation through one
   `EditorAction` execution interface for asset import, scene loading, and
   transform edits. The execution module owns validation, results, and undo
   records; UI code owns presentation. Establish this interface before adding
   background jobs, and keep render-frame handoff outside the action queue.

Keep conservative recording barriers until GPU measurements justify narrower
synchronization. Keep normal CI tolerant of unsupported hardware, but add a
strict GPU contract mode on a qualifying runner: device initialization failure
must fail that job rather than return a passing skipped test. Existing public
contract suites are the starting test surface; a mock RHI is not required.

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

- Safe extraction calls cannot read a destroyed source world. Extraction unwind
  leaves no parked pointer, and the app's main world remains recoverable.
- Thread-local GPU resources need no unsupported `Send + Sync` promises;
  recording access and pool synchronization have explicit, audited contracts.
- Invalid later roots, missing assets, and failed child templates leave no
  operation-owned entities behind. Tests specify asset-cache retention and any
  permitted non-entity side effects.
- Changing a shader request without changing its source triggers preparation
  and pipeline rebuilding. Failed preparation preserves a usable pipeline.
  Import-only edits invalidate artifacts when hot reload is implemented.
- A minimal mesh or splat app initializes its declared rendering shaders
  without `EditorPlugin`; preconfigured asset stores survive plugin composition.
  Missing required dependencies and anchors produce actionable diagnostics.
- Renderer and trainer consume one validated Gaussian domain interface;
  mismatched attribute arrays are rejected rather than truncated.
- UI and headless tests execute the same editor actions, with reversible
  transform edits and explicit import/load results.
- The strict GPU job executes public contract and representative mesh/splat
  tests on supported hardware; unsupported devices fail that job explicitly.

## Risks

Thread-local resources affect system parameter fetching and access checks. Audit
aliasing and unwind behavior together with storage changes; a new resource
category alone does not make extraction safe.

Scene templates can mutate resources or unrelated entities. Entity cleanup is
only a bounded failure guarantee until the commit interface restricts these
effects. Shader generation changes likewise must reach all consuming pipelines,
not only the compile cache.

The Gaussian split must preserve parameter layouts and color conventions across
import, rendering, and training. Editor action queues must preserve same-frame
transform propagation ordering. Neither change justifies an additional generic
framework without a concrete consumer.

A strict GPU job needs maintained hardware and drivers. This proposal makes no
throughput, frame-time, coverage, or GPU-validation claim from static inspection.
