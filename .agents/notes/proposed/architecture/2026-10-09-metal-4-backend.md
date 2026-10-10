# Agent Note: Metal 4 backend

Status: proposed

[中文](2026-10-09-metal-4-backend.zh.md)

## Problem

The supported targets are Windows and Linux. macOS needs a native backend:
MoltenVK cannot expose the device table `Device::new` requires
(`VK_EXT_descriptor_heap`, mesh shading, ray tracing), so a Vulkan path on
macOS is not viable — and local development on Apple silicon currently has
no GPU path at all. The requirement for the platform: macOS defaults to the
latest Metal 4 (`MTLGPUFamily.Metal4`, the macOS 26 SDK), with a clear error
when the device does not qualify. The constraint: downstream crates consume
the concrete RHI types, so the backend must match the Vulkan backend's public
names and shapes instead of introducing an abstraction layer.

## Proposal

Land a `metal/` sub-crate (`moonfield-rhi-metal`, mirroring
`moonfield-rhi-vulkan`) implementing the same curated public type list, on
`objc2` + `objc2-metal` (which carries the `MTL4*` types). The facade gains a
`metal` feature and a `compile_error!` guard rejecting builds where both
backend features are enabled; consumer manifests switch from `vulkan` to
target-split entries (non-Apple targets keep `vulkan`, Apple targets take
`metal`).

The API mapping, backend by backend:

| Vulkan concept | Metal 4 concept |
|---|---|
| `VK_EXT_descriptor_heap` heap + `cmd_bind_*_heap` | `MTL4ArgumentTable` (one per command buffer; `SetAddress(buffer.gpuAddress + offset, stride, slot)`, bound to render encoders for vertex/fragment/object/mesh stages and to compute encoders) |
| Heap/resource residency | `MTLResidencySet` on the device, added to every `MTL4CommandQueue`; register/unregister allocations under a lock |
| `Fence`/`Semaphore` + retire ring | `MTLSharedEvent` (monotonic signaled values drive the retire ring) |
| `Stage`/`Access` barriers | Metal 4 `BarrierAfterEncoderStages` (stage-to-stage); texture layout transitions are no-ops |
| `TimestampQueryPool` | `MTLCounterSampleBuffer` + `Device.QueryTimestampFrequency` |
| Indirect draw/dispatch, GPU memcpy (`device_address_commands`) | device-address-based dispatch/copy on `MTL4CommandBuffer`/encoders |
| `Surface`/`Swapchain` (ash-window) | `CAMetalLayer` (via raw-window-handle), `NextDrawable`, present after `SignalDrawable` |
| gpu-allocator `vulkan` slice | gpu-allocator `metal` slice (same crate, same release), backing `GpuAllocation`/`GpuPtr`/`Memory` over `MTLBuffer.gpuAddress` |

Shaders compile from the same Slang sources: `slangc -target metallib
-capability metallib_latest -Xmetal -std=metal4.0` emits a `.metallib`
directly (no MSL round-trip), loaded through `Device.MakeLibrary`. The
`Compiler` grows a target parameter; `Reflection`→binding mapping stays
per-backend (the Vulkan `RootBinder` writes pointer bytes into the push blob;
the Metal side maps reflection to argument-table slots). Capability gating
replaces the hard extension table where the platforms differ: mesh shading
is `MTLGPUFamily.Apple7`+, ray tracing reports at runtime, and features the
device lacks degrade instead of failing device creation. The engine-level
Y-up reverse-Z clip convention adapts at the Metal boundary (Metal NDC is
z∈[0,1]), never in scene code. The editor's egui backend needs a Metal
sibling of `egui_vk.rs` before macOS can run the editor.

Sequencing, each phase shippable on its own:

1. `moonfield-rhi-metal` skeleton: MTL4 device gate (`SupportsFamily(Metal4)`
   → error otherwise), `MTL4CommandQueue` + residency set,
   `MTL4CommandBuffer` + argument table, `MTLSharedEvent` timeline, barriers,
   copies; offscreen-triangle smoke test under `metal/src/gpu_tests/`.
