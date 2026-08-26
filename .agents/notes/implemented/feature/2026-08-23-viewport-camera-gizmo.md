# Agent Note: Viewport orbit camera and transform gizmo

Status: implemented

[中文](2026-08-23-viewport-camera-gizmo.zh.md)

## Problem

The editor viewport was a read-only texture: the camera pose was fixed by
whatever `PrimaryCamera` entity the scene happened to spawn, and the only way
to move an entity was dragging numeric fields in the inspector. Neither
navigating the scene nor manipulating objects spatially was possible, which
made the viewport a preview rather than an editing surface.

## Decision

The viewport panel becomes the editor's interactive surface, built entirely
on the existing single-threaded render seam — no renderer or Vulkan changes.

- `moonfield-editor/src/interaction.rs` holds all interaction math as pure,
  headless-testable functions: the `OrbitCamera` (pivot/yaw/pitch/distance
  with pitch and distance clamping), reverse-Z- and Y-flip-aware
  `world_to_screen` / `screen_to_ray` conversion, gizmo hit-testing (8 px
  screen-space threshold), and the `GizmoDrag` state machine for translate,
  rotate, and scale.
- The editor owns the viewport camera: the `OrbitCamera` is initialized once
  from the `PrimaryCamera` entity's `Transform` and written back every frame
  (`apply_orbit_camera` in `lib.rs`). Right-drag orbits, middle-drag pans,
  the wheel dollies toward the pivot.
- The gizmo is a screen-space overlay drawn with egui's `Painter` on top of
  the viewport image — axis arrows for translate, rings for rotate, axis
  handles plus a uniform center handle for scale, switched with W/E/R
  (guarded by `egui_wants_keyboard_input` so text fields keep their keys).
  Handles operate on the entity's local axes; the hovered or dragged handle
  highlights yellow. Handle geometry is screen-space: an axis handle's
  endpoint is placed at a fixed pixel length along the projected 2D axis
  direction, and ring radii derive from the least-foreshortened basis
  direction — sizing never goes through a world-space length, which
  perspective foreshortening would make distance-dependent (the handle
  visibly grew and shrank as the entity moved).
- Drag math freezes the axis direction and origin at drag start. Applying a
  drag against the live `GizmoFrame` would feed back: the origin moves with
  the entity during translate, and the axes rotate with it during rotate, so
  deltas would be measured against a moving reference. Axis translation
  additionally drags against a plane (contains the axis, faces the drag
  ray), with the plane normal frozen at drag start.
- Drags compute a world-space TRS; `world_trs_to_local` converts it back to
  the entity's local `Transform` through the parent's `GlobalTransform`
  affine inverse, so gizmo edits compose correctly with hierarchy. The
  propagation system then refreshes `GlobalTransform` the same frame.

## Alternatives considered

- **Render the gizmo in the 3D scene pass.** Rejected: it needs a line/overlay
  pipeline, depth handling, and picking support in the Vulkan renderer — a
  large surface for what is editor chrome. The 2D egui overlay keeps all
  gizmo code in one module and is the standard first implementation.
- **Adopt a third-party gizmo crate (e.g. transform-gizmo-egui).** Rejected:
  it would add a dependency pinned to the egui version the workspace anchors,
  while the required math (drag-plane intersection for translate and
  rotate, screen-distance ratio for scale) is
  small and fully unit-testable in-house.
- **Closest-point-between-lines for axis translation.** Rejected after it
  shipped briefly: when the view ray is nearly parallel to the dragged axis
  (the axis handle points at the camera), the closest-point parameter
  diverges and the entity teleports off-screen on the first drag pixel. The
  drag plane — containing the axis, facing the drag ray, its normal frozen
  at drag start — degenerates only at exact parallelism, where the handle
  projects to a point and no drag begins at all.
- **Give the editor its own camera entity instead of driving
  `PrimaryCamera`.** Rejected: two camera sources need synchronization rules
  and a way to pick which one renders. Owning the primary camera's pose is
  one write per frame and keeps one source of truth; the price is documented
  below.

## Consequences

- The editor owns the viewport camera while it runs: editing the primary
  camera's `Transform` in the inspector is overwritten on the next frame.
- The gizmo is local-mode only (handles follow the entity's rotation); a
  world/local toggle and click-to-select mesh picking in the viewport are
  deliberately out of scope.
- Viewport left-click does nothing without a gizmo handle under the pointer —
  the slot is reserved for click-to-select.
- `interaction.rs` confines the engine's reverse-Z and Y-flip conventions to
  two conversion functions; all gizmo math above them works in egui's
  top-left-origin screen space.
- The gizmo pipeline works in (translation, rotation, scale) order while
  glam's `to_scale_rotation_translation` returns (scale, rotation,
  translation); the reorder happens explicitly at the decompose boundary in
  `ui.rs`. A bare destructure there once swapped the two ends: the entity's
  translation was written into its scale, the zero components degenerated
  the next frame's decomposition, and rotation came back NaN/inf — the
  "gizmo vanishes on first drag" failure.
