# Agent Note: Shader source compiles in memory

Status: implemented

[中文](2026-09-01-shader-source-compiles-in-memory.zh.md)

## Problem

`Compiler::compile_source_to_spirv` received in-memory Slang source but
compiled it through a temp file: `Session::load_module` is the file-based
Slang entry point, so the source was written to `std::env::temp_dir()` as
`{module_name}.slang`, compiled, then deleted. Each source entry point owned
its own copy of that round trip, and the whole path depended on a writable
system temp directory.

## Decision

`compile_source_to_spirv` and `compile_source_to_spirv_with_capabilities` load
the source with `Session::load_module_from_source_string`, registering it under
`module_name` with a synthetic `{module_name}.slang` path for diagnostics. No
file is written or removed.

The compile pipeline shared by file and source inputs is factored into
`Compiler::create_session` (a SPIR-V session for the given capabilities) and
`Compiler::finish_compile` (entry-point lookup, link, bytecode extraction);
`compile_file_to_spirv_impl` uses the same helpers.

## Alternatives considered

- **Keep the temp-file round trip.** Rejected: `shader-slang-rs` already
  exposes `load_module_from_source_string` at the pinned revision, so the
  workaround defended against a limitation that does not exist.

## Consequences

- Source shader compilation is a pure in-memory operation; diagnostics name
  the module as `{module_name}.slang`.
- Both source entry points share one implementation with the file entry
  points instead of each owning a temp-file variant.
- Relative `import`s in in-memory shaders resolve against the synthetic module
  path (the working directory) rather than the system temp directory. No
  shader compiled this way uses relative imports.