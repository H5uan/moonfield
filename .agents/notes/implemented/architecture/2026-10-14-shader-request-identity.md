# Agent Note: Shader request identity and artifact generations

Status: implemented

[中文](2026-10-14-shader-request-identity.zh.md)

## Problem

The [architecture contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)'s
third item: `PreparedShaders::needs_prepare` compared only the pipeline name,
asset id, and asset revision. Replacing a request's entry points, reflection
entry, or capabilities for the same asset revision silently reused the old
artifacts, and pipelines keyed their rebuilds on the source revision, so a
shader change and a request change were indistinguishable from a no-op as far
as pipeline state was concerned.

## Decision

**Requests carry a fingerprint.** `PipelineShader::fingerprint()` hashes the
reflection entry and the entry set (names and capabilities) — everything in
the request that changes the compiled artifacts. `PreparedShaders` slots store
the fingerprint they were compiled from, and `needs_prepare` takes the whole
request: an asset change, a source revision advance, or a request change all
re-arm preparation.

**Pipelines observe artifact generations, not source revisions.**
`PreparedShaders` hands out a monotonically increasing `generation` on every
successful compile and never on a failed one. `PreparedShader::generation()`
replaces `revision()` in every consumer's staleness check (the core-3d and
splat passes, the egui pipeline). A failed recompile therefore cannot disturb
a running pipeline: no new generation is issued, `get` stops serving the
pipeline's artifacts, and the consumer keeps what it built from the previous
generation.

The `ShaderCacheKey` import-content gap the proposal names under hot reload
stays open — hot reload does not exist yet; import dependency invalidation
lands with it.

## Alternatives considered

- **Keying pipeline rebuilds on (revision, fingerprint) pairs.** Lost:
  consumers would each have to store and compare both, and a failed compile's
  slot advances its revision (to suppress per-frame retries), which reads as
  "new artifacts" to any consumer not also checking the result. One monotone
  counter is the single comparison with the keep-on-error rule folded in.
- **Hashing the source into the fingerprint.** Lost: the source already
  identities through the asset revision; hashing it again would couple
  request identity to content the asset store already tracks.

## Consequences

- Changing a shader request without touching the source triggers preparation
  and pipeline rebuilds; covered by tests in
  `moonfield-render-feature::shader`.
- A broken shader edit no longer reaches any pipeline state: the last usable
  pipeline runs until a compile succeeds and issues a new generation.
- `AssetRevision` is gone from pipeline identity everywhere; it remains the
  asset-store concept `needs_prepare` and extraction use.
