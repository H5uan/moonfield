# Agent Note: Unified image layouts — GENERAL everywhere

Status: implemented

[中文](2026-08-26-unified-image-layouts.zh.md)

## Problem

The RHI tracked a per-use image layout for every image. `AttachmentLayout`
mapped to `PRESENT_SRC_KHR`, `SHADER_READ_ONLY_OPTIMAL`, and
`DEPTH_STENCIL_ATTACHMENT_OPTIMAL`; uploads and readbacks transitioned through
`TRANSFER_DST_OPTIMAL` / `TRANSFER_SRC_OPTIMAL`; the descriptor writer
hardcoded `SHADER_READ_ONLY_OPTIMAL`. Every transition was another chance for
the actual layout to diverge from the layout declared in a descriptor set or a
render-pass attachment — the classic validation-level hazard — and the
engine had to babysit a layout state machine it never wanted to own.

## Decision

- Every internal image (`Texture`, `OffscreenTarget` color and depth) lives
  in `VK_IMAGE_LAYOUT_GENERAL` for its whole lifetime. Barriers carry only
  stage/access synchronization; their `old_layout`/`new_layout` are both
  `GENERAL`.
- `AttachmentLayout::to_vk` maps `ShaderRead` and `DepthStencil` to `GENERAL`;
  `Present` keeps `PRESENT_SRC_KHR` — presentation is one of the scenarios the
  unified-layout guarantee explicitly does not cover.
- The image-creation `UNDEFINED` initial layout is kept, and the first
  barrier stays `UNDEFINED -> GENERAL`: initialization is the other explicit
  exception, and it is what makes a freshly created image's content defined.
- Descriptor writes use `GENERAL`, so the declared layout always equals the
  layout the command buffer actually executes in.
- `VK_KHR_unified_image_layouts` is enabled opportunistically. Device creation
  probes `PhysicalDeviceUnifiedImageLayoutsFeaturesKHR` via
  `vkGetPhysicalDeviceFeatures2`; when supported, the extension name and the
  feature (with `unifiedImageLayouts` set) are added to device creation.
  When absent, nothing is gated — `GENERAL` is valid without the extension and
  the RHI ships exactly one code path.

## Alternatives considered

- **Full unification including the swapchain (`Present -> GENERAL`)**: rejected
  — presentation layout is an explicit exception in the extension, and keeping
  `PRESENT_SRC_KHR` costs nothing since the swapchain already requires one
  transition per frame anyway.
- **Keep per-use layouts and just enable the extension**: rejected — the point
  of the change is to delete the layout state machine; the extension's
  efficiency guarantee only matters once every internal use actually goes
  through `GENERAL`.
- **Hard-require the extension like `VK_EXT_descriptor_heap`**: rejected —
  CI creates a real device for `headless_triangle` on lavapipe, and probing
  with an optional enable is five lines that keep CI green without a
  probe-and-skip test helper. Unlike the descriptor heap, here the layout code
  is correct even when the driver lacks the extension.

## Consequences

- The RHI performs no layout transitions after creation except the
  `UNDEFINED -> GENERAL` initialization and the swapchain present path.
- Descriptor/render-pass/command layout mismatches are impossible by
  construction: every internal image is `GENERAL` everywhere.
- On drivers exposing `VK_KHR_unified_image_layouts`, `GENERAL` is a driver
  guarantee of efficiency for nearly all uses; without it the code is still
  correct, only potentially non-optimal on some hardware.
- Standard validation can no longer catch layout mismatches (nothing to
  mismatch); Synchronization Validation — already the regime the RHI runs
  under via synchronization2 — is the safety net.
- Verified locally: `headless_triangle` and `egui_headless` pass on the
  editing machine's driver; `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo fmt --check` are clean.