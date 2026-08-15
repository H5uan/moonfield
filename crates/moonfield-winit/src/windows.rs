//! Window entity ↔ winit window mapping and ECS→winit synchronization.
//!
//! Mirrors `bevy_winit`'s `WinitWindows` + `changed_windows`, minus change
//! detection: the sync runs a per-field diff against a [`CachedWindow`]
//! component (Bevy's own `CachedWindow` pattern), so consumers mutate the
//! [`Window`] component directly and never touch a dirty flag.

use moonfield_ecs::{Entity, World};
use moonfield_log::error;
use moonfield_window::{CursorMode, InputState, Window};
use std::collections::HashMap;
use std::sync::Arc;
use winit::window::{CursorGrabMode, Window as WinitWindowHandle, WindowId};

/// Maps window entities to their winit windows and back.
///
/// The architecture is multi-window-shaped (any number of entities), but
/// the current backend only ever creates the primary window.
#[derive(Default)]
pub struct WinitWindows {
    windows: HashMap<Entity, Arc<WinitWindowHandle>>,
    window_to_entity: HashMap<WindowId, Entity>,
}

impl WinitWindows {
    /// Register a created winit window for `entity`.
    pub fn insert(&mut self, entity: Entity, window: Arc<WinitWindowHandle>) {
        self.window_to_entity.insert(window.id(), entity);
        self.windows.insert(entity, window);
    }

    /// The winit window for `entity`, if created.
    pub fn get_window(&self, entity: Entity) -> Option<&Arc<WinitWindowHandle>> {
        self.windows.get(&entity)
    }

    /// The entity a winit [`WindowId`] belongs to.
    pub fn get_entity(&self, window_id: WindowId) -> Option<Entity> {
        self.window_to_entity.get(&window_id).copied()
    }

    /// All `(entity, window)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &Arc<WinitWindowHandle>)> {
        self.windows.iter().map(|(e, w)| (*e, w))
    }
}

/// Backend-side cache of the [`Window`] fields that were last applied to
/// the OS window.
///
/// Attached to the window entity by the backend at creation time. The
/// per-frame sync diffs the live [`Window`] component against this cache;
/// only changed fields are applied to the winit window, and the cache is
/// updated to match.
pub struct CachedWindow {
    title: String,
    cursor_mode: CursorMode,
}

impl CachedWindow {
    /// Initialize the cache from the window's state at creation time.
    pub fn new(window: &Window) -> Self {
        Self {
            title: window.title.clone(),
            cursor_mode: window.cursor_mode,
        }
    }
}

/// The fields of a [`Window`] component that changed since the last sync.
#[derive(Debug, Default, PartialEq)]
pub struct WindowDiff {
    /// New title, if it changed.
    pub title: Option<String>,
    /// New cursor mode, if it changed.
    pub cursor_mode: Option<CursorMode>,
}

/// Diff live [`Window`] field values against the [`CachedWindow`] cache,
/// advancing the cache to the live values. Pure — unit-testable without an
/// OS window. Resolution is excluded: it is owned by the backend (written
/// back on OS resize / DPI change), not by gameplay code.
pub fn diff_window(title: &str, cursor_mode: CursorMode, cache: &mut CachedWindow) -> WindowDiff {
    let title = if cache.title != title {
        let title = title.to_string();
        cache.title = title.clone();
        Some(title)
    } else {
        None
    };
    let cursor_mode = if cache.cursor_mode != cursor_mode {
        cache.cursor_mode = cursor_mode;
        Some(cursor_mode)
    } else {
        None
    };
    WindowDiff { title, cursor_mode }
}

/// Apply ECS-side [`Window`] component changes to the winit windows.
///
/// Runs once per frame after the app update.
pub fn sync_windows(world: &mut World) {
    // Snapshot the entity → window pairs first: resources borrow the world
    // immutably, and the component pass below needs `&mut World`.
    let pairs: Vec<(Entity, Arc<WinitWindowHandle>)> = match world.get_resource::<WinitWindows>() {
        Some(windows) => windows.iter().map(|(e, w)| (e, w.clone())).collect(),
        None => return,
    };

    for (entity, winit_window) in pairs {
        let Some((title, cursor_mode)) = world
            .get_component::<Window>(entity)
            .map(|w| (w.title.clone(), w.cursor_mode))
        else {
            continue;
        };
        let Some(diff) = world
            .get_component_mut::<CachedWindow>(entity)
            .map(|cache| diff_window(&title, cursor_mode, cache))
        else {
            continue;
        };

        if let Some(title) = diff.title {
            winit_window.set_title(&title);
        }
        if let Some(cursor_mode) = diff.cursor_mode {
            let (grab, visible) = match cursor_mode {
                CursorMode::Normal => (CursorGrabMode::None, true),
                CursorMode::Hidden => (CursorGrabMode::None, false),
                CursorMode::Locked => (CursorGrabMode::Locked, false),
            };
            if let Err(e) = winit_window.set_cursor_grab(grab) {
                error!("failed to set cursor grab mode: {e}");
            }
            winit_window.set_cursor_visible(visible);
            if let Some(mut input) = world.get_resource_mut::<InputState>() {
                input.set_cursor_mode(cursor_mode);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_window::WindowResolution;

    #[test]
    fn cached_window_diff_detects_changes() {
        let window = Window {
            title: "A".to_string(),
            resolution: WindowResolution::new(800, 600, 1.0),
            cursor_mode: CursorMode::Locked,
        };
        let mut cache = CachedWindow::new(&Window::default());

        let diff = diff_window(&window.title, window.cursor_mode, &mut cache);
        assert_eq!(diff.title.as_deref(), Some("A"));
        assert_eq!(diff.cursor_mode, Some(CursorMode::Locked));

        // Second diff with the same values reports no changes.
        let diff = diff_window(&window.title, window.cursor_mode, &mut cache);
        assert_eq!(diff, WindowDiff::default());
    }
}
