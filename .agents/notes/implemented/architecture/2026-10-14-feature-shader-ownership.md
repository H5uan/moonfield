# Agent Note: Feature shader ownership and composition validation

Status: implemented

[中文](2026-10-14-feature-shader-ownership.zh.md)

## Problem

The [architecture contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)'s
fourth item: `EditorPlugin` loaded and registered every pipeline's shader
requests (`core_3d`, the splat trio, and egui alike), so rendering without
the editor meant reproducing the editor's initialization. And
`RenderFeaturePlugin` inserted its asset stores with plain `insert_resource`
defaults, silently replacing stores the application had preconfigured.

## Decision

**Features declare and load their own shaders.** `load_feature_shaders` (in
`render-feature`'s shader module) is the shared startup path: it reuses the
application's `AssetServer` and `Assets<Shader>` when present (creating a
minimal `.slang`-only server and an empty store otherwise), loads each
feature shader from `<assets>/shaders/`, and pushes the pipeline request.
`RenderFeaturePlugin` registers a `Startup` system for `core_3d.slang`;
`SplatForwardPlugin` registers one for `radix_sort.slang`, `gaussian.slang`,
and `gs/composite.slang`. The editor keeps exactly one request of its own:
`egui.slang` (`load_editor_shaders`).

**Plugin initialization never replaces configuration.** `World` and `App`
gained `init_resource` (insert the default only when absent); both feature
plugins initialize their asset stores, request lists, and prepared-artifact
resources through it, so application-provided stores survive composition.

**Composition is validated after all plugins build.**
`RenderFeaturePlugin::finish` panics with the remedy when the render world's
`Render` schedule lacks the `PrepareAssets`/`Queue`/`PrepareViews`/
`CameraDriver` anchors (`RenderPlugin` missing); `SplatForwardPlugin::finish`
does the same for the `Core3d` schedule's `Core3dOpaquePass` anchor
(`RenderFeaturePlugin` missing). `Schedule::has_set` is the new query behind
both.

## Alternatives considered

- **Loading feature shaders at plugin build time.** Lost: the application's
  `AssetServer` is often inserted after the plugin builds (the editor inserts
  its own during `EditorPlugin::build`), so build-time loading would create a
  competing server instead of reusing the configured one. `Startup` runs once
  all plugins are built and still precedes the first extraction.
- **Warning instead of panicking on missing anchors.** Lost: the failure mode
  is systems running in registration order with no anchors — silent
  misordering of GPU work. A panic at `finish` names the missing plugin and
  the fix; a warning reads as noise.
- **A shader-path configuration resource.** Deferred: the shader directory
  already resolves through `moonfield_asset::assets_dir` (with its
  `MOONFIELD_ASSETS_DIR` override), and an application can push its own
  requests before startup. No consumer needs per-pipeline path configuration
  yet.

## Consequences

- A minimal app (`RenderPlugin` + `RenderFeaturePlugin`, optionally
  `SplatForwardPlugin`) renders meshes and splats without the editor; the
  editor's role narrows to egui and scene I/O.
- `insert_resource` keeps its replace semantics; `init_resource` is the
  composition-safe form, and plugin authors choose per resource.
- Tests pin the contract: store preservation and startup shader
  self-registration in `render-feature`'s plugin tests, the anchor panics as
  `should_panic` tests, and the editor's reduction to egui-only loading in
  `moonfield-editor`.
