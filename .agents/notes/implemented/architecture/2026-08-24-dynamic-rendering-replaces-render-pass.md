# Agent Note: Dynamic rendering replaces render pass objects

Status: implemented

[中文](2026-08-24-dynamic-rendering-replaces-render-pass.zh.md)

## Problem

The RHI's graphics path was built around retained-mode objects —
`RenderPass` and `Framebuffer`. A render pass hard-coded one color
attachment and (optionally) a `D32Sfloat` depth attachment, declared
initial/final layouts and hand-written subpass dependencies, and every
begin call had to thread a `VkRenderPassBeginInfo` through. Vulkan 1.3
promoted dynamic rendering (`vkCmdBeginRendering`), which removes the
need for those objects entirely: attachments are passed inline per pass
with their load/store ops and clear values, and the pipeline names its
attachment *formats* instead of a compatible render pass. The device
already enabled `dynamicRendering` (and the 1.4 `dynamicRenderingLocalRead`).

Separately, blend/cull/depth state was baked into
`VkGraphicsPipelineCreateInfo` per permutation — exactly the PSO
permutation explosion Sebastian Aaltonen argues against in [No Graphics
API][no-gapi]. Vulkan 1.3 made most of that state dynamic
(`CmdSetColorBlend*`, `CmdSetCullMode`, `CmdSetDepthTest*`), and the
`VK_EXT_extended_dynamic_state3` extension adds the blend equation and
write mask. Following the blog's `gpuBeginRenderPass` shape, this change
replaces the retained render-pass objects with a flat per-pass
description and moves the raster state to per-draw dynamic commands.

[no-gapi]: https://www.sebastianaaltonen.com/blog/no-graphics-api

## Decision

`RenderPass` and `Framebuffer` are deleted; no `VkRenderPass` or
`VkFramebuffer` is created anywhere in the tree.

- **`RenderPassDesc` + `RenderAttachment`** replace both objects.
  `CommandBuffer::begin_rendering(&RenderPassDesc)` builds a `VkRenderingInfo`
  inline: color attachments (and optional depth) with image view, layout,
  load/store op, and clear value. `image_layout` is both the rendering- and
  the final layout — dynamic rendering transitions automatically, so the
  old `SHADER_READ_ONLY_OPTIMAL` external subpass-dependency hack is gone;
  swapchain passes use `PRESENT_SRC_KHR`, offscreen passes use
  `SHADER_READ_ONLY_OPTIMAL`.
- **Rasterizer state is fully dynamic.** `CullState` and `DepthState` are
  set per draw via `set_cull_state` / `set_depth_state` (Vulkan 1.3 core)
  and `set_blend_state` (VK_EXT_extended_dynamic_state3, loaded once on
  the device). `begin_rendering` resets all dynamic state to defaults
  (blend off, back-face culling, depth off, viewport/scissor = render
  area) so a pass never inherits stale state — the no_gfx_api
  beginRenderPass convention.
- **`GraphicsPipeline`** takes `color_formats: &[Format]` and
  `depth_format: Option<Format>` instead of a `&RenderPass`, feeding
  `VkPipelineRenderingCreateInfo` (pNext on the pipeline create info,
  `render_pass = VK_NULL_HANDLE`). `PipelineOptions` keeps only
  `set_layouts`; `blend`, `cull_mode`, and `depth_test` are gone.
- **`Format`** gains `D32Sfloat` so the RHI-neutral format enum can name
  the depth attachment.

Callers (editor viewport, egui backend, window renderer, all tests) build
a `RenderPassDesc` per scene from the target's image views; the viewport's
depth pass sets `DepthState { test: true, write: true, GREATER_OR_EQUAL }`
(reverse-Z) and egui sets `BlendMode::PremultipliedAlpha` before drawing.

## Alternatives considered

- **Keep `vk::RenderingInfo` in the public API.** Rejected: the point of
  the refactor was to retire raw Vulkan pass types behind the
  `RenderAttachment`/`RenderPassDesc` boundary, matching the blog's
  descriptor shape and leaving room for MRT (an array of color
  attachments).
- **Keep blend/cull/depth baked in the pipeline.** Rejected: it preserves
  the PSO permutation explosion the blog identifies; making them dynamic
  is what allows one pipeline per shader pair. The only extra cost is a
  few `CmdSet*` calls per draw, which are cheap on modern drivers.
- **Speed between a `cmd_set_*` command per state vs a struct.** Adopted
  the struct (`CullState`, `DepthState`) — the no_gfx_api `cmd_set_depth_state`
  style — because the calls stay callers readable and one struct name
  implies all fields are covered.

## Consequences

- `VK_EXT_extended_dynamic_state3` is a required device extension and its
  feature struct is enabled at device creation; the blend `CmdSet*`
  commands otherwise fail validation. Depth/cull dynamic state is Vulkan
  1.3 core, so no extra feature is needed for those.
- Pipelines now specify formats at creation; a pipeline must be recreated
  if the target format changes (it already was, via render-pass
  compatibility — the dependency just moved).
- All subpass/external-dependency logic is gone: dynamic rendering has no
  subpasses, and the final-layout transition is implicit. Renderers that
  need a layout change after the pass (e.g. offscreen → sampler) express
  it through the same `image_layout` field.
- The editor's egui backend now records blend state as a dynamic command;
  the premultiplied-alpha equation is encoded in `set_blend_state` and
  shared between window and offscreen targets.