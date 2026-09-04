# moonfield-rhi — Vulkan RHI rules

Lunar Mare, the Vulkan-only rendering RHI (ash). See the root
[AGENTS.md](../../AGENTS.md) and [crates/AGENTS.md](../AGENTS.md) for standing
rules; this file adds what is specific to a Vulkan backend. The engine layer
(extraction, `ExtractedView`/`ViewTarget`, window frame loop, `RenderPlugin`)
lives in `moonfield-render-core` (Selene), never here.

## Boundary discipline

- All `ash` types and Vulkan calls stay inside `src/vulkan/`. The engine-level
  clip convention is Y-up with reverse-Z; any Vulkan viewport adjustment is made
  at this boundary, never in scene code.
- Public resource descriptions (`Format`, `BufferUsage`, `VertexBufferLayout`)
  live in `src/types.rs` as the crate's own vocabulary, not raw `ash` types.
- Module map inside `src/vulkan/`: `memory.rs` owns the allocation/pointer
  model (`GpuAllocation`/`GpuPtr`/`HostPtr`/`Memory`), `sync.rs` the barrier
  vocabulary (`Stage`/`BarrierHazard`) plus fences/semaphores, `pipeline.rs`
  both pipeline types, `view.rs` the `TextureView` wrapper.

## Object ownership and lifecycle

- All Vulkan objects live on the main thread; nothing is `Send` across threads
  yet. Do not leak `Vk*` handles through public APIs.
- Devices, descriptor heaps, pipelines, and swapchains are owned by the
  renderer and destroyed in reverse creation order; keep drop order explicit.

## Shaders

- Runtime Slang→SPIR-V compilation is provided by `vulkan/shader.rs`;
  `ShaderModule::from_spirv` loads SPIR-V bytecode directly.
- One offline Slang compile (`slangc -target spirv`) can also produce embedded
  shader bytes with `include_bytes!`.
- Native deps: **Slang** (`shader-slang-sys` links it dynamically — set
  `SLANG_DIR` or fall back to `VULKAN_SDK`; the shared library must be on the
  runtime library path when running tests), **libclang** (bindgen for
  `shader-slang-sys`).

## Smoke test

- `cargo test -p moonfield-rhi --test headless_triangle` runs the headless
  Vulkan smoke test on machines with a compatible driver (the RHI requires
  `VK_EXT_descriptor_heap` plus the mesh/ray-tracing extensions in
  [`Device::new`]'s device table, so software renderers like lavapipe skip
  it); tests skip gracefully when no such device is present.