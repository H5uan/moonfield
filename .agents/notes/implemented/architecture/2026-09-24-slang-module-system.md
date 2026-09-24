# Agent Note: Slang shaders organize into explicit modules

Status: implemented

[中文](2026-09-24-slang-module-system.zh.md)

## Problem

Every shader under `assets/shaders/` was a legacy single file: no `module`
declaration, so the compiler treated all symbols as implicitly public
(Slang's compatibility mode for pre-module code, which the user guide
reserves the right to deprecate). The one shared library, `gs/gaussian.slang`,
already used `public` modifiers without belonging to a declared module, and
`editor_metadata.slang` was not a shader dependency at all — the reflection
test reached it by concatenating its text into the probe source from Rust
(`include_str!` + `concat!`), a sharing mechanism the host side should not
own. Nothing established how a multi-file module or an importable library
should be laid out, so the pattern would have been improvised per-shader as
the collection grows.

## Decision

All shaders opt into Slang's module system (Slang 2026.16.1, already pinned
by `shader-slang-rs-sys`), following the user guide's recommended layout:
top-level files are importable module primaries, implementation details live
in subdirectories.

- `gaussian.slang` (moved up from `gs/`) is the primary file of the `gaussian`
  module: a `module gaussian;` declaration plus `__include`s. The
  implementation splits into `gs/types.slang`, `gs/projection.slang`, and
  `gs/color.slang`, each starting with `implementing gaussian;`. The split
  follows the module's own seams — data types, projection math (`cov3d`,
  `project`), color evaluation — and extracting the quaternion-to-rotation
  code into the module-internal `quat_rotation` helper gave the module a
  genuinely non-public symbol.
- `editor_metadata.slang` becomes the importable `editor_metadata` module
  with `public` attribute structs — the editor-metadata attribute types are
  used from *other* modules' shader sources, so `internal` would hide them
  behind the module boundary. The reflection test now compiles a probe that
  does `import editor_metadata;` instead of concatenating the file's text.
- The entry-point shaders — `core_3d.slang`, `egui.slang`,
  `util/radix_sort.slang`, `ml/adam.slang` — each declare their module
  (`module core_3d;` etc.) and keep the default `internal` visibility:
  they are compiled directly and never imported, so nothing needs to cross
  their boundary.
- Two compile tests pin the semantics: a probe importing `gaussian` through
  the module-name path hint (now resolving against the top-level primary),
  and `internal_symbols_are_invisible_across_import`, which asserts that
  reaching for `quat_rotation` from an importing module fails to compile —
  proving the declarations are load-bearing, not decorative.

No `#language slang 2026;` directive is added: the explicit `public`
modifiers already cover the visibility the module needs, and opting into
the 2026 member-visibility defaults would change semantics no current code
relies on.

## Alternatives considered

- **Keep the files where they are and only add `module` declarations.**
  Rejected: the gs/ml/util directories would keep acting as a hand-rolled
  namespace while the module names act as another, and the top-level
  directory would stay ambiguous about what is importable API versus
  entry-point implementation. The user guide's convention (primaries at the
  top, details below) costs two test-path updates and removes the ambiguity.
- **Keep `gaussian` as a single declared file; split later.** Rejected: at
  108 lines the split is cheap, and the multi-file pattern (`module` +
  `__include` + `implementing`) needed a template in-repo before any module
  actually grows — otherwise the first real split would improvise structure
  again. The split also required an internal helper (`quat_rotation`), which
  is exactly the access-control surface the module system exists to express.
- **Keep the `include_str!` concatenation for `editor_metadata`.** Rejected:
  it duplicates the module dependency in Rust source instead of declaring it
  in Slang where the dependency lives, and it cannot be reused by a real
  shader that wants `[EditorColor]`-style metadata without the host
  concatenating more text. The virtual-path import pattern was already
  proven by the gaussian probe test.
- **Import `editor_metadata` with `internal` attribute structs.** Rejected:
  attribute structs are referenced from importing modules' sources, which is
  precisely what `internal` forbids; `public` is the correct specifier and
  the reflection test verifies attributes still surface through the import.

## Consequences

- Legacy module mode is gone: an undeclared symbol is now `internal`, so a
  future accidental cross-module reach fails at compile time instead of
  silently resolving. The `internal_symbols_are_invisible_across_import`
  test keeps that contract observable.
- Preprocessor state does not propagate across `import`/`__include`
  boundaries. Today that is free — `defines` variants only feed
  entry-point modules compiled directly — but a future module that wants
  macro-driven behavior must carry its own configuration, not inherit the
  importer's macros.
- In-memory wrapper shaders (tests, future training-kernel wrappers) must
  be compiled under a module name that places them next to the module they
  import; the virtual-path convention (`assets/shaders/__*_probe.slang`) is
  now the established way to do that and moved with `gaussian.slang`.
- The `gaussian` module's public surface is exactly its `public` symbols:
  the three data types, `cov3d`, `project`, `eval_color`. Everything else
  (currently `quat_rotation`) is implementation detail the module can
  reorganize without touching importers.
- SPIR-V output is unchanged: the gs_math test's numerical assertions
  against the reference implementation still pass bit-for-bit after the
  split, so this is a pure structural change with no codegen drift.
