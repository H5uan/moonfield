# Agent Note: UE5-style editor layout

Status: implemented

[中文](2026-08-23-ue5-style-layout.zh.md)

## Problem

The editor's initial dock layout was a generic three-pane arrangement
(hierarchy left, inspector right, viewport center) that did not match the
shape users expect from a scene editor: the viewport — the primary work
surface, now that it carries camera controls and the transform gizmo (see
[viewport camera gizmo](2026-08-23-viewport-camera-gizmo.md)) — shared the
window evenly with side panels, and asset/scene file operations crowded the
top of the hierarchy tree.

## Decision

The initial layout follows UE5's editor shell, built with the existing
egui_dock splits in `ui.rs::initial_dock_state`:

- **Viewport** dominates the center.
- **Outliner** (hierarchy, renamed) sits top-right, **Details** (inspector,
  renamed) below it — a ~22%-wide right column split evenly.
- **Content Browser** (new `Tab::Content`) occupies a bottom strip under the
  viewport (~28% height) and takes over the asset-load and scene save/load
  rows that used to sit on top of the hierarchy tree. The hierarchy panel is
  now purely the entity tree.

Only the default layout and tab titles change; dock rearrangement by the
user is unaffected, and no panel logic moved between modules.

## Alternatives considered

- **Keep the generic three-pane layout.** Rejected: the viewport is the
  editor's center of gravity; giving it the dominant area and grouping
  entity editing on the right matches the mental model of the engine's
  audience.
- **Build a real content browser (directory listing, thumbnails) now.**
  Deferred: it needs the native file dialog and asset enumeration, both
  known debts. The bottom panel is the right home for the existing typed-path
  rows today and for the real browser later.
- **Rename the `Tab` variants to UE5 terms (`Outliner`, `Details`).**
  Rejected: pure identifier churn; only the displayed titles changed.

## Consequences

- Users see UE5 panel names (Outliner / Details / Content Browser) while the
  code keeps the `Hierarchy` / `Inspector` variant names.
- Asset and scene file operations moved from the hierarchy panel to the
  Content Browser; the hierarchy panel lost its top rows and shows only the
  entity tree.
- The layout is only the initial state — egui_dock still lets users
  rearrange everything at runtime.
