# Agent Note: Metal 4 backend

Status: rejected — abandoned by the [RHI Vulkan pivot](../../implemented/architecture/2026-10-13-rhi-vulkan-pivot.md)

**Shelved:** the Metal 4 backend is removed in favor of the Vulkan pivot —
see
[the Vulkan pivot note](../../implemented/architecture/2026-10-13-rhi-vulkan-pivot.md).

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

**Deferral (2026-10-11):** the bindless channel (phase 2) and the egui port
(phase 5) are on hold. The engine keeps the push-data + descriptor-heap
resource model — the direction DX12 and Vulkan both support — and migrating
root structs to the `DescriptorHandle` shape Metal requires
(argument-table slots, no root-blob bytes; measured below) would split the
binder and the engine's root pushes per backend. The alternative
`ConstantBuffer<T>` channel (measured: Metal lowers its handle fields to
CPU-writable inline `gpuResourceID`/`gpuAddress` u64s in the bound buffer —
the shape Zenith.NET rides) unifies the write path but gives up push data
for per-draw roots, the opposite trade. Phases 1–4's landed work stays: the
Metal backend compiles, `render-core`/`render-feature`/`ml` compile and pass
on Apple Silicon (see the
[frame-loop note](../../implemented/architecture/2026-10-11-metal-frame-loop-device-surface.md)),
and the editor's Vulkan path is unchanged. The editor does not build on
macOS until this is revisited — the trigger is a Metal surface that carries
the push-data model's semantics, or a decision to move root delivery to
bound constant buffers.

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
  point ... for 'metal' compilation target"). The cross-target channel is
  Slang's built-in `DescriptorHandle<T>` as fields in the root struct
  (measured, both targets):
  - **SPIR-V**: the field is a `uint2` in the root blob (8 bytes at the
    reflected offset); the heap slot index is the low `u32` (`.x`). A
    runtime `DescriptorHandle<T>(uint2)` construction is available.
  - **Metal**: the field is *not* root-blob bytes. The `EntryPointParams`
    blob skips handle fields (the uniform offsets pack without them), and
    reflection reports them as argument-table resource bindings
    (`shaderResource` / `samplerState` indices). Runtime construction from
    an index is unavailable on Metal (the constructor's target-family gate
    excludes it), so the handle must be a root field, bound API-side.
  - The API-side binding is `MTL4ArgumentTable::setTexture_atIndex` /
    `setSamplerState_atIndex`, which take an `MTLResourceID` — the
    resource's `gpuResourceID`, a plain `u64`. Metal's
    `TextureHandle`/`SamplerHandle` are gpuResourceID-backed values; the
    engine binds them at the reflected table indices.
  - Consequence for the engine: the root layout diverges per backend (blob
    offsets on Vulkan, table slots on Metal), so whole-struct root mirrors
    stop being cross-backend — per-draw root pushes go through
    `RootBinder`-resolved places, and the binder grows texture/sampler
    placements (uniform offset on Vulkan, table slot on Metal). Shader
    sources that index `ResourceDescriptorHeap` (egui.slang, core_3d.slang)
    migrate to `DescriptorHandle<T>` fields in their root structs.
- **`objc2-metal` MTL4 coverage** may lag the headers the backend needs;
  gaps get local `extern_class!` declarations until upstream ships them.
- **Argument-table slot budget** (Metal 4 caps buffer bind counts) differs
  from the descriptor heap's capacities; the reflection→slot mapping must
  budget slots explicitly or the bindless model strains on dense passes.
- **The egui port** is its own chunk of editor work; phases 1–4 deliver a
  headless/render-loop-capable RHI on macOS before the editor is usable.
