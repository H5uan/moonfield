//! Integration tests for the scripting runtime.

#[cfg(feature = "quickjs-backend")]
use moonfield_script::script::QuickJsRuntime as Runtime;
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
use moonfield_script::script::V8Runtime as Runtime;

use moonfield_script::script::{
    load_script, HostValue, HotReloadHandler, ModuleRegistry, ScriptApi, ScriptRuntime,
};
use std::path::PathBuf;
use std::rc::Rc;

/// Stub `record_frame` so test scripts can call it without a Vulkan device.
/// Also exercises the `#[script_function]` macro from a downstream crate
/// (generated code must use absolute `::moonfield_script::script` paths).
#[moonfield_script::script_function]
fn record_frame() -> Result<(), String> {
    Ok(())
}

fn test_api() -> ScriptApi {
    let mut api = ScriptApi::new();
    api.register_fn::<record_frame_Fn>();
    api
}

fn script_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("scripts")
}

fn unique_temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "moonfield_script_test_{}_{}",
        tag,
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn load_and_call_record_frame() {
    let path = script_dir().join("record_frame.js");
    let source = load_script(&path).expect("load script");

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load(path.to_string_lossy().as_ref(), &source)
        .expect("load into runtime");
    runtime.call("main").expect("call main");
}

#[test]
fn reload_changes_behavior() {
    let path = script_dir().join("record_frame.js");
    let source = load_script(&path).expect("load script");

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load(path.to_string_lossy().as_ref(), &source)
        .expect("load into runtime");

    // Reload with a different script that exports a counter.
    runtime.reload().expect("reload");
    runtime
        .load("counter.js", "function main() { return 42; }")
        .expect("load counter");
    runtime.call("main").expect("call reloaded main");
}

/// V8 backend requires TypeScript to be pre-compiled via tsc.
/// This test verifies that trying to load a `.ts` source directly gives a
/// helpful error (the `load_script` function will reject it before reaching V8).
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
#[test]
fn v8_rejects_raw_typescript() {
    let ts = "function main(): void { record_frame(); }";
    let mut runtime = Runtime::new(test_api()).expect("runtime");
    let result = runtime.load("main.ts", ts);
    assert!(result.is_err(), "V8 should reject raw TS source");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("SyntaxError")
            || msg.contains("unexpected token")
            || msg.contains("pre-compiled"),
        "error should mention syntax error or pre-compiled requirement, got: {msg}"
    );
}

/// V8 backend: load a simple module graph (no imports).
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
#[test]
fn v8_module_graph_simple() {
    let mut registry = ModuleRegistry::new();
    registry.register(
        "main",
        "export function main() { console.log('module works'); }".to_string(),
    );

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load_module_graph(Rc::new(registry), "main")
        .expect("load_module_graph should succeed");
}

/// V8 backend: load a module graph with a function import.
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
#[test]
fn v8_module_graph_with_imports() {
    let mut registry = ModuleRegistry::new();
    registry.register(
        "main",
        "import { greet } from \"./utils\";\n\
         export function main() { greet(); }"
            .to_string(),
    );
    registry.register(
        "./utils",
        "export function greet() { console.log('hello from module!'); }".to_string(),
    );

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load_module_graph(Rc::new(registry), "main")
        .expect("load_module_graph should succeed");
}

/// QuickJS backend needs swc-based transpilation for TypeScript.
#[cfg(feature = "quickjs-backend")]
#[test]
fn transpile_strips_typescript_annotations() {
    let ts = "function main(): void {\n    const x: number = 42;\n    const y = x as number;\n    record_frame();\n}";
    let js = moonfield_script::script::transpile_typescript(ts).expect("transpile");

    // Type annotations must be gone.
    assert!(!js.contains(": void"));
    assert!(!js.contains(": number"));
    assert!(!js.contains("as number"));

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load("main.ts", &js)
        .expect("load transpiled script");
    runtime.call("main").expect("call transpiled main");
}

#[test]
fn console_binding_is_callable() {
    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load(
            "console.js",
            "function main() {\n    console.log('hello', 42);\n    console.warn('careful');\n    console.error('oops');\n}",
        )
        .expect("load console script");
    // console.* should run without throwing.
    runtime.call("main").expect("console calls succeed");
}

#[test]
fn runtime_error_reports_useful_message() {
    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load("err.js", "function main() { callMissingThing(); }")
        .expect("load error script");

    let err = runtime.call("main");
    assert!(err.is_err(), "calling an undefined function must error");
    let msg = err.unwrap_err().to_string();
    // The message must carry the actual engine exception text, not a generic
    // "failed to run script".
    assert!(
        msg.contains("not defined") || msg.contains("callMissingThing"),
        "error message should mention the missing symbol, got: {msg}"
    );
}

