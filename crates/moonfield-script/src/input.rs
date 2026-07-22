//! Script-facing input state, mirrored from the world's `InputState`
//! resource each frame and shared with the `input_*` host functions.
//!
//! Two edge-latch views are maintained so `just_pressed` semantics stay
//! correct for both hook rates (this is the pitfall Bevy's #6183 hits with
//! a single per-frame latch):
//!
//! - **frame-latched** — edges valid for exactly one frame; used by
//!   `on_update` and `on_input` (Godot's "frame" scope).
//! - **step-latched** — edges accumulate until a fixed step consumes them;
//!   used by `on_fixed_update` (Godot's "physics tick" scope). A press is
//!   reported to exactly one fixed step, never duplicated across steps in
//!   one frame, and never lost when a frame runs zero steps.
//!
//! A focus loss clears both views, mirroring the world's `InputState`.

use moonfield_window::{CursorMode, InputEvent, InputState};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::script::{HostValue, ScriptApi};

/// Input state shared between the script plugin's update system (writer)
/// and the `input_*` host functions (readers).
#[derive(Debug, Default)]
pub struct ScriptInputState {
    pressed_keys: HashSet<String>,
    pressed_buttons: HashSet<String>,
    frame_just_pressed_keys: HashSet<String>,
    frame_just_released_keys: HashSet<String>,
    frame_just_pressed_buttons: HashSet<String>,
    frame_just_released_buttons: HashSet<String>,
    step_just_pressed_keys: HashSet<String>,
    step_just_released_keys: HashSet<String>,
    step_just_pressed_buttons: HashSet<String>,
    step_just_released_buttons: HashSet<String>,
    mouse_delta: (f64, f64),
    mouse_scroll: (f64, f64),
    mouse_position: (f64, f64),
    cursor_mode: CursorMode,
    /// Named action → bound key/button codes, registered from scripts via
    /// `input_bind_action`.
    actions: HashMap<String, Vec<String>>,
    /// Whether the currently running hook is a fixed step (selects the
    /// step-latched edge view for `just_pressed` queries).
    in_fixed_step: bool,
}

impl ScriptInputState {
    /// Mirror this frame's state from the world resource. Step-latched
    /// edges are merged in and persist until a fixed step consumes them.
    pub fn sync_frame(&mut self, input: &InputState) {
        // A focus loss wiped the world's pressed/edge sets (so keys can't
        // get stuck); the step-latched edges must follow, or a press
        // latched before the focus loss would surface in a later step.
        if input.events().contains(&InputEvent::FocusLost) {
            self.step_just_pressed_keys.clear();
            self.step_just_released_keys.clear();
            self.step_just_pressed_buttons.clear();
            self.step_just_released_buttons.clear();
        }
        sync_set(&mut self.pressed_keys, input.pressed_keys());
        sync_set(&mut self.pressed_buttons, input.pressed_buttons());
        sync_set(&mut self.frame_just_pressed_keys, input.just_pressed_keys());
        sync_set(
            &mut self.frame_just_released_keys,
            input.just_released_keys(),
        );
        sync_set(
            &mut self.frame_just_pressed_buttons,
            input.just_pressed_buttons(),
        );
        sync_set(
            &mut self.frame_just_released_buttons,
            input.just_released_buttons(),
        );
        self.step_just_pressed_keys
            .extend(input.just_pressed_keys().iter().cloned());
        self.step_just_released_keys
            .extend(input.just_released_keys().iter().cloned());
        self.step_just_pressed_buttons
            .extend(input.just_pressed_buttons().iter().cloned());
        self.step_just_released_buttons
            .extend(input.just_released_buttons().iter().cloned());
        self.mouse_delta = input.mouse_delta();
        self.mouse_scroll = input.mouse_scroll();
        self.mouse_position = input.mouse_position();
        self.cursor_mode = input.cursor_mode();
    }

    /// Mark the start of a fixed-step hook (`just_pressed` queries switch
    /// to the step-latched view).
    pub fn begin_fixed_step(&mut self) {
        self.in_fixed_step = true;
    }

