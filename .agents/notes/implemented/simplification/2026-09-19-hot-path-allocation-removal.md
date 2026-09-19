# Agent Note: Hot-path allocation removal in AssetServer and message updates

Status: implemented

[中文](2026-09-19-hot-path-allocation-removal.zh.md)

## Problem

Two per-call allocations sat on paths that run every frame or per asset
access:

1. `AssetServer::load` built its cache key `(TypeId, PathBuf)` with
   `path.to_path_buf()` before probing the cache, so every hit — the common
   case — cloned the path only to drop it.
2. `message_update_system` cloned the whole `MessageRegistry.updates` vector
   of fn pointers each frame, to release the registry's resource borrow before
   running update fns against the world.

## Decision

- The asset cache is nested maps, `HashMap<TypeId, HashMap<PathBuf, AssetId>>`,
  so the hit path probes with a borrowed `&Path` (the inner map's `Borrow`
  lookup) and allocates the `PathBuf` only on an actual load. The logical key
  stays `(TypeId, PathBuf)`: a path loads at most once per type, and a cached
  id that no longer resolves in the `Assets<T>` store falls through to a
  reload whose insert replaces the stale entry. The public `load` signature is
  unchanged; no caller needed updating.
- `message_update_system` iterates the registry by index: each step takes the
  resource borrow, copies one `fn(&mut World)` (a `Copy` pointer), and drops
  the borrow before calling it. No borrow of `MessageRegistry` is held across
  an update fn, so an update fn may itself touch the registry resource without
  a borrow conflict. The registry stays present in the world throughout the
  run.

## Alternatives considered

- **Raw-entry or `hashbrown` lookup on the flat `(TypeId, PathBuf)`
  map.** Rejected: the crate has no `hashbrown` dependency and std's raw-entry
  API is unstable; nested maps reach the same zero-alloc hit path with stable
  std alone, at the cost of one extra map hop per lookup.
- **Take the `updates` vector out of the registry (or `remove_resource` the
  registry), run it, and put it back.** Rejected: an update fn running against
  the world would observe an empty or missing `MessageRegistry` mid-run, and
  the swap adds a failure path where the vector must be restored. Index
  iteration keeps the registry intact and reads each fn under a short borrow.
- **Store `Rc<[fn(&mut World)]>` snapshots in the registry.** Rejected:
  resources are `Send + Sync`, so this means `Arc` and a refcount bump per
  frame instead of a clone; index iteration costs neither.

## Consequences

- A cache hit in `AssetServer::load` performs no allocation; only a real load
  (or a reload of a removed asset) allocates the `PathBuf` key.
- `message_update_system` allocates nothing per frame; each registered message
  type costs one resource lookup plus a fn-pointer copy per frame.
- A message type registered while updates run has its buffer swap picked up in
  the same frame (registration currently happens only at app build time, so
  this is unreachable in practice).
