//! Editor-wide dark theme: a coherent palette of deep blue-grey surfaces
//! (`WINDOW_BG` / `PANEL_BG` / `INPUT_BG`), an accent blue, semantic status
//! colors, and compact spacing, defined once here so every egui-dock panel,
//! widget, and overlay renders consistently.
//!
//! Installed once in `EditorMainState::new`; `egui_dock::DockArea` derives
//! its style from the `egui::Style` here via `Style::from_egui`, so no
//! per-panel styling is needed.

use egui::{Color32, CornerRadius, Stroke, Vec2};

/// Window/panel-edge background (`WINDOW_BG`).
pub const WINDOW_BG: Color32 = Color32::from_rgb(0x1F, 0x1F, 0x24);
/// Panel surface background (`PANEL_BG`).
pub const PANEL_BG: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2E);
/// Elevated surface: inputs, hovered widgets (`INPUT_BG`).
pub const INPUT_BG: Color32 = Color32::from_rgb(0x36, 0x37, 0x3B);
/// Primary accent blue: selection, active tabs, links (`ACCENT_BLUE`).
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x20, 0x6E, 0xC8);
/// Primary text (`TEXT_PRIMARY`).
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xEC, 0xEC, 0xEC);
/// Secondary / dimmed text (`TEXT_SECONDARY`).
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xA8, 0xA8, 0xA8);
/// Subtle border color (`BORDER_SUBTLE`).
pub const BORDER: Color32 = Color32::from_rgb(0x41, 0x41, 0x42);
/// Success message color (`TEXT_SUCCESS`).
pub const TEXT_SUCCESS: Color32 = Color32::from_rgb(0x5C, 0xB8, 0x6B);
/// Warning message color (`TEXT_WARNING`).
pub const TEXT_WARNING: Color32 = Color32::from_rgb(0xEB, 0xBF, 0x47);
/// Error message color (`TEXT_ERROR`).
pub const TEXT_ERROR: Color32 = Color32::from_rgb(0xE6, 0x52, 0x52);

/// The status color for a Load/Save result message: success green unless the
/// message names a failure (`TEXT_SUCCESS` / `TEXT_ERROR`).
pub fn status_color(message: &str) -> Color32 {
    if message.contains("failed") {
        TEXT_ERROR
    } else {
        TEXT_SUCCESS
    }
}

/// Install the editor theme on an egui context. Call once after creating the
/// context; the dock area then inherits everything through egui-dock's
/// `Style::from_egui` bridge.
pub fn install(ctx: &egui::Context) {
    ctx.set_style_of(egui::Theme::Dark, style());
}

/// The full style (visuals + spacing) the editor chrome renders with.
pub fn style() -> egui::Style {
    let mut style = egui::Style {
        spacing: egui::Spacing {
            item_spacing: Vec2::new(6.0, 4.0),
            button_padding: Vec2::new(6.0, 3.0),
            interact_size: Vec2::new(28.0, 20.0),
            indent: 12.0,
            window_margin: egui::Margin::same(4),
            menu_margin: egui::Margin::same(4),
            ..Default::default()
        },
        ..Default::default()
    };
    style.visuals = visuals();
    style
}

/// Editor widget visuals for one interaction state: flat fill, hairline
/// border, small corner radius.
fn state_widget(bg: Color32) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: bg,
        weak_bg_fill: bg,
        bg_stroke: Stroke::new(1.0, BORDER),
        corner_radius: CornerRadius::same(2),
        fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
        expansion: 0.0,
    }
}

/// The editor's dark `Visuals`: the surface ladder and accent colors above.
pub fn visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals {
        dark_mode: true,
        // egui-dock derives the tab bar from `extreme_bg_color` and the tab
        // body from `window_fill` (via `Style::from_egui`), so these two
        // fields carry the whole surface hierarchy: window stays darkest,
        // panels sit one step lighter, inputs one above that.
        window_fill: PANEL_BG,
        panel_fill: WINDOW_BG,
        extreme_bg_color: WINDOW_BG,
        text_edit_bg_color: Some(INPUT_BG),
        faint_bg_color: PANEL_BG,
        code_bg_color: INPUT_BG,
        weak_text_color: Some(TEXT_SECONDARY),
        hyperlink_color: ACCENT_BLUE,
        warn_fg_color: TEXT_WARNING,
        error_fg_color: TEXT_ERROR,
        window_corner_radius: CornerRadius::same(4),
        menu_corner_radius: CornerRadius::same(4),
        window_stroke: Stroke::new(1.0, BORDER),
        selection: egui::style::Selection {
            bg_fill: Color32::from_rgba_unmultiplied(0x20, 0x6E, 0xC8, 80),
            stroke: Stroke::new(1.0, ACCENT_BLUE),
        },
        widgets: egui::style::Widgets {
            noninteractive: state_widget(PANEL_BG),
            inactive: state_widget(PANEL_BG),
            hovered: state_widget(INPUT_BG),
            active: state_widget(INPUT_BG),
            open: state_widget(INPUT_BG),
        },
        striped: false,
        ..Default::default()
    };
    // Hovered/active strokes take the accent so interactive chrome reads
    // against the quiet surfaces.
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_BLUE);
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_palette_ladders_darkest_to_lightest() {
        let v = visuals();
        assert!(v.dark_mode);
        assert_eq!(v.hyperlink_color, ACCENT_BLUE);
        // Surfaces ladder: window < panel < elevated input.
        let lum = |c: Color32| u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b());
        assert!(lum(WINDOW_BG) < lum(PANEL_BG));
        assert!(lum(PANEL_BG) < lum(INPUT_BG));
    }

    #[test]
    fn selection_uses_accent_blue() {
        let v = visuals();
        assert_eq!(v.selection.stroke.color, ACCENT_BLUE);
        // The selection fill is the accent at low alpha; `Color32` stores
        // premultiplied values, so unmultiply before comparing. The round
        // trip through the premultiply LUT is exact modulo ±2 per channel.
        let [r, g, b, a] = v.selection.bg_fill.to_srgba_unmultiplied();
        assert!((r as i16 - ACCENT_BLUE.r() as i16).abs() <= 2);
        assert!((g as i16 - ACCENT_BLUE.g() as i16).abs() <= 2);
        assert!((b as i16 - ACCENT_BLUE.b() as i16).abs() <= 2);
        assert!(a < 255, "selection fill should be translucent");
        assert!(a > 0);
    }
}