    /// End a fixed-step hook: the step consumed this step's edges.
    pub fn end_fixed_step(&mut self) {
        self.in_fixed_step = false;
        self.step_just_pressed_keys.clear();
        self.step_just_released_keys.clear();
        self.step_just_pressed_buttons.clear();
        self.step_just_released_buttons.clear();
    }

    /// Leave the fixed-step context without consuming edges (used when the
    /// fixed hook is absent or errors before any step ran).
    pub fn cancel_fixed_step(&mut self) {
        self.in_fixed_step = false;
    }

    /// Is the key `code` currently held down?
    pub fn is_key_pressed(&self, code: &str) -> bool {
        self.pressed_keys.contains(code)
    }

    /// Is the mouse `button` currently held down?
    pub fn is_mouse_button_pressed(&self, button: &str) -> bool {
        self.pressed_buttons.contains(button)
    }

    /// Was the key `code` pressed in the current latch context?
    pub fn is_key_just_pressed(&self, code: &str) -> bool {
        if self.in_fixed_step {
            self.step_just_pressed_keys.contains(code)
        } else {
            self.frame_just_pressed_keys.contains(code)
        }
    }

    /// Was the key `code` released in the current latch context?
    pub fn is_key_just_released(&self, code: &str) -> bool {
        if self.in_fixed_step {
            self.step_just_released_keys.contains(code)
        } else {
            self.frame_just_released_keys.contains(code)
        }
    }

    /// Was the mouse `button` pressed in the current latch context?
    pub fn is_mouse_button_just_pressed(&self, button: &str) -> bool {
        if self.in_fixed_step {
            self.step_just_pressed_buttons.contains(button)
        } else {
            self.frame_just_pressed_buttons.contains(button)
        }
    }

    /// Was the mouse `button` released in the current latch context?
    pub fn is_mouse_button_just_released(&self, button: &str) -> bool {
        if self.in_fixed_step {
            self.step_just_released_buttons.contains(button)
        } else {
            self.frame_just_released_buttons.contains(button)
        }
    }

    /// Is `code` (key or mouse button) currently held down?
    fn is_pressed(&self, code: &str) -> bool {
        self.pressed_keys.contains(code) || self.pressed_buttons.contains(code)
    }

    /// Was `code` (key or mouse button) pressed in the current latch
    /// context? Used for action queries.
    fn is_just_pressed(&self, code: &str) -> bool {
        if self.in_fixed_step {
            self.step_just_pressed_keys.contains(code)
                || self.step_just_pressed_buttons.contains(code)
        } else {
            self.frame_just_pressed_keys.contains(code)
                || self.frame_just_pressed_buttons.contains(code)
        }
    }

    /// Cursor motion accumulated this frame, in logical pixels.
    pub fn mouse_delta(&self) -> (f64, f64) {
        self.mouse_delta
    }

    /// Scroll accumulated this frame, in lines.
    pub fn mouse_scroll(&self) -> (f64, f64) {
        self.mouse_scroll
    }

    /// Last reported absolute cursor position, in logical pixels.
    pub fn mouse_position(&self) -> (f64, f64) {
        self.mouse_position
    }

    /// Current cursor visibility / grab mode.
    pub fn cursor_mode(&self) -> CursorMode {
        self.cursor_mode
    }

    /// Bind a named action to a set of key/button codes (replaces any
    /// previous binding with the same name).
    pub fn bind_action(&mut self, name: String, codes: Vec<String>) {
        self.actions.insert(name, codes);
    }

    /// Is any code bound to `name` currently held?
    pub fn is_action_pressed(&self, name: &str) -> bool {
        self.actions
            .get(name)
            .is_some_and(|codes| codes.iter().any(|c| self.is_pressed(c)))
    }

    /// Was any code bound to `name` pressed in the current latch context?
    pub fn is_action_just_pressed(&self, name: &str) -> bool {
        self.actions
            .get(name)
            .is_some_and(|codes| codes.iter().any(|c| self.is_just_pressed(c)))
    }

    /// Signed axis from two actions: `+1` when `positive` is held, `-1`
    /// when `negative` is held, `0` for both or neither.
    pub fn axis(&self, negative: &str, positive: &str) -> f64 {
        let pos = self.is_action_pressed(positive) as i32;
        let neg = self.is_action_pressed(negative) as i32;
        (pos - neg) as f64
    }

