//! Script host API bindings for the moonfield application.
//!
//! This module is the composition root for scripting: the functions scripts
//! can call are assembled here, where both the scripting crate and the
//! render crate are in scope. Keeping the bindings out of `moonfield-script`
//! keeps that crate free of engine-layer dependencies.

use moonfield_render::HeadlessContext;
use moonfield_script::input::{register_input_api, SharedInputState};
use moonfield_script::register_window_api;
use moonfield_script::script::ScriptApi;
use moonfield_script::time::{register_time_api, SharedTimeState};
use moonfield_window::{SharedWindow, WindowControl, WindowRequests};
use std::cell::RefCell;

thread_local! {
    /// Building the headless context means creating a Vulkan instance and
    /// device, compiling the shaders, and building the pipeline — far too
    /// expensive to repeat per call (hot reload re-runs `main()`, and
    /// scripts may call `record_frame` every frame). It is created lazily
    /// on first use and reused from then on, re-created only when the
    /// requested resolution changes.
    static HEADLESS_CONTEXT: RefCell<Option<HeadlessContext>> = const { RefCell::new(None) };
}

/// Default resolution when a script calls `record_frame` without arguments.
const DEFAULT_WIDTH: u32 = 800;
const DEFAULT_HEIGHT: u32 = 600;

/// Initialize the shared headless context on first use; subsequent calls
/// with the same resolution are cheap no-ops, and a different resolution
/// re-creates the context. A failed attempt is not cached — the next call
/// retries.
fn ensure_headless_context(width: u32, height: u32) -> Result<(), String> {
    HEADLESS_CONTEXT.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.as_ref().map(HeadlessContext::extent) != Some((width, height)) {
            *slot = Some(HeadlessContext::record_frame(width, height).map_err(|e| e.to_string())?);
        }
        Ok(())
    })
}

/// Build the host API exposed to scripts.
pub fn build_script_api(
    input: &SharedInputState,
    time: &SharedTimeState,
    window_control: &WindowControl,
    window: &SharedWindow,
    window_requests: &WindowRequests,
) -> ScriptApi {
    let mut api = ScriptApi::new();
    api.register_fn::<record_frame_Fn>();
    register_input_api(&mut api, input);
    register_time_api(&mut api, time);
    register_window_api(&mut api, window_control, window, window_requests);
    api
}

/// `record_frame` host function: render one frame with the headless context.
///
/// **Debug/headless tool only** — scripts must not own or drive GPU objects
/// (see "Threading Model" in AGENTS.md). Gameplay-facing host APIs should
/// keep GPU work behind the logic-thread/render-thread handoff instead of
/// following this pattern.
///
/// Accepts optional `(width, height)` arguments, defaulting to 800×600.
/// The context is built once and reused, or re-built when the requested
/// resolution changes (see [`ensure_headless_context`]).
#[moonfield_script::script_function]
fn record_frame(width: Option<u32>, height: Option<u32>) -> Result<(), String> {
    ensure_headless_context(
        width.unwrap_or(DEFAULT_WIDTH),
        height.unwrap_or(DEFAULT_HEIGHT),
    )
}

/// Fast-path `record_frame` that extracts `u32` args directly from V8
/// values, bypassing the `HostValue` marshaling chain.
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
fn direct_record_frame(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let width = if args.length() >= 1 {
        args.get(0).uint32_value(scope).unwrap_or(0)
    } else {
        DEFAULT_WIDTH
    };
    let height = if args.length() >= 2 {
        args.get(1).uint32_value(scope).unwrap_or(0)
    } else {
        DEFAULT_HEIGHT
    };

    match ensure_headless_context(width, height) {
        Ok(()) => {
            retval.set_undefined();
        }
        Err(e) => {
            scope.throw_exception(v8::String::new(scope, &e).unwrap().into());
        }
    }
}

/// Fast-path `record_frame` that extracts `u32` args directly from QuickJS
/// values, bypassing the `HostValue` marshaling chain.
#[cfg(feature = "quickjs-backend")]
fn direct_record_frame(
    ctx: rquickjs::Ctx,
    args: rquickjs::function::Rest<rquickjs::Value>,
) -> rquickjs::Result<()> {
    let width = args
        .0
        .first()
        .and_then(|v| v.as_int())
        .map(|i| i.max(0) as u32)
        .unwrap_or(DEFAULT_WIDTH);
    let height = args
        .0
        .get(1)
        .and_then(|v| v.as_int())
        .map(|i| i.max(0) as u32)
        .unwrap_or(DEFAULT_HEIGHT);

    ensure_headless_context(width, height)
        .map_err(|e| rquickjs::Exception::throw_message(&ctx, &e))?;
    Ok(())
}

/// Install backend-specific fast-path host functions on a freshly created
/// script runtime.
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
pub fn configure_runtime(rt: &mut moonfield_script::Runtime) {
    rt.register_direct("record_frame", direct_record_frame);
}

#[cfg(feature = "quickjs-backend")]
pub fn configure_runtime(rt: &mut moonfield_script::Runtime) {
    rt.register_direct("record_frame", direct_record_frame);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checked-in `scripts/moonfield.d.ts` must match what the host API
    /// generates, so IDE autocomplete never drifts from the real bindings.
    #[test]
    fn dts_matches_registered_api() {
        let api = build_script_api(
            &moonfield_script::new_shared_input(),
            &moonfield_script::new_shared_time(),
            &WindowControl::default(),
            &moonfield_window::new_shared_window(),
            &WindowRequests::default(),
        );
        let generated = api.generate_dts();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("scripts")
            .join("moonfield.d.ts");
        // Regenerate the checked-in file with:
        // `MOONFIELD_UPDATE_DTS=1 cargo test -p moonfield dts`.
        if std::env::var_os("MOONFIELD_UPDATE_DTS").is_some() {
            std::fs::write(&path, &generated)
                .unwrap_or_else(|e| panic!("failed to write {}: {}", path.display(), e));
        }
        let on_disk = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
        // Normalize line endings so the test passes regardless of git's
        // `core.autocrlf` setting on Windows.
        let normalize = |s: &str| s.replace("\r\n", "\n");
        assert_eq!(
            normalize(generated.trim_end()),
            normalize(on_disk.trim_end()),
            "scripts/moonfield.d.ts is out of sync; regenerate it from ScriptApi::generate_dts()"
        );
    }
}
