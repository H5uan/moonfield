# Agent Note: Main-world parking scope and thread contracts

Status: implemented

[中文](2026-10-14-main-world-parking-and-thread-contracts.zh.md)

## Problem

The [contract hardening proposal](../../proposed/architecture/2026-10-14-architecture-contract-hardening.md)
identifies lifetime and threading contracts that safe callers can bypass.
Removing a parked resource at scope exit does not contain its pointer when a
closure moves the resource or the destination world. Atomic recording flags
likewise do not synchronize Vulkan command recording, pools, or queues.

## Decision

**Parking uses an expiring shared token.** `MainWorld` holds an
`Arc` to an atomic world pointer and the parking thread's identity.
`World::with_parked_main_world` invalidates the token before cleaning up
the destination, including on unwind. A moved resource or world retains only
an expired token. Nested parking into an occupied destination is rejected
without replacing its source. Access checks expiration and thread ownership
before dereferencing; the unsafe reference getters retain their lifetime and
aliasing preconditions.

`App::render` restores the taken-out main world before resuming an
extraction panic. The extraction interface remains compatible with ordinary
resource and query parameters; moving a token does not grant access to its
source on another thread.

**GPU operations are bound to the device's creating thread.**
`DeviceShared` records that thread and checks it before providing core
or extension entry points, the allocator, or its command pool. Recording,
submission, presentation, uploader batches, descriptor writes, and retirement
draining enforce the same ownership. `CommandBuffer::end` checks before
changing its atomic flag, so a rejected foreign-thread call cannot end the
owner's recording. `Send + Sync` permits ECS storage, not concurrent
driver access. Immutable handle values and mutex-protected retirement enqueue
operations remain shareable; GPU work and resource teardown belong on the
owner thread.

**Thread-local resources share the existing storage and drop order.**
`Resources` tracks insertion-thread pins in its single map and LIFO
drop list. Thread-local accessors reject foreign threads and mismatched
regular accessors. No new system-parameter category is introduced.

## Alternatives considered

**Destination cleanup alone.** Rejected: generic resource removal and moving
the destination can escape a cleanup guard. Invalidating the shared token
contains access independently of its storage location.

**Atomic recording state alone.** Rejected: the flag protects a Rust field,
not command-buffer recording, command-pool operations, or queue submission.
The owner-thread check covers those driver calls.

**Non-Send GPU objects and thread-local parameters throughout rendering.**
Deferred: this requires migrating render-world resource fetching and every
recording consumer. Device-level ownership checks enforce the current
single-threaded execution model without adding that parameter category.

**A separate thread-local resource map.** Rejected: independent destruction
passes break the shared LIFO ordering of dependent resources.

## Consequences

- Escaped parked tokens are unusable after normal return or unwind, and
  cannot be dereferenced through safe extraction on a foreign thread.
- GPU recording, pool access, and queue operations fail before driver access
  on a foreign thread. Ownership checks also guard direct extension paths.
- Extraction retains the main-world change clock and the app restores its
  world before propagating panics.
- Mixed resource storage retains its existing drop order; thread-local
  consumers use world accessors rather than a new parameter interface.