    /// 2D vector from four actions, clamped to length 1 (circular
    /// normalization, Godot's `Input.get_vector`).
    pub fn vector(
        &self,
        x_negative: &str,
        x_positive: &str,
        y_negative: &str,
        y_positive: &str,
    ) -> (f64, f64) {
        let x = self.axis(x_negative, x_positive);
        let y = self.axis(y_negative, y_positive);
        let len = (x * x + y * y).sqrt();
        if len > 1.0 {
            (x / len, y / len)
        } else {
            (x, y)
        }
    }
}

/// Replace `dst` with a copy of `src`, skipping the clone when both are
/// already equal — held keys persist across frames, so re-cloning the same
/// `String`s every frame would be pure churn. (`clone_from` already reuses
/// the set's allocation; this also avoids re-cloning the elements.)
fn sync_set(dst: &mut HashSet<String>, src: &HashSet<String>) {
    if dst != src {
        dst.clone_from(src);
    }
}

/// Marshal an [`InputEvent`] into a `HostValue::Object` for the
/// `on_input(event)` script hook.
pub fn input_event_to_host(event: &InputEvent) -> HostValue {
    fn s(v: &str) -> HostValue {
        HostValue::String(v.to_string())
    }
    // Three entries is the largest payload (`mouse_motion`); pre-size so
    // high-frequency events (cursor moves fire per OS event) don't grow
    // the map repeatedly.
    let mut map = HashMap::with_capacity(3);
    match event {
        InputEvent::KeyPressed { code } => {
            map.insert("type".to_string(), s("key_pressed"));
            map.insert("code".to_string(), s(code));
        }
        InputEvent::KeyReleased { code } => {
            map.insert("type".to_string(), s("key_released"));
            map.insert("code".to_string(), s(code));
        }
        InputEvent::MouseButtonPressed { button } => {
            map.insert("type".to_string(), s("mouse_button_pressed"));
            map.insert("button".to_string(), s(button));
        }
        InputEvent::MouseButtonReleased { button } => {
            map.insert("type".to_string(), s("mouse_button_released"));
            map.insert("button".to_string(), s(button));
        }
        InputEvent::MouseMotion { dx, dy } => {
            map.insert("type".to_string(), s("mouse_motion"));
            map.insert("dx".to_string(), HostValue::Number(*dx));
            map.insert("dy".to_string(), HostValue::Number(*dy));
        }
        InputEvent::MouseWheel { dx, dy } => {
            map.insert("type".to_string(), s("mouse_wheel"));
            map.insert("dx".to_string(), HostValue::Number(*dx));
            map.insert("dy".to_string(), HostValue::Number(*dy));
        }
        InputEvent::FocusLost => {
            map.insert("type".to_string(), s("focus_lost"));
        }
    }
    HostValue::Object(map)
}

/// Shared handle to a [`ScriptInputState`]: the composition root passes
/// clones to both [`register_input_api`] and `ScriptPlugin::with_input_state`.
pub type SharedInputState = Arc<Mutex<ScriptInputState>>;

/// Create the shared input-state handle.
pub fn new_shared_input() -> SharedInputState {
    Arc::new(Mutex::new(ScriptInputState::default()))
}

/// Lock the shared input state, tolerating a poisoned mutex — a panicking
/// host function must not permanently break input polling.
fn lock(input: &SharedInputState) -> MutexGuard<'_, ScriptInputState> {
    input.lock().unwrap_or_else(|e| e.into_inner())
}

/// Extract a string argument.
fn arg_str(args: &[HostValue], i: usize) -> Result<&str, String> {
    args.get(i)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("arg {}: expected string", i))
}

/// Pack two numbers as a `[x, y]` array.
fn pair(x: f64, y: f64) -> HostValue {
    HostValue::Array(vec![HostValue::Number(x), HostValue::Number(y)])
}

