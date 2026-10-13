# Agent Note: RHI Vulkan pivot

Status: implemented

[中文](2026-10-13-rhi-vulkan-pivot.zh.md)

## Problem

The private-module layout keeps two platform-selected backends behind two
cfg-gated re-export lists (`vulkan` on non-Apple targets, `metal` on Apple).
Every RHI change pays the second arm's tax: a new API needs a Metal
implementation or a capability gate, the re-export lists drift, and CI
(ubuntu/windows) never compiles the Metal arm — its conformance is verifiable
only on a Mac, where the editor does not build (the bindless channel is the
Metal proposal's deferred phase work).

## Decision

Vulkan is the RHI's pivot API — the one backend, and the semantics the public
vocabulary is shaped around:

- `backend/metal/` is deleted; `backend/vulkan/` is the single private
  backend, with no platform selection. The facade carries one re-export list,
  and `shader_target()` is `Spirv`.
- The native stack (ash/ash-window/gpu-allocator) is unconditional in
  `Cargo.toml`; the objc2 stack is gone with the Metal arm.
- The interface discipline that keeps a second backend cheap is unchanged:
  the vocabulary modules own the semantics, the facade's re-export list is
  the entire public surface, `verify_rhi_boundary.py` forbids backend types
  in it, and the contract suites (`tests/contract_*.rs`) pin the public
  behavior any later backend has to honor.
- A later second backend conforms to this pivot the way Godot's
  RenderingDevice backends conform to its Vulkan-shaped interface. The
  likeliest one, D3D12, maps onto the descriptor-heap bindless vocabulary
  (`VK_EXT_descriptor_heap` is D3D12-shaped by design) and owns the
  stage-to-resource-state translation internally; barriers stay on the
  `Stage` vocabulary, and the render-pass description stays a lightweight
  desc, not a Vulkan render-pass object.
- The Slang `ShaderTarget` vocabulary keeps its Metal targets for offline
  compilation; the backend's runtime target is `Spirv`.

## Alternatives considered

**Keep the Metal arm dormant.** Lost: the platform cfg hides the arm from
CI, so it rots silently — nothing builds it, nothing verifies it, and the
contracts drift underneath. A frozen backend is a rewrite on return; the
same outcome as deletion with a tax in between.

**Keep both arms and finish the Metal bindless channel.** Lost: the deferred
phase work serves no consumer; the editor is the only binary and the
supported targets are Windows and Linux.

## Consequences

- macOS is outside the supported targets until a backend lands there again.
  Git history and the Metal notes preserve the implementation knowledge, and
  the Vulkan loader (MoltenVK) can reach Apple hardware in principle.
- Every RHI change compiles one backend; the capability-gate tax on new APIs
  and the dual re-export lists are gone.
- The sub-crate-era re-export layer `backend/vulkan/shader/` is deleted;
  backend code imports the shared shader vocabulary directly from
  `crate::shader`.
