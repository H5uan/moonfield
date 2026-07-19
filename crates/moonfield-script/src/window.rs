//! Window lifecycle events and the `app_*` host functions.
//!
//! Window events (`close_requested`, `resized`, `focus_gained`,
//! `focus_lost`) travel on their own channel — they are app lifecycle
//! signals, not gameplay input (see Godot's `NOTIFICATION_WM_*` / Unreal's
//! `FCoreDelegates` for the same split). The backend queues them into the
//! `WindowEvents` world resource; the script plugin replays them to the
//! `on_window_event` hook each frame.
//!
//! Exit policy (Godot's `auto_accept_quit`): by default the backend exits
//! immediately on `CloseRequested`. Scripts call
//! `app_set_auto_exit_on_close(false)` to receive `close_requested` events
//! instead, then call `app_exit()` when actually ready to quit.

use moonfield_window::{WindowControl, WindowEventKind};
use std::collections::HashMap;

use crate::script::{HostValue, ScriptApi};

/// Marshal a [`WindowEventKind`] into a `HostValue::Object` for the
/// `on_window_event(event)` script hook.
pub fn window_event_to_host(event: &WindowEventKind) -> HostValue {
    fn s(v: &str) -> HostValue {
        HostValue::String(v.to_string())
    }
    let mut map = HashMap::new();
    match event {
        WindowEventKind::CloseRequested => {
            map.insert("type".to_string(), s("close_requested"));
        }
        WindowEventKind::Resized { width, height } => {
            map.insert("type".to_string(), s("resized"));
            map.insert("width".to_string(), HostValue::Number(*width as f64));
            map.insert("height".to_string(), HostValue::Number(*height as f64));
        }
        WindowEventKind::FocusGained => {
            map.insert("type".to_string(), s("focus_gained"));
        }
        WindowEventKind::FocusLost => {
            map.insert("type".to_string(), s("focus_lost"));
        }
    }
    HostValue::Object(map)
}

/// Register the built-in `app_*` window-control host functions.
///
/// These only touch the shared [`WindowControl`] signals — no engine-layer
/// dependencies, so they live here rather than in the composition root.
pub fn register_window_api(api: &mut ScriptApi, control: &WindowControl) {
    {
        let control = control.clone();
        api.register_closure("app_exit", move |_args| {
            control.request_exit();
            Ok(HostValue::Null)
        });
        api.declare("declare function app_exit(): void;");
    }
    {
        let control = control.clone();
        api.register_closure("app_set_auto_exit_on_close", move |args| {
            let enabled = args
                .first()
                .and_then(|v| v.as_bool())
                .ok_or_else(|| "arg 0: expected bool".to_string())?;
            control.set_auto_exit_on_close(enabled);
            Ok(HostValue::Null)
        });
        api.declare("declare function app_set_auto_exit_on_close(enabled: boolean): void;");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_control_roundtrip_through_host_fns() {
        let control = WindowControl::default();
        let mut api = ScriptApi::new();
        register_window_api(&mut api, &control);

        let mut fns: HashMap<&str, &crate::script::HostFn> = HashMap::new();
        for entry in api.iter() {
            fns.insert(entry.0, &entry.1);
        }

        // Default: auto-exit on, no exit requested.
        assert!(control.auto_exit_on_close());
        assert!(!control.exit_requested());

        // Script takes over close handling, then asks to quit.
        fns["app_set_auto_exit_on_close"](&[HostValue::Bool(false)]).unwrap();
        assert!(!control.auto_exit_on_close());
        fns["app_exit"](&[]).unwrap();
        assert!(control.exit_requested());

        // Arg validation.
        assert!(fns["app_set_auto_exit_on_close"](&[HostValue::Number(1.0)]).is_err());
    }
}
