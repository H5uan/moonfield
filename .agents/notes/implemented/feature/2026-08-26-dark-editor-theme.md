# Agent Note: dark editor theme

Status: implemented

[中文](2026-08-26-dark-editor-theme.zh.md)

## Problem

The editor rendered with egui's stock dark theme: a neutral near-black
palette with the default spacing, 6px widget corners, and white text. Every
panel, tab bar, and widget looked like a default egui demo. The editor
needed a coherent dark palette — deep blue-grey surfaces (`WINDOW_BG`
`#1F1F24`, `PANEL_BG` `#2A2A2E`, `INPUT_BG` `#36373B`), an accent blue
(`ACCENT_BLUE` `#206EC8`), semantic status colors, tight spacing, and 2/4px
corner radii.

The layout stays UE-style (see [UE5-style editor layout](2026-08-23-ue5-style-layout.md));
the scope here is colour, density, and chrome only.

## Decision

A dedicated `theme.rs` owns the palette and composition. It exposes:

- `theme::install(&egui::Context)` — applies `set_style_of(Theme::Dark, …)`
  from `EditorMainState::new`, so the whole editor inherits via egui-dock's
  `Style::from_egui` bridge (tab bar, tab body, and overlay colors derive
  from `extreme_bg_color` / `window_fill` automatically).
- `theme::visuals()` / `style()` — the palette and the compact spacing
  (`item_spacing` 6×4, `button_padding` 6×3, 2px widget corners, 4px
  window/menu margins).
- `theme::status_color(&str)` — the Load/Save result messages in the
  Content panel color by outcome: `TEXT_SUCCESS` green unless the message
  contains `failed`, then `TEXT_ERROR` red (was a gray `ui.small`).

Viewport overlay hints switched from `Color32::from_white_alpha(160)` to
`theme::TEXT_SECONDARY` so they track the theme.

## Alternatives considered

- **An egui_dock `Style` built by hand in `ui.rs::show`.** Rejected: the
  `DockArea` already derives its style from the egui style when none is set
  (`Style::from_egui`), so per-panel styling duplicates the palette in a
  second home. `theme.rs` stays the single owner; egui-dock's derivation
  does the mapping.
- **A lighter "pro theme" with fonts and icons.** Rejected for this pass:
  font work was explicitly out of scope, and icons belong with the toolbar
  chrome work, not the palette.

## Consequences

- The editor still runs on egui's built-in fonts; text sizes are untouched.
- Gizmo axis colors stay the industry red/green/blue — the theme does not
  recolor the gizmo pipeline.
- The palette is a constant set, not user-configurable; a settings surface
  would be a separate feature.