# Agent Note: Editor per-frame allocation removal

Status: implemented

[中文](2026-09-19-editor-per-frame-allocation-removal.zh.md)

## Problem

The editor re-allocated the same shapes of memory every frame: the egui
upload built fresh vertex/index/draw Vecs per frame slot, texture deltas
copied pixels one `Color32::to_array` at a time, the hierarchy panel stored
an owned `String` label per row plus one `format!` per row for the hover
tooltip, the viewport overlay built its status lines as `String`s, the
inspector's `prettify` allocated a `String` per field, and two debug env
vars were re-read through `std::env::var` every frame. Alongside the churn,
`prepare_egui_frame` picked the egui pipeline's color format from whichever
`WindowSurfaces` entry the HashMap yielded first — correct only because the
editor is single-window.

## Decision

- `egui_vk::FrameResources` owns persistent CPU staging Vecs (vertices,
  indices) alongside the existing `mesh_draws`; `update` clears and refills
  them instead of allocating.
- Texture uploads cast `ColorImage.pixels` to `&[u8]` with
  `bytemuck::cast_slice`; the editor's `egui` dependency enables egui's
  `bytemuck` feature so `Color32` is `Pod`. Zero copies, no per-pixel calls.
- `HierarchyEntry` stores only the entity and its depth; the label resolves
  at draw time (`entity_label` borrows the `Name`, only unnamed entities
  format), the hover text moved to `on_hover_ui` so it is built only while
  hovered, and the panel's row buffer lives on `EditorMainState`, refilled
  through `collect_hierarchy_into`.
- The viewport overlay's lines are `&'static str`: the mode line is one
  static string per `GizmoMode`, and the selection-hint row is simply absent
  when a selection exists.
- `EditorDebugEnv` parses `MOONFIELD_EDITOR_AUTO_CLOSE` and
  `MOONFIELD_EDITOR_DUMP_VIEWPORT` once in `EditorPlugin::build` and is
  inserted into both worlds; the per-frame systems read the resource.
- `prepare_egui_frame` takes the swapchain format from
  `WindowSurfaces::primary()` — the surface resolved from the
  `PrimaryWindow` marker — instead of the map's first entry.
- `registry::prettify` writes into a scratch `String` owned by the
  `reflect_ui` call, reused across that level's fields.

## Alternatives considered

- **Pool the per-row label `String`s across frames.** Rejected: keyed reuse
  has to map frame-to-frame row identity; resolving the label at draw time
  removes the storage question, and named rows — the common case — borrow.
- **Cache the hierarchy tree and rebuild on change detection.** Rejected:
  the panel stays correct only if every structural mutation is tracked; the
  flat rebuild is simple and, with the reused buffer, cheap.
- **A `Cow<str>` fast path in `prettify`.** Rejected: reflected field names
  are snake_case, so nearly every call takes the owned branch and the fast
  path is dead code; the scratch buffer reuses one allocation per component.

## Consequences

- A steady-state editor frame allocates nothing in the egui upload, the
  hierarchy rows, the viewport overlay, or the inspector's field labels.
- The two env vars are launch-time configuration: changing them mid-run has
  no effect.
- With multiple windows, the egui pipeline's format deterministically
  follows the `PrimaryWindow` surface; a tick where no surface acquired an
  image skips preparation instead of guessing.
- `HierarchyEntry` lost its `label` field; the tree's labels are verified
  through `entity_label` in the tests.
