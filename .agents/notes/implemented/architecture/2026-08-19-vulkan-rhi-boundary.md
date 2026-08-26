# Agent Note: Vulkan RHI boundary

Status: implemented

[中文](2026-08-19-vulkan-rhi-boundary.zh.md)

## Problem

The renderer must let scene and renderer crates express their work without
depending on `ash` types, while the Vulkan backend keeps all driver calls
inside one crate. Two conventions make the seam load-bearing: the engine's clip
space is Y-up with reverse-Z (Vulkan is Y-down), and callers must never need to
know where the viewport flip happens. Without an explicit boundary, raw `Vk*`
handles and coordinate flips leak into scene code and become unfixable later.

## Decision

`moonfield-rhi` is the only crate that links `ash`, and all Vulkan-specific
code lives in `src/vulkan/` (device, swapchain, pipeline, command, sync,
offscreen, texture, shader). The surface it exposes is its own
vocabulary:

- Public resource descriptions — `Format`, `BufferUsage`, `VertexBufferLayout` —
  are declared in `src/types.rs`, never raw `ash` types. The pass-recording
  surface follows the same rule: `RenderAttachment`/`RenderPassDesc`,
  `LoadOp`/`StoreOp`/`ClearValue`/`AttachmentLayout`, `Viewport`/`Rect2d`/
  `Extent2d`, `CompareOp`/`CullMode`/`FrontFace`, `ShaderStages`/
  `PushConstantRange`, `CommandBufferUsage`, and `SamplerDesc` are crate
  vocabulary, so feature crates and the editor record passes without linking
  `ash` (raw handles remain available through `.raw()`/`.raw_vk()` escape
  hatches and the compute/bindless/indirect command family).
- `Texture` (sampled image + upload) and `OffscreenTarget::read_pixels` cover
  texture upload and readback; `Device::submit_and_wait` covers blocking
  one-shot submission outside the window frame loop.
- The engine clip convention is **Y-up with reverse-Z**; any Vulkan viewport
  adjustment happens at this boundary (`vulkan::*`), not in scene or renderer
  code.
- All Vulkan objects live on the main thread; nothing is `Send` across threads
  yet. Objects are destroyed in reverse creation order with explicit drop order
  (render-world resources drop LIFO — see
  [no renderer objects](2026-08-25-no-renderer-objects.md)).
- Shaders: the backend compiles Slang→SPIR-V at runtime
  (`vulkan/shader.rs`), `ShaderModule::from_spirv` loads bytecode directly, and
  one offline `slangc -target spirv` compile can also produce embedded bytes via
  `include_bytes!`.
- `cargo test -p moonfield-rhi --test headless_triangle` runs headless on
  lavapipe; on Windows/macOS it skips when no Vulkan driver is present.

## Alternatives considered

- **Expose raw `ash` types across crates.** Rejected: every consumer would then
  depend on `ash` and on Vulkan lifetime rules; `types.rs` keeps the surface
  testable and replaceable.
- **Wrap every Vulkan object in a full object model.** Rejected: a per-object
  abstraction layer adds hierarchy without extra safety; only the resource
  description vocabulary is exported, everything else stays behind `vulkan/`.
- **Make clip space Vulkan-native (Y-down).** Rejected: the engine's math layer
  (reverse-Z, Y-up) matches the camera/rendering code; adjusting the viewport in
  one place at the boundary is cheaper than flipping the convention everywhere.
- **Offline-compile every shader.** Rejected: runtime compilation is needed for
  iteration and for letting the backend own the toolchain; offline embeds stay
  available for shipping.

## Consequences

- Switching the backend (or testing without a GPU) happens behind `types.rs`,
  no scene code changes.
- Single-threaded ownership is simple and safe today but means GPU work and ECS
  updates cannot overlap; that is deferred until a command-queue handoff lands.
- `shader-slang-sys` needs Slang at build and runtime (`SLANG_DIR` or
  `VULKAN_SDK`); CI's `setup-slang` action pins the release.