/// Register one read-only `input_*` query plus its `.d.ts` declaration.
fn reg_query(
    api: &mut ScriptApi,
    input: &SharedInputState,
    name: &'static str,
    ts: &'static str,
    f: impl Fn(&ScriptInputState, &[HostValue]) -> Result<HostValue, String> + Send + Sync + 'static,
) {
    let handle = Arc::clone(input);
    api.register_closure(name, move |args| f(&lock(&handle), args));
    api.declare(ts);
}

/// Register the built-in `input_*` host functions: frame-latched polling
/// for held state, context-scoped edge queries (`just_pressed` is
/// frame-scoped in `on_update`, fixed-step-scoped in `on_fixed_update`),
/// mouse accumulators, and script-side named actions (Godot's InputMap
/// shape).
///
/// These are registered here (not in the composition root) because they
/// only read [`ScriptInputState`] — no engine-layer dependencies.
pub fn register_input_api(api: &mut ScriptApi, input: &SharedInputState) {
    reg_query(
        api,
        input,
        "input_is_key_pressed",
        "declare function input_is_key_pressed(code: string): boolean;",
        |s, args| Ok(HostValue::Bool(s.is_key_pressed(arg_str(args, 0)?))),
    );
    reg_query(
        api,
        input,
        "input_is_key_just_pressed",
        "declare function input_is_key_just_pressed(code: string): boolean;",
        |s, args| Ok(HostValue::Bool(s.is_key_just_pressed(arg_str(args, 0)?))),
    );
    reg_query(
        api,
        input,
        "input_is_key_just_released",
        "declare function input_is_key_just_released(code: string): boolean;",
        |s, args| Ok(HostValue::Bool(s.is_key_just_released(arg_str(args, 0)?))),
    );
    reg_query(
        api,
        input,
        "input_is_mouse_button_pressed",
        "declare function input_is_mouse_button_pressed(button: string): boolean;",
        |s, args| {
            Ok(HostValue::Bool(
                s.is_mouse_button_pressed(arg_str(args, 0)?),
            ))
        },
    );
    reg_query(
        api,
        input,
        "input_is_mouse_button_just_pressed",
        "declare function input_is_mouse_button_just_pressed(button: string): boolean;",
        |s, args| {
            Ok(HostValue::Bool(
                s.is_mouse_button_just_pressed(arg_str(args, 0)?),
            ))
        },
    );
    reg_query(
        api,
        input,
        "input_is_mouse_button_just_released",
        "declare function input_is_mouse_button_just_released(button: string): boolean;",
        |s, args| {
            Ok(HostValue::Bool(
                s.is_mouse_button_just_released(arg_str(args, 0)?),
            ))
        },
    );
    reg_query(
        api,
        input,
        "input_mouse_delta",
        "declare function input_mouse_delta(): [number, number];",
        |s, _| {
            let (x, y) = s.mouse_delta();
            Ok(pair(x, y))
        },
    );
    reg_query(
        api,
        input,
        "input_mouse_scroll",
        "declare function input_mouse_scroll(): [number, number];",
        |s, _| {
            let (x, y) = s.mouse_scroll();
            Ok(pair(x, y))
        },
    );
    reg_query(
        api,
        input,
        "input_mouse_position",
        "declare function input_mouse_position(): [number, number];",
        |s, _| {
            let (x, y) = s.mouse_position();
            Ok(pair(x, y))
        },
    );
    reg_query(
        api,
        input,
        "input_cursor_mode",
        "declare function input_cursor_mode(): \"normal\" | \"hidden\" | \"locked\";",
        |s, _| {
            let mode = match s.cursor_mode() {
                CursorMode::Normal => "normal",
                CursorMode::Hidden => "hidden",
                CursorMode::Locked => "locked",
            };
            Ok(HostValue::String(mode.to_string()))
        },
    );
    reg_query(
        api,
        input,
        "input_is_action_pressed",
        "declare function input_is_action_pressed(name: string): boolean;",
        |s, args| Ok(HostValue::Bool(s.is_action_pressed(arg_str(args, 0)?))),
    );
    reg_query(
        api,
        input,
        "input_is_action_just_pressed",
        "declare function input_is_action_just_pressed(name: string): boolean;",
        |s, args| Ok(HostValue::Bool(s.is_action_just_pressed(arg_str(args, 0)?))),
    );
    reg_query(
        api,
        input,
        "input_get_axis",
        "declare function input_get_axis(negative: string, positive: string): number;",
        |s, args| {
            Ok(HostValue::Number(
                s.axis(arg_str(args, 0)?, arg_str(args, 1)?),
            ))
        },
    );
    reg_query(
        api,
        input,
        "input_get_vector",
        "declare function input_get_vector(xNegative: string, xPositive: string, yNegative: string, yPositive: string): [number, number];",
        |s, args| {
            let (x, y) = s.vector(
                arg_str(args, 0)?,
                arg_str(args, 1)?,
                arg_str(args, 2)?,
                arg_str(args, 3)?,
            );
            Ok(pair(x, y))
        },
    );

    // Bindings are script-side state — this one needs mutable access.
    {
        let handle = Arc::clone(input);
        api.register_closure("input_bind_action", move |args| {
            let name = arg_str(args, 0)?.to_string();
            let codes = args
                .get(1)
                .and_then(|v| v.as_array())
                .ok_or_else(|| "arg 1: expected string array".to_string())?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            lock(&handle).bind_action(name, codes);
            Ok(HostValue::Null)
        });
        api.declare("declare function input_bind_action(name: string, codes: string[]): void;");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(input: &mut InputState, code: &str) {
        input.apply_event(InputEvent::KeyPressed {
            code: code.to_string(),
        });
    }

    #[test]
    fn step_edges_survive_until_consumed() {
        let mut world_input = InputState::default();
        press(&mut world_input, "Space");

        let mut input = ScriptInputState::default();
        input.sync_frame(&world_input);

        // Frame view: exactly one frame.
        assert!(input.is_just_pressed("Space"));
        input.sync_frame(&InputState::default()); // next frame, no new events
        assert!(!input.is_just_pressed("Space"));

        // Step view: still pending until a fixed step consumes it.
        assert!(input.step_just_pressed_keys.contains("Space"));
        input.begin_fixed_step();
        assert!(input.is_just_pressed("Space"));
        input.end_fixed_step();

        input.begin_fixed_step();
        assert!(!input.is_just_pressed("Space"));
        input.end_fixed_step();
    }

    #[test]
    fn focus_lost_clears_step_latched_edges() {
        let mut world_input = InputState::default();
        press(&mut world_input, "Space");

        let mut input = ScriptInputState::default();
        input.sync_frame(&world_input);
        assert!(input.step_just_pressed_keys.contains("Space"));

        // Focus is lost before any fixed step ran: the world wipes its
        // edge sets, and the step-latched edges must follow.
        let mut next_frame = InputState::default();
        next_frame.apply_event(InputEvent::FocusLost);
        input.sync_frame(&next_frame);

        input.begin_fixed_step();
        assert!(!input.is_key_just_pressed("Space"));
        input.end_fixed_step();

        // An edge arriving after the focus loss still latches normally.
        let mut frame = InputState::default();
        press(&mut frame, "Enter");
        input.sync_frame(&frame);
        input.begin_fixed_step();
        assert!(input.is_key_just_pressed("Enter"));
        input.end_fixed_step();
    }

    #[test]
    fn actions_axis_and_vector() {
        let mut input = ScriptInputState::default();
        input.bind_action("left".into(), vec!["KeyA".into()]);
        input.bind_action("right".into(), vec!["KeyD".into()]);
        input.bind_action("up".into(), vec!["KeyW".into()]);
        input.bind_action("down".into(), vec!["KeyS".into()]);
        assert_eq!(input.axis("left", "right"), 0.0);

        let mut world_input = InputState::default();
        press(&mut world_input, "KeyD");
        press(&mut world_input, "KeyS");
        input.sync_frame(&world_input);

        assert!(input.is_action_pressed("right"));
        assert!(input.is_action_just_pressed("right"));
        assert_eq!(input.axis("left", "right"), 1.0);
        // Diagonal input is clamped to length 1.
        let (x, y) = input.vector("left", "right", "up", "down");
        let len = (x * x + y * y).sqrt();
        assert!((len - 1.0).abs() < 1e-9);
        assert!(x > 0.0 && y > 0.0);
    }
}
