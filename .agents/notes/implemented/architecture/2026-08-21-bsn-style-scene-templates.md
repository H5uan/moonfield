# Agent Note: BSN-style scene templates with a glTF carrier

Status: implemented

[中文](2026-08-21-bsn-style-scene-templates.zh.md)

## Problem

The workspace had no scene save/load: hierarchy, `Transform`, `Camera`, and
components lived in the `World` with no serialization path, and asset loading
was caller-side synchronous with no dedup — the editor parsed PLY files
directly, one parse per load. The [reference-follow
roadmap](2026-08-19-ecs-driven-infrastructure-roadmap.md) points this layer
at the vendored 0.20-dev tree (`target/bevy-src`), whose BSN direction
replaced the deleted `DynamicScene`/RON system with typed templates and a
two-phase scene→resolved→apply pipeline. But 0.20 has no runtime text format
and no save direction yet, so following BSN still leaves the file-format
layer to us.

## Decision

The system is a synchronous miniature of BSN — typed templates, two-phase
apply, zero runtime reflection — with a glTF 2.0 JSON carrier of our own on
top. Mechanisms live in [docs/architecture.md](../../../../docs/architecture.md).

- `moonfield-ecs` gains the typed half: a `Template` trait (plain data that
  builds its `Output` in a `TemplateContext { world: &mut World }`), a
  blanket impl making every `Clone` type its own template, and
  `TemplateError`; `World::iter_entities()` enumerates entities for the save
  side.
- `moonfield-asset` gains a synchronous `AssetServer` world resource:
  `AssetLoader` implementors (`Send + Sync`, which the blanket `Resource`
  impl requires) dispatch by file extension, and a `(TypeId, PathBuf)` path
  cache serves repeat loads, reloading when the cached slot went stale. The
  crate stays zero-dependency and async-free.
- The new `moonfield-scene` crate carries the scene half:
  `HandleTemplate<T>` (a path or a resolved handle), the type-erased
  `SceneTemplate` (blanket over `Template` with `Component` output),
  `ResolvedScene` (one entity's templates plus child subtrees; `apply`
  spawns and links `ChildOf`), and `SceneRegistry` — stable short names
  (`"transform"`, never Rust type paths) mapped to native glTF entries
  (transform/camera/hierarchy/name) or extras-channel entries (generic serde
  or custom hooks). `save_scene`/`load_scene` map the world onto a glTF 2.0
  JSON document via `gltf-json`; `SceneError` wraps the failure modes.
- `moonfield-render` adds serde derive on `scene::MeshRenderer` only — the
  one component that crosses the extras channel as plain data today.
- The editor wires it: `SplatCloudLoader`, the `editor_asset_server()` /
  `editor_scene_registry()` resources, `load_splat_cloud` routed through the
  server (path-deduped), and a Scene path field with Save/Load buttons
  (`SceneIoState`) in the Hierarchy panel.

Skipped on purpose, against the vendored 0.20-dev source: the `bsn!`
proc-macro DSL, `ScenePatch` caching, `QueuedScenes`/`WaitingScenes`
(async-only), the `BundleWriter` bump arena, named entity references, and
full glTF mesh/material import.

## Alternatives considered

- **Port the old `DynamicScene`/RON reflection system.** Rejected: it is
  deleted upstream — the 0.20-dev tree we follow has already moved past it —
  and it needs runtime reflection (type registry, `DynamicStruct`) that this
  workspace deliberately does not have; the mini reflection in
  `moonfield-reflect` is editor-inspection only.
- **RON as the text carrier on top of typed templates.** Rejected: RON would
  still need per-component serde plumbing while buying nothing over JSON for
  interchange — no DCC or external tool reads it.
- **USD.** Rejected as disproportionate: a composition engine with layers,
  references, and variants is far more machinery than scene save/load needs,
  and its dependency weight dwarfs the miniature.
- **Plain JSON with our own schema.** Rejected as the carrier: same serde
  cost, but the hierarchy/TRS/camera mapping would be our own invention with
  zero interop. glTF maps all three natively, DCC interop and future mesh
  import come free, and `serde_json` still serves as the extras-channel
  encoding inside `node.extras`.

## Consequences

- The file format is the `SceneRegistry`'s public contract: stable short
  names, never Rust type paths, so component renames do not break files.
  Unknown `extras.components` keys skip on load rather than error, so a scene
  written by a newer registry still loads.
- Output is a valid glTF 2.0 document an external DCC can open. Lossy edges
  are explicit: matrix-form nodes load without a `Transform`, orthographic
  cameras load without a `Camera`, and `Camera::clear_color` rides
  `extras.camera` because glTF has no field for it.
- Handle components appear in files as plain path strings; loading a scene
  resolves them through the `AssetServer`, so reloading into the same world
  reuses cached asset slots instead of re-parsing.
- `GlobalTransform` is never registered or saved — the hierarchy propagation
  systems recompute it after load.
- Everything blocks the calling thread: no async queue, no hot reload, no
  background loading. Right for the editor today; a known debt if scenes grow.