2. Slang `metallib` target in the `Compiler`, `ShaderModule::from_metallib`,
   reflection→argument-table slots.
3. `CAMetalLayer` swapchain + platform-selected `RenderDevice`, lighting up
   the Selene window frame loop on macOS.
4. Mesh shading pipelines, timestamps, uploader/bump equivalents; ml
   autodiff→`metallib` verified on its own.
5. egui Metal backend for the editor.

## Alternatives considered

**MoltenVK on macOS** would reuse the Vulkan backend untouched. It loses
because the RHI's device table requires `VK_EXT_descriptor_heap` and
mesh/ray-tracing extensions MoltenVK does not expose — the backend would
have to drop to a degraded feature set, which is exactly what the
capability-gated native backend avoids.

**A runtime trait layer before the Metal work** would make both backends
selectable at runtime and the backend choice a library concern. It loses for
the same reason recorded in
[the sub-crate note](../../implemented/architecture/2026-10-09-rhi-backend-subcrates.md):
it rewrites every downstream signature for a selection capability the
editor-only build graph does not need; same-name re-exports under exclusive
features deliver platform selection without the churn.

**Metal 3 as the baseline** would cover more machines (any macOS with Apple
silicon). It loses against the requirement that macOS defaults to the latest
Metal 4: argument tables, residency sets, and the new barrier model are the
Metal 4 surface that maps onto the RHI's bindless/sync vocabulary; targeting
Metal 3 would mean approximating those with argument buffers and
manual tracking, a different backend for the same amount of work.

## Acceptance criteria

- On an M-series Mac: `cargo run` builds `moonfield-editor` with the `metal`
  feature (no `ash` in the graph) and renders the editor viewport through the
  Selene frame loop.
- A device without `MTLGPUFamily.Metal4` produces the explicit
  "does not support Metal 4" error at device creation.
- Enabling both backend features fails the facade with `compile_error!`.
- Windows/Linux builds are unchanged: same public names, no new deps.
- `cargo test -p moonfield-rhi-metal` runs the offscreen smoke test on
  qualifying hardware and skips gracefully otherwise.
- The boundary gate (`verify_rhi_boundary.py`) passes with the metal sources
  scanned — no `objc2`/`MTL*` in any public signature.

## Risks

- **Slang's Metal target is officially experimental.** Vertex, fragment,
  compute, task, and mesh stages are listed as supported; the engine's own
  ray-query and autodiff kernels compiling to `metallib` is unverified until
  phase 2/4 prove it. Fallback: keep rt and ml feature-gated off on macOS
  until verified, as capability-gated features.
- **`ResourceDescriptorHeap[]` — the Vulkan bindless heap syntax — does not
  compile for the Metal target** (measured: "unavailable features in entry
  point ... for 'metal' compilation target"). The Metal bindless channel is
  the array-of-resources parameter: Slang emits
  `array<texture2d<float>, N>` / `array<sampler, N>` entry-point parameters
  plus the root blob (`EntryPointParams` at `[[buffer(0)]]`, measured).
  Shader sources that index `ResourceDescriptorHeap` need a Metal variant
  (the arrays are declared as `uniform Texture2D g_textures[N]` in Slang,
  one source with target-gated sections, or a small Metal-specific module);
  how the array parameter maps onto `MTL4ArgumentTable` slots
  (`setResource_atBufferIndex` / `setTexture_atIndex`) is the next measured
  step before the `DescriptorHeap` equivalent is designed.
- **`objc2-metal` MTL4 coverage** may lag the headers the backend needs;
  gaps get local `extern_class!` declarations until upstream ships them.
- **Argument-table slot budget** (Metal 4 caps buffer bind counts) differs
  from the descriptor heap's capacities; the reflection→slot mapping must
  budget slots explicitly or the bindless model strains on dense passes.
- **The egui port** is its own chunk of editor work; phases 1–4 deliver a
  headless/render-loop-capable RHI on macOS before the editor is usable.
