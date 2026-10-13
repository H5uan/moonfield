# Agent Note: Scene save/load drop per-entity extras rebuild and double JSON parse

Status: implemented

[中文](2026-09-19-scene-save-load-typed-parse-and-extras-cache.zh.md)

## Problem

Two wasteful patterns sat in the moonfield-scene save/load paths:

- `save_scene` called `SceneRegistry::extras_entries` per entity (inside
  `has_registered_component`) and again per node (inside `save_node`,
  multiplied by recursion). Each call allocated a `Vec` of the extras-channel
  entries and re-sorted it — the registry's `HashMap` iteration order had to
  be re-normalized thousands of times per save.
- `parse_root` parsed the whole document into `serde_json::Value`, patched
  `"nodes": []` into scene objects that lack the key, then converted the
  `Value` into `gltf_json::Root` — two full parses of every scene file,
  existing only because `gltf-json`'s `Scene::nodes` has
  `skip_serializing_if` but no `#[serde(default)]`, so the empty scenes
  `save_scene` writes (`"scenes": [{}]`) fail a direct `Root` parse.

## Decision

- `save_scene` collects `registry.extras_entries()` once and passes the
  `&[(&str, SaveFn)]` slice down through `has_registered_component` and
  `save_node`; the per-entity check is now a slice scan with no allocation.
  The cache lives per save call, not in the registry: `SceneRegistry` is a
  world resource with public `&mut self` registration methods, so an internal
  cache would need invalidation on every mutation for no extra win.
- `parse_root` is a single typed pass through `RootFile`, a load-side mirror
  of `gltf_json::Root` whose `scenes` are `SceneFile` — a mirror of
  `gltf_json::Scene` with `#[serde(default)]` on `nodes`. The mirror is
  fielded out by hand rather than `#[serde(flatten)]`-wrapped around `Root`:
  flatten buffers the document through serde's private `Content`, which
  rejects the `Box<RawValue>` extras gltf-json stores on nodes.

## Alternatives considered

- **Caching the sorted extras list inside `SceneRegistry`** (e.g. a
  `OnceCell` invalidated by the `register*` methods). `save_scene` takes
  `&SceneRegistry`, so this needs interior mutability plus invalidation
  discipline; building one `Vec` per save call is already O(entries) once per
  save instead of per entity, which removes the actual waste.
- **`#[serde(flatten)]` on a `RootFile { scenes, root: Root }` wrapper.**
  Fails at runtime: serde's flatten buffering does not support
  `serde_json::value::RawValue`, and gltf-json's `Extras` is
  `Option<Box<RawValue>>`, so any node extras break the parse.
- **Try `Root::from_str` first, fall back to the `Value` patch on error.**
  Keeps two code paths and makes the empty-scene roundtrip — the case
  `save_scene` itself produces — permanently take the slow path.

## Consequences

- Saving costs one `extras_entries` collection per call; the sorted order
  (deterministic document output) is unchanged.
- Loading parses the document once. Unknown `extras.components` keys are
  still skipped, and documents that gltf-json would reject still fail with
  `SceneError::Json`.
- `RootFile`/`SceneFile` must track gltf-json's `Root`/`Scene` field lists;
  the dependency is workspace-pinned (`gltf-json = "1.4"`), and a field
  addition there silently drops data on load until the mirror is updated —
  the same trade gltf-json's own non-`deny_unknown_fields` structs make for
  unknown keys.
- The `moonfield-scene` dev-dependency on `moonfield-render-feature` enables
  the `mesh` feature: the crate does not compile with
  `default-features = false` (its plugin module imports the mesh-gated
  modules unconditionally), so isolated `cargo test -p moonfield-scene` runs
  rely on the dev-dependency declaring the feature its test code uses.
