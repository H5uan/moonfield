# Agent Note: RHI-owned interface and private backend modules

Status: implemented

[中文](2026-10-10-rhi-interface-and-backend-layering.zh.md)

## Problem

The [backend sub-crate decision](2026-10-09-rhi-backend-subcrates.md)
isolated backend dependencies and kept downstream consumers on concrete RHI
types. The facade selected backend re-export lists, but did not define a
common interface that both implementations had to satisfy. The Vulkan and
Metal lists differed, and matching names did not guarantee matching behavior
or signatures.

`Memory` illustrated the mismatch: in Vulkan it was an allocation class
(`Default`, `Gpu`, `Readback`); in Metal it owned a buffer. `GpuAllocation`
constructors, command-pool allocation methods, and command-buffer `end`
return types also differed. Upper layers could not switch backends by
changing the facade feature alone.

Backend knowledge also reached consumers: the render, editor, and ml
manifests enabled `vulkan`, and shader preparation chose
`ShaderTarget::Spirv`. Moving implementation files between crates and
modules does not resolve these contracts. Interface ownership and package
organization need separate decisions.

## Decision

`moonfield-rhi` owns the public interface, and backend selection lives
inside its implementation:

- One crate carries the public interface: the vocabulary modules
  (`types.rs`, `error.rs`, `indirect.rs`, `shader/` — resource descriptions,
  memory classes, capabilities, the shared Slang compiler) and the facade's
  explicit re-export list, which is the entire public surface.
  `scripts/verify_rhi_boundary.py` mechanically forbids backend types in it.
- The backends are private modules under `src/backend/`, selected at
  compile time by platform — see
  [the private-modules note](2026-10-11-rhi-backend-private-modules.md). The
  [Vulkan pivot](2026-10-13-rhi-vulkan-pivot.md) then deleted the Metal arm,
  leaving Vulkan as the single backend and one re-export list.
- The common contracts the interface pins: memory classes have
  backend-independent meanings (host visibility, readback, ownership,
  GPU-completion); command recording, submission, waiting, and reuse have
  consistent signatures and ordering rules; `Stage`/`Access` semantics and
  root-data snapshot behavior are shared, with each backend owning its
  native mapping; the shader target derives from the selected backend
  (`shader_target()` is `Spirv`).
- Public types such as `Device` and `GpuAllocation` wrap the selected
  concrete backend where adaptation or shared invariants are needed. Upper
  layers keep concrete RHI types; neither backend generics nor trait
  objects are a requirement for render, editor, or ml. Consumers depend on
  `moonfield-rhi` without fixing a backend — the mutually-exclusive
  `vulkan`/`metal` cargo features are gone.

## Alternatives considered

**Retain backend sub-crates behind an RHI-owned interface.** This preserves
independent compilation units, dependency restrictions, and in-crate GPU tests.
It is the migration starting point and remains a valid final layout if those
properties justify the extra cross-crate interface. Private modules are preferred
when backends remain internal implementations with no independent consumers.

**Merge crates but keep backend re-exports as the interface.** This reduces
manifests without resolving semantic differences or upper-layer backend
knowledge. File relocation alone does not meet the proposal's objective.

**Propagate `Device<B>` and backend traits through upper layers.** Static
polymorphism can enforce an implementation contract, but exposing it to every
consumer broadens the interface they must understand. Keep any necessary traits
internal unless consumers have a concrete need to be generic over backends.

**Introduce runtime trait-object or enum dispatch throughout the RHI.** This
supports selecting multiple backends in one binary, but adds dispatch and
cross-backend resource-consistency concerns for a capability the platform-selected
deployment does not require. Revisit it if same-platform runtime selection
becomes a requirement.

## Consequences

- A single re-export list is the entire public surface; the
  `verify_rhi_boundary.py` gate scans the single `src/` tree, and
  target-gating plus private-module visibility carry the dependency and
  separation discipline the sub-crates' crate boundaries used to enforce.
- The contract test suites (`tests/contract_*.rs`) pin the public behavior
  any backend has to honor; backend-local GPU tests live on as
  `#[cfg(test)]` modules of the private backend.
- Consumers no longer name backends: manifests carry a plain
  `moonfield-rhi` dependency, shader targets come from the RHI, and no
  backend-specific imports remain in render, editor, or ml.
- macOS is outside the supported targets until a backend lands there
  again — the Vulkan pivot records the trade; adding D3D12 or runtime
  fallback revisits selection while keeping the public resource and command
  contracts stable.
- Any backend change recompiles the one RHI crate; a smaller package list
  does not imply faster builds.
