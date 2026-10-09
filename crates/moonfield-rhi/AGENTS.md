# moonfield-rhi — RHI rules

Lunar Mare, the rendering RHI. See the root
[AGENTS.md](../../AGENTS.md) and [crates/AGENTS.md](../AGENTS.md) for standing
rules; this file adds what is specific to the RHI. The engine layer
(extraction, `ExtractedView`/`ViewTarget`, window frame loop, `RenderPlugin`)
lives in `moonfield-render-core` (Selene), never here.

## Crate layout

`moonfield-rhi` is a facade over backend sub-crates that live under its own
directory:

- `src/` — the facade. The re-export lists in `src/lib.rs` are the RHI's
  entire public surface; anything not re-exported there is not public API.
- `core/` — `moonfield-rhi-core`, the backend-agnostic vocabulary
  (`types.rs`, `error.rs`, `indirect.rs`). Nothing in `core` may mention a
  backend.
- `vulkan/` — `moonfield-rhi-vulkan`, the Vulkan backend (`ash`).
- `metal/` — `moonfield-rhi-metal`, the Metal 4 backend (`objc2-metal`).

Dependency direction: `moonfield-rhi` → backend sub-crates → `core`. The
backend is selected by cargo feature; exactly one backend feature is active
in a build, and the editor is the selection point. The workspace dependency
carries `default-features = false`, so the editor and every crate that uses
backend types enables the feature explicitly.

## Boundary discipline

- All `ash` types and Vulkan calls stay inside `vulkan/src/`. The engine-level
  clip convention is Y-up with reverse-Z; any Vulkan viewport adjustment is made
  at this boundary, never in scene code.
- Public resource descriptions (`Format`, `BufferUsage`, `VertexBufferLayout`)
  live in `core/src/types.rs` as the crate's own vocabulary, not raw `ash`
  types. Backend mappings of the vocabulary live in the backend
  (`vulkan/src/formats.rs`, the `ToVk` extension trait).
- **The public API exposes no backend types — no `ash`/`vk::`/`gpu_allocator`/
  `objc2`/`MTL*` in any public signature or trait impl, across the facade,
  `core`, and every backend sub-crate.** New capabilities must be added as
  first-class APIs in the crate's vocabulary; there are no `raw()` escape
  hatches. `scripts/verify_rhi_boundary.py` (CI `rhi-boundary` job) enforces
  this mechanically.
- `gpu-allocator` is the allocation substrate shared by backends; each
  backend sub-crate declares its own feature slice (the Vulkan backend:
  `vulkan`) and keeps allocator types out of the public API.
- Module map inside `vulkan/src/`: `memory.rs` owns the allocation/pointer
  model (`GpuAllocation`/`GpuPtr`/`HostPtr`/`Memory`), `sync.rs` the barrier
  vocabulary (`Stage`/`Access`) plus fences/semaphores and the
  `TimestampQueryPool`, `pipeline.rs` both pipeline types, `view.rs` the
  `TextureView` wrapper, `image.rs` the `Image2d` creation helper (image +
  allocation + view in one call).

## Object ownership and lifecycle

- All Vulkan objects live on the main thread; nothing is `Send` across threads
  yet. Raw `Vk*`/`ash` handles never leave the backend sub-crate (see Boundary
  discipline).
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

- Runtime Slang→SPIR-V compilation is provided by the `vulkan/src/shader/`
  module (`compile.rs` — `Compiler`/`CompiledShader`/`ShaderCache`,
  `reflection.rs` — the self-referential `Reflection` wrapper and `Layout`,
  `root_binder.rs` — `RootParam*`/`RootBinder`); `ShaderModule::from_spirv`
  loads SPIR-V bytecode directly.
- One offline Slang compile (`slangc -target spirv`) can also produce embedded
  shader bytes with `include_bytes!`.
- Native deps: **Slang** (`shader-slang-sys` links it dynamically — set
  `SLANG_DIR` or fall back to `VULKAN_SDK`; the shared library must be on the
  runtime library path when running tests), **libclang** (bindgen for
  `shader-slang-sys`).

## Smoke test

- `cargo test -p moonfield-rhi-vulkan gpu_tests::headless_triangle` runs the
  headless Vulkan smoke test on machines with a compatible driver (the RHI
  requires `VK_EXT_descriptor_heap` plus the mesh/ray-tracing extensions in
  [`Device::new`]'s device table, so software renderers like lavapipe skip
  it). The GPU tests live in-crate under `vulkan/src/gpu_tests/` (they verify
  `pub(crate)` internals) and skip gracefully when no such device is present.