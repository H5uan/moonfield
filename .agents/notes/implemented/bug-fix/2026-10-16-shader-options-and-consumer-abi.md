# Agent Note: Shader options and consumer ABI validation

Status: implemented

[中文](2026-10-16-shader-options-and-consumer-abi.zh.md)

## Problem

Bytecode and reflection used different SPIR-V profiles, and reflection could
not receive the capabilities or macros used for bytecode. A macro-dependent
root layout could therefore describe a different shader variant.

Root binding checked address placements and some storage sizes, while several
consumers wrote fixed Rust mirrors directly. Default-shader layout tests did
not protect runtime shader replacements from reordered fields, additional
roots, or changed pointer targets.

## Decision

Compilation and reflection share one session-options builder. Reflection
options participate in cache identity. Prepared entries with identical
capabilities share a reflection program; consumers select reflection by entry
instead of using the primary entry's group for every pipeline stage.

Consumer ABI checks run at pipeline construction against the actual entry
parameter or pointer-pointee layouts. The checks cover root names and delivery
kinds, complete field lists and offsets, storage sizes, and shared graphics
root placements. Inline recording validates byte lengths without allocation.
The runtime contract is in [architecture.md](../../../../docs/architecture.md#assets).

## Alternatives considered

- **Union every entry's capabilities into one reflection.** Rejected: the
  resulting session does not match entries compiled with fewer capabilities.
  Grouping preserves shared multi-entry reflection without changing an
  entry's compilation options.
- **Validate only total size.** Rejected: field reordering can preserve total
  size while changing the bytes a shader reads. Pointer addresses also say
  nothing about the layout of the records behind them.
- **Rely on default-shader tests.** Rejected: asset revisions and application
  shader overrides reach pipeline construction at runtime. Tests cannot
  enumerate those inputs.

## Consequences

Mixed-capability pipelines retain multiple reflection programs. The primary
reflection accessor names only its own group; entry-specific access returns
the matching group or an error. Undeclared extra reflection entries use empty
capabilities. Prepared pipelines use empty macro definitions; explicit RHI
reflection APIs accept the same definitions as bytecode compilation.

Incompatible host layouts fail pipeline construction, leaving existing valid
pipelines in place where the pass already retains them on failure. The checks
are storage-layout guards, not a shader semantic type system: same-size scalar
substitutions and matrix interpretation remain explicit host/shader contracts.

Regression tests exercise macro-controlled layouts, option-sensitive cache
identity, reflection groups, default consumer layouts, reordered fields,
changed pointer targets, and invalid root or inline-storage contracts.
