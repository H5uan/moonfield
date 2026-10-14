# Agent Note: Main-world parking scope and thread contracts

Status: implemented

[中文](2026-10-14-main-world-parking-and-thread-contracts.zh.md)

## Problem

The [architecture contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)'s
first item covered three soundness gaps: `World::park_main_world` stored a
borrowed world's address through a public safe method, so a caller could keep
the `MainWorld` resource alive past its source (and `App::render` dropping
the taken-out main world mid-panic left exactly that dangling pointer in the
render world); the RHI's `CommandBuffer` paired a `Cell<bool>` recording flag
with hand-written `Send + Sync` impls, so a concurrent `end` through two
shared references was a genuine data race; and `Resource`'s
`Send + Sync + 'static` bound gave GPU objects that cannot soundly be shared
no home in a `World` at all.

## Decision

**Parking is a scope, not a pair of calls.** `World::with_parked_main_world`
is the only way to park: it inserts the `MainWorld` resource, runs the
closure, and a drop guard removes the resource on every exit path including
unwind, so the raw pointer inside `MainWorld` never outlives the call that
borrows the source world. `park_main_world`/`unpark_main_world` are private.
`App::render` wraps extraction in `catch_unwind`/`resume_unwind`: the panic
still propagates, but only after the scope has unparked and the taken-out
main world is restored to the app, so a panicking extractor leaves the app
recoverable. `MainWorld::world`/`world_mut` keep their unsafe preconditions —
they are the retained raw-pointer entries the proposal asks to expose.

**`CommandBuffer` is auto `Send + Sync`; the shared mutation is atomic.**
The `ended` flag is an `AtomicBool`: `end` claims the end with a swap before
calling `vkEndCommandBuffer`, so a concurrent double-end is a no-op and never
a race. With the `Cell` gone, every field is `Send + Sync` and the manual
impls are deleted. The struct documentation carries the recording contract:
recording methods take `&self` because the frame loop hands shared
references to single-threaded schedule systems; sharing a *recording* buffer
across threads violates Vulkan's host-synchronization requirement and is a
caller contract violation.

**Thread-local resources are pinned storage, not a new world.** `Resources`
keeps one map and one LIFO drop order; a `thread_pins` table marks types
inserted through `World::insert_thread_local_resource` (any `'static` type)
and records the inserting thread. The thread-local accessors panic on
non-owner-thread access and on regular-vs-thread-local accessor mismatch.
Pinning is forward-looking — `World` is neither `Send` nor `Sync` today — and
turns a future cross-thread access into a panic instead of a data race.

The other manual impls were audited and stay: `Reflection` (read-only shared
queries over an owned Slang object), `ShaderCache` (all session access under
its mutexes), `HostPtr` (single-writer allocation contract), and
`RawHandleWrapper` (handles only consumed by surface creation) each carry
their justification at the impl site.

## Alternatives considered

- **Making parking `unsafe` instead of scoped.** Lost: the scope removes the
  failure mode (a parked pointer outliving its source) rather than asking
  every caller to avoid it, and keeps `App::render` free of unsafe.
- **Exclusive (`&mut self`) recording methods on `CommandBuffer`.** Deferred:
  recording flows through `RenderContext`'s shared references to render
  systems, so exclusivity would ripple through render-core, render-feature,
  the editor, and the trainer for a guarantee the single-threaded schedule
  already provides. The audited contract is documented instead; the atomic
  flag removes the one real race.
- **A separate map for thread-local resources.** Lost: a second store gets
  its own drop pass and breaks the LIFO guarantee when a thread-local GPU
  object depends on a regular resource created earlier.
- **Thread-local system parameters.** Deferred: no consumer fetches
  thread-local resources from systems yet; adding param support without one
  would be an unexercised framework surface.

## Consequences

- Safe extraction can no longer read a destroyed main world: the resource
  cannot outlive `with_parked_main_world`, and `App::render` restores the
  main world before a panic resumes (covered by tests in `moonfield-ecs` and
  `moonfield-app`).
- ECS resource drop order is unchanged across the mixed store; a thread-local
  resource drops before the resources it was created from, same as a regular
  one.
- `CommandBuffer`'s `Send + Sync` is now a compiler-checked property of its
  fields; a field that loses the traits fails the build where owners store
  buffers in resources, instead of weakening a hand-written promise.
- System-param access to thread-local resources does not exist; systems that
  need them take `&mut World` (or the `World` accessors) directly.
