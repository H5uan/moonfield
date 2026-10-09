# Agent Note: Abort failed frame submits

Status: implemented

[中文](2026-10-08-abort-failed-frame-submits.zh.md)

## Problem

A failed frame submit froze every window that had acquired an image.
`end_frame` consumes the frame plan before any fallible Vulkan call; its
caller logged the error and returned early, skipping the present loop that
is the only consumer of the acquired image. With the image bookkeeping
stranded, the window's next acquire skipped it, and swapchain recreation was
guarded on no image being held — one transient `vkEndCommandBuffer` /
`vkQueueSubmit2` error left the window on its last presented frame with an
error logged every tick. Two hazards came with the stranded state: the
acquire left `image_available[slot]` signaled with no consumer (binary
semaphores cannot be unsignaled), and a failed `end` could leave the command
buffer in the recording state, which `begin` does not accept.

## Decision

`submit_window_frames` aborts the frame when `end_frame` fails.
`FrameContext::abort_frame` resets the slot's command buffer and abandons
the uploader's un-submitted batch (`FrameUploader::abort_frame` resets the
batch's buffer and frees its staged arena); the frame number and slot stay —
the timeline value was never signaled, so the retried frame signals it.
`WindowSurfaceData::abort_frame` drops the acquisition bookkeeping, rebuilds
that slot's `image_available` semaphore (the acquire's signal completed when
the acquire returned and nothing waits on it, so the old one is destroyed
immediately), and flags the swapchain for recreation — the retired
swapchain's deferred destruction releases the image. The other slot's
semaphore may still be waited on by the in-flight previous frame and is left
alone. The next tick recreates the swapchain through the ordinary
`create_window_surfaces` path and presents from it, so a transient failure
costs one dropped frame per window. The acquire-time guard on a window still
holding an image is an invariant check, not a recovery path.

## Alternatives considered

**Advance the frame number on abort.** Rejected: the un-signaled timeline
value is reusable as-is, and advancing would move the next frame onto the
slot of the still in-flight previous frame, stalling on it for nothing.

**Present the acquired image through a recovery submit.** Rejected: it needs
a fresh command buffer that waits on the stranded semaphore — more machinery
than retiring the swapchain, under whatever host condition just failed the
submit.

**Rebuild every per-frame semaphore on swapchain recreation.** Rejected: the
other slot's semaphores may carry pending waits of the in-flight previous
frame; only the stranded slot's semaphore needs replacing.

**Re-stage the abandoned upload batch.** Rejected: the batch's copies target
GPU allocations whose render-side caches (prepared meshes, pooled view
targets) consider them delivered once staged; a re-staging signal is a retry
queue the renderer deliberately does not carry (see
[renderer aligned with Bevy](../architecture/2026-08-24-renderer-bevy-alignment.md)).
A submit-side uploader failure abandons the batch; the dropped writes surface
as stale GPU data until the source asset's revision advances or the target
is recreated.

## Consequences

A failed submit recovers on the next tick instead of wedging the window; the
command-buffer reset rides `CommandBuffer::reset` (frame pools are created
with `RESET_COMMAND_BUFFER`). Offscreen-only frames never strand window
state and keep their one-frame drop. Abandoning an upload batch loses its
staged copies, so GPU data staged in the failed frame can stay stale — the
accepted cost of not carrying re-staging state; the realistic trigger is
host resource exhaustion. The uploader's abort covers both of its failure
sub-cases (a batch still recording, or ended but not submitted): either way
the batch is dropped and the next frame stages fresh.