/// Call a module export with arguments and get a typed result back.
#[test]
fn module_export_call_with_args_roundtrip() {
    let mut registry = ModuleRegistry::new();
    registry.register(
        "main",
        "export function add(a, b) { return a + b; }".to_string(),
    );

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load_module_graph(Rc::new(registry), "main")
        .expect("load_module_graph");

    let v = runtime
        .call_module_export("add", &[HostValue::Number(20.0), HostValue::Number(22.0)])
        .expect("add");
    assert_eq!(v.as_f64(), Some(42.0));
}

/// `has_function` detects optional lifecycle hooks, and `call_module_export`
/// drives them with arguments.
#[test]
fn has_function_and_lifecycle_hooks() {
    let mut registry = ModuleRegistry::new();
    registry.register(
        "main",
        "let ticks = 0;\n\
         export function main() {}\n\
         export function on_update(dt) { ticks += dt; }\n\
         export function getTicks() { return ticks; }"
            .to_string(),
    );

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load_module_graph(Rc::new(registry), "main")
        .expect("load_module_graph");

    assert!(runtime.has_function("on_update"));
    assert!(runtime.has_function("getTicks"));
    assert!(!runtime.has_function("does_not_exist"));

    runtime
        .call_module_export("on_update", &[HostValue::Number(0.5)])
        .expect("on_update");
    let v = runtime
        .call_module_export("getTicks", &[])
        .expect("getTicks");
    assert_eq!(v.as_f64(), Some(0.5));

    // The unit (fire-and-forget) variant must drive the hook the same way.
    runtime
        .call_module_export_unit("on_update", &[HostValue::Number(0.25)])
        .expect("on_update unit");
    let v = runtime
        .call_module_export("getTicks", &[])
        .expect("getTicks");
    assert_eq!(v.as_f64(), Some(0.75));
}

/// Host function errors and panics must surface as catchable JS exceptions —
/// errors are thrown, and panics are caught at the FFI boundary instead of
/// unwinding through the engine's C/C++ frames (which would be UB).
#[test]
fn host_fn_failures_surface_as_js_exceptions() {
    let mut api = test_api();
    api.register_closure("fail_always", |_| Err("boom".to_string()));
    api.register_closure("panic_always", |_| panic!("bang"));

    let mut runtime = Runtime::new(api).expect("runtime");
    runtime
        .load(
            "host_err.js",
            "function tryFail() { try { fail_always(); return 'no-throw'; } catch (e) { return 'caught'; } }\n\
             function tryPanic() { try { panic_always(); return 'no-throw'; } catch (e) { return 'caught'; } }",
        )
        .expect("load");

    let v = runtime.call_with_args("tryFail", &[]).expect("tryFail");
    assert_eq!(v.as_str(), Some("caught"));
    let v = runtime.call_with_args("tryPanic", &[]).expect("tryPanic");
    assert_eq!(v.as_str(), Some("caught"));

    // The runtime stays usable after a host panic.
    let v = runtime
        .call_with_args("tryFail", &[])
        .expect("call after panic");
    assert_eq!(v.as_str(), Some("caught"));
}

/// Hot reload: editing a dependency on disk recompiles it and its importers,
/// and the new behavior is observable immediately.
#[test]
fn hot_reload_recompiles_changed_module() {
    let dir = unique_temp_dir("hot_reload_basic");
    let utils_path = dir.join("utils.js");
    std::fs::write(&utils_path, "export function value() { return 1; }").unwrap();

    let mut registry = ModuleRegistry::new();
    registry.register(
        "main",
        "import { value } from \"./utils\";\n\
         export function mainValue() { return value(); }"
            .to_string(),
    );
    let utils_source = std::fs::read_to_string(&utils_path).unwrap();
    registry.register("./utils", utils_source);

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load_module_graph(Rc::new(registry), "main")
        .expect("load_module_graph");

    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call mainValue");
    assert_eq!(v.as_f64(), Some(1.0));

    // Change the dependency on disk and trigger a hot reload.
    std::fs::write(&utils_path, "export function value() { return 2; }").unwrap();
    runtime
        .on_files_changed(std::slice::from_ref(&utils_path))
        .expect("hot reload");

    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call after reload");
    assert_eq!(v.as_f64(), Some(2.0));

    std::fs::remove_dir_all(&dir).ok();
}

