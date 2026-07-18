//! Script host API bindings for the moonfield application.
//!
//! This module is the composition root for scripting: the functions scripts
//! can call are assembled here, where both the scripting crate and the
//! render crate are in scope. Keeping the bindings out of `moonfield-script`
//! keeps that crate free of engine-layer dependencies.

use moonfield_render::HeadlessContext;
use moonfield_script::script::ScriptApi;

/// Build the host API exposed to scripts.
pub fn build_script_api() -> ScriptApi {
    let mut api = ScriptApi::new();
    api.register_fn::<record_frame_Fn>();
    api
}

/// `record_frame` host function: render one frame with the headless context.
///
/// Accepts optional `(width, height)` arguments; defaults to the headless
/// context's default resolution.
#[moonfield_script::script_function]
fn record_frame(width: Option<u32>, height: Option<u32>) -> Result<(), String> {
    let _ = (width, height);
    let ctx = HeadlessContext::record_frame().map_err(|e| e.to_string())?;
    drop(ctx);
    Ok(())
}

/// Fast-path `record_frame` that extracts `u32` args directly from V8
/// values, bypassing the `HostValue` marshaling chain.
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
fn direct_record_frame(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let _ = (
        if args.length() >= 1 {
            args.get(0).uint32_value(scope).unwrap_or(0)
        } else {
            0
        },
        if args.length() >= 2 {
            args.get(1).uint32_value(scope).unwrap_or(0)
        } else {
            0
        },
    );

    match HeadlessContext::record_frame() {
        Ok(ctx) => {
            drop(ctx);
            retval.set_undefined();
        }
        Err(e) => {
            scope.throw_exception(v8::String::new(scope, &e.to_string()).unwrap().into());
        }
    }
}

/// Install backend-specific fast-path host functions on a freshly created
/// script runtime.
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
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
        let api = build_script_api();
        let generated = api.generate_dts();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("scripts")
            .join("moonfield.d.ts");
        let on_disk = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
        assert_eq!(
            generated.trim_end(),
            on_disk.trim_end(),
            "scripts/moonfield.d.ts is out of sync; regenerate it from ScriptApi::generate_dts()"
        );
    }
}
