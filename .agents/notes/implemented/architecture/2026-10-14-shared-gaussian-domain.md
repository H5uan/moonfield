# Agent Note: Shared Gaussian domain crate

Status: implemented

[中文](2026-10-14-shared-gaussian-domain.zh.md)

## Problem

The [architecture contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)'s
fifth item: the trainer (`moonfield-ml`) imported `GaussianScene` from
`moonfield-render-feature`, dragging the app/render framework into a consumer
of plain CPU domain data. `GaussianScene`'s parallel public vectors could
disagree in length, and `pack_gaussians` silently truncated to the shortest
attribute. The glTF loader's `ln`/`logit` conversions could also mint
non-finite values from degenerate file data (a zero scale or a 0/1 opacity).

## Decision

**`moonfield-gaussian` owns the domain.** The new leaf crate (no framework
dependencies) carries `GaussianScene` and `SH_REST_LEN`; the renderer and the
trainer both consume it, and `moonfield-ml` drops its
`moonfield-render-feature` dependency entirely. The crate packaging follows
the dependency graph: the scene type is the only shared concept, so it gets
its own crate rather than a home inside either consumer.

**Validity is a construction invariant.** `GaussianScene`'s fields are
private; `new` and `push` validate equal attribute lengths and finite values
(`GaussianSceneError::{LengthMismatch, NonFinite}`), and accessors are
read-only. A mismatched scene is unrepresentable, so `pack_gaussians` packs
exactly `scene.len()` records — the truncation path is gone.

**The loader clamps before the space conversion.** `SCALE` clamps to a
positive minimum before `ln`, `OPACITY` into `[1e-6, 1 - 1e-6]` before
`logit`, so file data cannot produce the non-finite values the scene now
rejects; a scene-level failure surfaces as `SplatGltfError::Domain`.

## Alternatives considered

- **Keeping the type in render-feature and letting ml depend on it.** Lost:
  that is the coupling under repair — the trainer would keep compiling the
  app framework for a plain data type.
- **Public fields plus validation at pack time only.** Lost: construction
  sites are the cheapest place to reject (the glTF loader's error then names
  the file, not a GPU-side symptom), and public fields leave every future
  writer free to break the invariant.
- **Validating the representation conventions numerically** (log-space
  scales, logit opacities, (w, x, y, z) rotations). Dropped: the conventions
  are not numerically distinguishable from their render-space counterparts;
  they stay documented caller-upheld contracts.

## Consequences

- Renderer and trainer consume one validated interface; mismatched attribute
  arrays are rejected at `new`/`push` rather than truncated at pack.
- `moonfield-ml` compiles without the render framework; the editor and tests
  construct scenes through the validated constructor.
- The `0.282095·c + 0.5` SH bias and kernel conventions stay owned by the
  [splat forward decision](2026-10-14-splat-forward-pass.md); this note owns
  only the data's home and validity.