/// Hot reload: a broken edit must fail without killing the runtime — the
/// previously evaluated code stays callable, and a later fix reloads fine.
#[test]
fn hot_reload_recovers_after_broken_edit() {
    let dir = unique_temp_dir("hot_reload_recovery");
    let utils_path = dir.join("utils.js");
    std::fs::write(&utils_path, "export function value() { return 1; }").unwrap();

    let mut registry = ModuleRegistry::new();
    registry.register(
        "main",
        "import { value } from \"./utils\";\n\
         export function mainValue() { return value(); }"
            .to_string(),
    );
    let utils_source = std::fs::read_to_string(&utils_path).unwrap();
    registry.register("./utils", utils_source);

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load_module_graph(Rc::new(registry), "main")
        .expect("load_module_graph");

    // Break the dependency: reload must fail but keep old code alive.
    std::fs::write(&utils_path, "export function value( { return 3; }").unwrap();
    let result = runtime.on_files_changed(std::slice::from_ref(&utils_path));
    assert!(result.is_err(), "broken source must fail to reload");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("old code still callable");
    assert_eq!(v.as_f64(), Some(1.0));

    // Fix it: the next reload recovers and picks up the new behavior.
    std::fs::write(&utils_path, "export function value() { return 2; }").unwrap();
    runtime
        .on_files_changed(std::slice::from_ref(&utils_path))
        .expect("reload after fix");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call after recovery");
    assert_eq!(v.as_f64(), Some(2.0));

    std::fs::remove_dir_all(&dir).ok();
}

/// Promises settled during a script call must have their `.then` callbacks
/// run before the call returns (microtask checkpoint).
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
#[test]
fn v8_microtasks_run_after_call() {
    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load(
            "promise.js",
            "globalThis.flag = 0;\n\
             function main() { Promise.resolve().then(() => { globalThis.flag = 42; }); }\n\
             function readFlag() { return globalThis.flag; }",
        )
        .expect("load");
    runtime.call("main").expect("main");
    let v = runtime.call_with_args("readFlag", &[]).expect("readFlag");
    assert_eq!(v.as_f64(), Some(42.0));
}

/// With a sibling `.js.map` present, error locations are remapped from the
/// compiled JS back to the original TypeScript positions.
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
#[test]
fn v8_error_locations_remap_to_typescript() {
    let dir = unique_temp_dir("sourcemap");
    let js_path = dir.join("main.js");
    std::fs::write(
        &js_path,
        "function main() {\n    throw new Error('boom');\n}\n\
         //# sourceMappingURL=main.js.map\n",
    )
    .unwrap();

    // Map generated line 2 (0-based 1) from column 0 back to
    // main.ts line 42 (0-based 41), col 5 (0-based 4).
    let mut builder = sourcemap::SourceMapBuilder::new(Some("main.js"));
    builder.add(1, 0, 41, 4, Some("main.ts"), None, false);
    let map = builder.into_sourcemap();
    let mut map_bytes = Vec::new();
    map.to_writer(&mut map_bytes).unwrap();
    std::fs::write(dir.join("main.js.map"), &map_bytes).unwrap();

    let source = std::fs::read_to_string(&js_path).unwrap();
    let name = js_path.to_string_lossy().replace('\\', "/");
    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime.load(&name, &source).expect("load");

    let err = runtime.call("main");
    let msg = err.expect_err("main must throw").to_string();
    assert!(
        msg.contains("main.ts:42"),
        "error should be remapped to the TS location, got:\n{}",
        msg
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A runaway script (infinite loop) must be terminated by the execution
/// watchdog, and the runtime must stay usable for subsequent calls.
#[test]
fn runaway_script_is_terminated_and_recovers() {
    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime.set_execution_timeout(std::time::Duration::from_millis(100));
    runtime
        .load(
            "loop.js",
            "function main() { while (true) {} }\nfunction ok() { return 7; }",
        )
        .expect("load");

    let err = runtime.call("main");
    assert!(err.is_err(), "infinite loop must be terminated");

    // The runtime recovers for subsequent calls.
    let v = runtime
        .call_with_args("ok", &[])
        .expect("call after termination");
    assert_eq!(v.as_f64(), Some(7.0));
}

/// Host values with non-finite floats and typed arrays must round-trip
/// through the call boundary without corruption.
#[test]
fn call_with_args_preserves_special_values() {
    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load(
            "roundtrip.js",
            "function classify(n) {\n\
                 if (Number.isNaN(n)) return 'nan';\n\
                 if (n === Infinity) return 'inf';\n\
                 if (n === -Infinity) return '-inf';\n\
                 return 'finite';\n\
             }\n\
             function sumBytes(arr) { let s = 0; for (const b of arr) s += b; return s; }",
        )
        .expect("load");

    for (input, want) in [
        (f64::NAN, "nan"),
        (f64::INFINITY, "inf"),
        (f64::NEG_INFINITY, "-inf"),
        (1.5, "finite"),
    ] {
        let v = runtime
            .call_with_args("classify", &[HostValue::Number(input)])
            .expect("classify");
        assert_eq!(v.as_str(), Some(want), "classify({input})");
    }

    let v = runtime
        .call_with_args(
            "sumBytes",
            &[HostValue::TypedArray(
                moonfield_script::script::TypedArrayValue::Uint8(vec![1, 2, 3, 4]),
            )],
        )
        .expect("sumBytes");
    assert_eq!(v.as_f64(), Some(10.0));
}
