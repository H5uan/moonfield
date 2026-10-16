# Agent Note: Shader session synchronization and reflected layouts

Status: implemented

[中文](2026-10-16-shader-session-and-layout-safety.zh.md)

## Problem

The bytecode and reflection caches had separate mutexes around one Slang
global session. A cache miss in each table could enter the session at the
same time. `Reflection` also exposed unsynchronized lazy queries through
handwritten `Send`/`Sync` implementations; owning its pointers established
lifetime, not thread safety.

`RootBinder::set_bytes` accepted shorter inputs before copying into the whole
parameter range, which panicked. Struct code generation printed reflected
offsets as comments without representing alignment or padding in Rust fields.
Packed matrix arrays also omitted their reflected storage padding.

## Decision

The global session is the synchronization and ownership unit. The locked
Slang bindings provide `Send` for their owned session and component objects,
so Rust mutexes provide shared access without handwritten thread markers.
Reflections retain the originating global session, and native-program release
uses its lock as well as compilation and queries. Native reflection nodes
remain inside those operations; returned layouts are value snapshots.

Root writes require exact storage lengths. Generated Rust structs encode
reflected offsets and total size with explicit byte padding and alignment.
Matrix fields describe physical storage, including padding, rather than a
tightly packed list of mathematical elements. The runtime contracts live in
[architecture.md](../../../../docs/architecture.md#assets).

## Alternatives considered

- **One mutex per reflected program.** Rejected: distinct programs and
  compilation sessions can share a global session, so program-local locks
  do not synchronize all access to that session.
- **Return layouts with a held session lock.** Rejected: a caller retaining
  one layout while querying another would require recursive locking. Value
  snapshots let layout inspection run without further native calls.
- **Accept partial root writes.** Rejected: the API supplies a complete
  parameter value; a short host mirror indicates a layout mismatch rather
  than an intentional update range.
- **Reject every padded struct or matrix.** Rejected: explicit storage
  represents these layouts while retaining the existing source-generation API.

## Consequences

Compilation, queries, and reflected-program destruction sharing a global
session serialize; independent compilers retain independent locks. Layout
queries allocate owned field-offset snapshots. Reflection nodes do not escape
the synchronized owner, and returned reflections keep working after the
`Compiler` handle is dropped.

Regression tests exercise lock exclusion and concurrent cache callers, reject
incorrect root lengths without mutation, and compile generated structs with
their `bytemuck` derives before comparing Rust sizes and field offsets against
Slang reflection. Unsupported generated field types remain errors.
