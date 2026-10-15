# moonfield-rhi — RHI rules

Lunar Mare, the rendering RHI. See the root
[AGENTS.md](../../AGENTS.md) and [crates/AGENTS.md](../AGENTS.md) for standing
rules; this file adds what is specific to the RHI. The engine layer
(extraction, `ExtractedView`/`ViewTarget`, window frame loop, `RenderPlugin`)
lives in `moonfield-render-core` (Selene), never here.

## Crate layout

`moonfield-rhi` is one crate owning the public interface. The layout:

- `src/lib.rs` — the facade. The re-export list in `src/lib.rs` is the RHI's
  entire public surface; anything not re-exported there is not public API.
- `src/types.rs`, `src/error.rs`, `src/indirect.rs`, `src/shader/` — the
  backend-agnostic layer: the vocabulary, error types, indirect-argument
  layouts, and the shared Slang compiler (`Compiler`/`ShaderCache`/
  `CompiledShader` with a `ShaderTarget` (Spirv / MetalLib), `Reflection`,
  `RootBinder`/`RootParam`/`RootParamPlace`). Nothing in these modules may
  mention a backend.
- `src/backend/` — the private backend: `vulkan/` (ash), the pivot API (see
  the [Vulkan pivot note](../../.agents/notes/implemented/architecture/2026-10-13-rhi-vulkan-pivot.md)).
  It keeps the in-crate `gpu_tests/` (they verify `pub(crate)` internals and
  skip gracefully without qualifying hardware).

Dependency direction: `src/backend/*` → the vocabulary modules → nothing.
The `validation` feature enables the Vulkan validation layer on
development builds.

## Boundary discipline

- All `ash` types and Vulkan calls stay inside `backend/vulkan/`. The engine-level
  clip convention is Y-up with reverse-Z; any Vulkan viewport adjustment is made
  at this boundary, never in scene code.
- Public resource descriptions (`Format`, `BufferUsage`, `VertexBufferLayout`)
  live in `src/types.rs` as the crate's own vocabulary, not raw `ash`
  types. Backend mappings of the vocabulary live in the backend
  (`backend/vulkan/formats.rs`, the `ToVk` extension trait).
- **The public API exposes no backend types — no `ash`/`vk::`/`gpu_allocator`/
  `objc2`/`MTL*` in any public signature or trait impl, across the facade,
  the vocabulary modules, and every backend module.** New capabilities must be added as
  first-class APIs in the crate's vocabulary; there are no `raw()` escape
  hatches. `scripts/verify_rhi_boundary.py` (CI `rhi-boundary` job) enforces
  this mechanically.
- `gpu-allocator` is the allocation substrate of the Vulkan backend; it
  keeps allocator types out of the public API.
- Module map inside `backend/vulkan/`: `memory.rs` owns the allocation/pointer model
  (`GpuAllocation`/`GpuPtr`/`HostPtr` over the core `MemoryResidency`
  vocabulary), `sync.rs` the stage-to-barrier
  translation (`stage_to_vk`, deriving sync2 stage/access pairs from the core
  `Stage` vocabulary) plus the frame `Timeline` over crate-internal
  semaphores (and the test-only `TimestampQueryPool`), `pipeline.rs` both
  pipeline types, `view.rs` the
  `TextureView` wrapper, `image.rs` the `Image2d` creation helper (image +
  allocation + view in one call). Device-level capability flags
  (`Capabilities`) and the frame-retirement ring depth (`RETIRE_RING`) are
  core vocabulary; each backend reports its flags through
  `Device::capabilities()`.

## Object ownership and lifecycle

- Create, access, and destroy Vulkan objects on the main thread. GPU wrappers'
  `Send + Sync` bounds support ECS storage, not cross-thread driver operations
  (see [threading model](../../docs/architecture.md#threading-model)). Raw
  `Vk*`/`ash` handles never leave the backend module (see Boundary discipline).
- Every `unsafe` block carries a `// SAFETY:` comment arguing why it is sound
  (handle validity/lifetime, exclusivity, pointer bounds). A comment that
  cannot be written means the block needs a guard, not a waiver.
- Shared ownership, wgpu-style: the device's teardown-critical state lives in
  `DeviceShared` (device handle, allocator, retirement ring, extension
  loaders, instance keepalive), and every GPU object holds a cloneable
  crate-internal `DeviceContext` (`Arc<DeviceShared>`). The logical device is
  destroyed in `DeviceShared::drop`, when the last referent goes away — an
  object outliving its `Device` handle is safe by construction, so there are
  no leak guards and no caller-side drop-order contracts. `Surface` likewise
  holds an `Arc<InstanceShared>`.

## Shaders

- Slang compilation is target-agnostic and lives in `src/shader/`:
  `compile.rs` — `Compiler`/`CompiledShader`/`ShaderCache` with a
  `ShaderTarget` (`Spirv` for the backend; the other targets serve offline
  tooling);
  `reflection.rs` — the self-referential `Reflection` wrapper, `Layout`,
  `RootParam`, and the user-attribute types;
  `root_binder.rs` — the root-parameter vocabulary
  (`RootBinder`/`RootParamPlace`), turning reflection results into push-data
  blobs.
  `ShaderModule::from_spirv` loads SPIR-V bytecode directly and
  `ShaderModule::from_compiled` maps the shared stage onto Vulkan's stage
  flags.
- One offline Slang compile (`slangc -target spirv`) can also produce embedded
  shader bytes with `include_bytes!`.
- Native deps: **Slang** (`shader-slang-sys` links it dynamically — set
  `SLANG_DIR` or fall back to `VULKAN_SDK`; the shared library must be on the
  runtime library path when running tests), **libclang** (bindgen for
  `shader-slang-sys`).

## Smoke test

- `cargo test -p moonfield-rhi backend::vulkan::gpu_tests::headless_triangle` runs the
  headless Vulkan smoke test on machines with a compatible driver (the RHI
  requires `VK_EXT_descriptor_heap` plus the mesh/ray-tracing extensions in
  [`Device::new`]'s device table, so software renderers like lavapipe skip
  it). The GPU tests live in-crate under `backend/vulkan/gpu_tests/` (they verify
  `pub(crate)` internals) and skip gracefully when no such device is present.
