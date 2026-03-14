# System-level Slang Wrappers

This module provides safe wrappers around the low-level FFI bindings to Slang.

## Overview

The `sys` module sits between the raw FFI bindings and the high-level idiomatic Rust API.
It provides:

- **COM Interface Management** - Safe handling of Slang's COM-style interfaces with automatic reference counting
- **Error Handling** - Conversion of Slang result codes to Rust `Result` types
- **String Conversions** - Safe conversion between C strings and Rust strings
- **Null Safety** - Checks for null pointers before dereferencing

## COM Interface Pattern

Slang uses a COM-style interface system where:

1. All interfaces inherit from `ISlangUnknown` which provides:
   - `queryInterface()` - Cast to other interface types
   - `addRef()` - Increment reference count
   - `release()` - Decrement reference count (frees when zero)

2. Interfaces are identified by UUIDs (GUIDs)

3. Methods use the `SLANG_MCALL` calling convention (stdcall on Windows)

The `ComPtr<T>` smart pointer handles reference counting automatically:

```rust
use slang_rs::sys::ComPtr;

// ComPtr automatically calls release() when dropped
let session: ComPtr<GlobalSession> = GlobalSession::new()?;

// Clone calls addRef()
let session2 = session.clone();
```

## Error Handling

Slang uses `SlangResult` (alias for `int32_t`) for error codes:
- Values >= 0 indicate success
- Values < 0 indicate failure

The `result_from_slang()` function converts these to Rust `Result` types.

## Safety

All functions in this module are safe Rust wrappers. Unsafe operations are contained within:
- FFI calls to Slang C functions
- Pointer dereferencing with null checks
- Transmuting between interface types

## Future Work

- Add more interface wrappers (ISession, IModule, etc.)
- Implement iterator support for reflection types
- Add builder patterns for descriptor structures
- Thread safety verification (Slang currently not thread-safe)