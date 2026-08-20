# Agent Note: In-house egui→Vulkan backend

Status: implemented

[中文](2026-08-20-egui-vulkan-backend.zh.md)

## Problem

The editor's UI rendering depended on `egui-ash-renderer`, whose
compatibility table anchored the whole egui stack (egui / egui-winit /
egui_dock / ash / winit had to bump together with it), kept UI resources in a
separate allocator world from Lunar Mare's, and left rendering behavior in
third-party hands.

## Decision

`moonfield-editor::egui_vk` is the editor's egui backend: an `EguiRenderer`
built on Lunar Mare (`moonfield-render`), shaders written in Slang. The
feature spec is egui-wgpu 0.33, ported to Vulkan idioms:

- API: `update_texture` (full + partial `ImageDelta`), `free_texture`
  (user textures only lose their descriptor set), `register_native_texture` /
  `register_native_texture_with_options` / `update_native_texture` /
  `update_native_texture_with_options` (external image → `TextureId::User`,
  with id-stable rebinds for resizable targets), `texture` lookup,
  `update_buffers`, `render` (records into the caller's open render pass).
- Textures: one `R8G8B8A8_UNORM` image per managed `TextureId`, samplers
  cached by `TextureOptions`, no mipmaps, no atlas packing.
- Pipeline: premultiplied-alpha blending, 20-byte vertices (f32×2 pos,
  f32×2 uv, packed u32 sRGB color), a screen-size uniform, scissor from
  clip rect × pixels_per_point, u32 indices, grow-only doubling vertex/index
  buffers per frame-in-flight slot, texture frees deferred past the slot's
  fence (the editor's free ring).
- Shader options: dithering (interleaved gradient noise, default on) and
  predictable texture filtering (manual bilinear, default off); two fragment
  entry points cover gamma (unorm) and sRGB targets.

RHI support landed as `PipelineOptions` (blend mode, cull mode, descriptor
set layouts) on `GraphicsPipeline`, a `Uint32` vertex format, scissor and
descriptor-set binding on `CommandBuffer`, buffer bindings typed by their
layout entry in `bind.rs`, and `Buffer::read` plus `OffscreenTarget` readback
support for tests.

Explicitly not supported: `msaa_samples`, `depth_stencil_format`,
`CallbackTrait` paint callbacks, multiple viewports. The callback seam is
reserved (`render` records into the caller's pass; `callback_resources` is
the reserved shared-state bag) so callbacks can land without an API break.

## Alternatives considered

- **Keep egui-ash-renderer.** Rejected: the version anchoring and the
  separate allocator world were the problems to solve.
- **Fork egui-ash-renderer into the tree.** Rejected: inheriting its
  internals would inherit its structure; writing against Lunar Mare keeps UI
  resources in the same RHI as the scene and exercises the RHI's own gaps
  (pipeline options, descriptor binding), which had to be closed anyway.
- **Add wgpu as an intermediate layer.** Rejected: a second GPU API stack on
  top of the Vulkan one buys nothing once the renderer is in-house.

## Consequences

- The egui stack's version anchor is egui_dock's compatibility table; UI
  rendering upgrades track egui_dock alone.
- The viewport keeps its offscreen-target + user-texture architecture; the
  offscreen image registers through `register_native_texture` and rebinds in
  place on resize via `update_native_texture`.
- `cargo test -p moonfield-editor --test egui_headless` renders one egui
  frame headlessly (text, user texture, clip rects) and reads the pixels
  back; it runs on lavapipe in CI and skips where no Vulkan driver exists.
- Visual verification of the dock panels and viewport remains a manual check
  at editor startup.
