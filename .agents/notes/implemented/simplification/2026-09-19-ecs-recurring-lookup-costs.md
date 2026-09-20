# Agent Note: Recurring lookup and allocation costs in ECS resources, hooks, entities, and the fixed loop

Status: implemented

[中文](2026-09-19-ecs-recurring-lookup-costs.zh.md)

## Problem

An audit of `moonfield-ecs` and `moonfield-time` surfaced four recurring
costs on per-frame or per-entity paths:

1. The resource store (`Resources`) and the schedule store (`Schedules`) key
   `HashMap<TypeId, …>` with the default SipHash hasher, so every
   `Res<T>`/`ResMut<T>` fetch and every schedule run hashes the `TypeId`
   through SipHash.
2. `World::fire_hook` performs two registry lookups per component event
   (take, then restore) even when no hooks are registered at all.
3. `Entities::contains` linear-scans the reserved tail of `pending`, and
   `Entities::alloc_at` linear-scans the whole list to swap an id out of the
   free list. `alloc_at` runs on every `Commands::spawn` application (a
   reserved id is never in the free list, so the common case is a full miss
   scan), and `contains` runs in relationship insert hooks on every link.
4. `run_fixed_main_schedule` re-inserts the generic `Time` resource on every
   fixed step, and `Resources::insert` builds a fresh
   `RefCell::new(Box::new(..))` — one heap allocation per step, plus an entry
   in the LIFO drop-order list each time.

## Decision

- `Resources` and `Schedules` key on `TypeIdMap`, the crate's existing
  identity-hash map for `TypeId` keys (a `TypeId` is already unique, so the
  hasher forwards the id bits instead of mixing them).
- `fire_hook` returns early when the hook registry is empty, skipping both
  lookups on worlds (or component types) without hooks.
- `Entities` maintains a dense side index `pending_pos: Vec<u32>` mapping an
  entity id to its position in `pending`, or `u32::MAX` when absent.
  Reservations only move `free_cursor` and never reorder `pending`, so
  positions survive reserve/flush cycles; the sites that add or remove
  `pending` entries (`free`, `alloc`, `alloc_at`, `flush`, `set_freelist`,
  `clear`) keep the index in sync. `contains` becomes one indexed load
  compared against `free_cursor`, and `alloc_at` swap-removes by index
  directly.
- The fixed loop writes each step's snapshot into the existing generic `Time`
  through its `RefCell` borrow (`*generic = snapshot`) instead of replacing
  the resource, inserting only when the resource is absent; the
  restore-to-virtual write after the loop does the same.

## Alternatives considered

- **A `HashMap<u32, u32>` side index for `pending`.** Rejected: entity ids are
  dense indices into `meta`, so a `Vec` gives O(1) lookups with no hashing at
  all, at 4 bytes per ever-allocated id next to `meta`'s 8.
- **Keep `pending` sorted and binary-search it.** Rejected: `alloc`/`free`
  use `pending` as a stack and the id-reuse order is observable in the
  generations handed out; sorting changes that order.
- **Re-key the hook registry map itself on `TypeIdMap`.** Not taken: the
  empty-registry fast path already removes the lookup cost for the no-hooks
  case, and the registry is consulted only once any hook exists — the
  SipHash-keyed map can ride a future change that touches `world.rs`'s hook
  storage for another reason.

## Consequences

- Resource fetches and schedule lookups do no SipHash work; behavior is
  unchanged (the LIFO drop order of resources is preserved — the insertion
  order list, not the map, drives it).
- Worlds without registered hooks pay nothing per component event; once any
  hook is registered, lookups still go through the SipHash-keyed registry
  map.
- `contains` and `alloc_at` are O(1) in the size of `pending`; `alloc` and
  `free` each pay one extra array write.
- A steady-state fixed loop allocates nothing per step. The observable clock
  contract is unchanged: the generic `Time` mirrors `Time<Fixed>` while the
  fixed schedules run and is restored to `Time<Virtual>` afterwards, and a
  missing generic `Time` resource is still created on the first step.
- Existing `moonfield-ecs` and `moonfield-time` test suites cover all four
  paths (entity reservation/flush round trips, hook firing order, resource
  round trip, fixed-step counting with generic-clock assertions).
