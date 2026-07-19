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

/// Feeding *raw* TypeScript straight into `runtime.load` (bypassing
/// `load_script`'s transpilation) must still fail — the engine itself
/// never learned TS syntax.
#[cfg(all(feature = "v8-backend", not(feature = "quickjs-backend")))]
#[test]
fn v8_rejects_untranspiled_typescript() {
    let ts = "function main(): void { record_frame(); }";
    let mut runtime = Runtime::new(test_api()).expect("runtime");
    let result = runtime.load("main.ts", ts);
    assert!(result.is_err(), "V8 should reject raw TS source");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("SyntaxError") || msg.contains("unexpected token"),
        "error should mention a syntax error, got: {msg}"
    );
}

/// A `.ts` entry with no pre-compiled `.js` alongside is transpiled
/// in-process (swc) and runs on both backends — no `tsc` step needed.
#[test]
fn loads_typescript_entry_without_precompiled_js() {
    let dir = unique_temp_dir("ts_entry");
    let ts_path = dir.join("main.ts");
    std::fs::write(
        &ts_path,
        "export function value(): number { const n: number = 42; return n; }",
    )
    .unwrap();

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    moonfield_script::load_module_entry(&mut runtime, &ts_path).expect("load ts entry");
    let v = runtime
        .call_module_export("value", &[])
        .expect("call value");
    assert_eq!(v.as_f64(), Some(42.0));

    std::fs::remove_dir_all(&dir).ok();
}

/// The real on-disk flow: an entry importing a sibling module must have the
/// dependency discovered by `resolve_dependencies`, loaded, registered
/// under a canonical name, and resolvable by the engine at instantiation.
#[test]
fn loads_module_entry_with_relative_import() {
    let dir = unique_temp_dir("entry_import");
    let main_path = dir.join("main.js");
    let utils_path = dir.join("utils.js");
    std::fs::write(&utils_path, "export function value() { return 7; }").unwrap();
    std::fs::write(
        &main_path,
        "import { value } from \"./utils.js\";\n\
         export function mainValue() { return value(); }",
    )
    .unwrap();

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    moonfield_script::load_module_entry(&mut runtime, &main_path).expect("load entry with import");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call mainValue");
    assert_eq!(v.as_f64(), Some(7.0));

    std::fs::remove_dir_all(&dir).ok();
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

/// swc strips TypeScript type annotations (the transform `load_script`
/// falls back to on both backends when no pre-compiled `.js` exists).
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

/// Hot reload: a changed module that gains a NEW import must have that
/// dependency discovered, loaded from disk, and registered during the
/// reload — previously this failed with "module not found".
#[test]
fn hot_reload_picks_up_new_import() {
    let dir = unique_temp_dir("hot_reload_new_import");
    let main_path = dir.join("main.js");
    let utils_path = dir.join("utils.js");
    std::fs::write(&utils_path, "export function value() { return 1; }").unwrap();
    std::fs::write(&main_path, "export function mainValue() { return 0; }").unwrap();

    let mut registry = ModuleRegistry::new();
    let main_source = std::fs::read_to_string(&main_path).unwrap();
    let canonical = registry.register(&main_path.to_string_lossy().replace('\\', "/"), main_source);
    registry
        .resolve_dependencies(&canonical)
        .expect("resolve deps");

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load_module_graph(Rc::new(registry), &canonical)
        .expect("load_module_graph");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call mainValue");
    assert_eq!(v.as_f64(), Some(0.0));

    // Edit main.js to import the previously unimported utils.js.
    std::fs::write(
        &main_path,
        "import { value } from \"./utils.js\";\n\
         export function mainValue() { return value(); }",
    )
    .unwrap();
    runtime
        .on_files_changed(std::slice::from_ref(&main_path))
        .expect("hot reload with new import");

    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call after reload");
    assert_eq!(v.as_f64(), Some(1.0));

    std::fs::remove_dir_all(&dir).ok();
}

/// Hot reload: editing a `.ts` file re-transpiles it in-process and the new
/// behavior is observable immediately — no `tsc -w` step required.
#[test]
fn hot_reload_transpiles_changed_typescript() {
    let dir = unique_temp_dir("hot_reload_ts");
    let utils_path = dir.join("utils.ts");
    std::fs::write(&utils_path, "export function value(): number { return 1; }").unwrap();

    let mut registry = ModuleRegistry::new();
    registry.register(
        "main",
        "import { value } from \"./utils\";\n\
         export function mainValue() { return value(); }"
            .to_string(),
    );
    // Initial registration goes through the same transpile path as the loader.
    let utils_js = load_script(&utils_path).expect("transpile utils.ts");
    registry.register("./utils", utils_js);

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    runtime
        .load_module_graph(Rc::new(registry), "main")
        .expect("load_module_graph");

    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call mainValue");
    assert_eq!(v.as_f64(), Some(1.0));

    // Edit the TypeScript source on disk and trigger a hot reload.
    std::fs::write(&utils_path, "export function value(): number { return 2; }").unwrap();
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

/// Hot reload: deleting a module file must surface as an error (the file
/// no longer reads) instead of silently keeping its last code — the
/// watcher forwards removals, and the backend reports the failed read
/// while the previously evaluated code keeps running.
#[test]
fn hot_reload_surfaces_deleted_module() {
    let dir = unique_temp_dir("hot_reload_delete");
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

    // Delete the dependency and trigger a hot reload: the reload must
    // fail loudly, not keep the stale code running silently.
    std::fs::remove_file(&utils_path).unwrap();
    let result = runtime.on_files_changed(std::slice::from_ref(&utils_path));
    let err = result.expect_err("deleting a module file must surface an error");
    assert!(
        err.to_string().contains("failed to read script"),
        "error should report the unreadable file, got: {err}"
    );

    // The previously evaluated code is still alive, and recreating the
    // file recovers through the same path the pending-retry uses.
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("old code still callable");
    assert_eq!(v.as_f64(), Some(1.0));

    std::fs::write(&utils_path, "export function value() { return 2; }").unwrap();
    runtime
        .on_files_changed(std::slice::from_ref(&utils_path))
        .expect("reload after recreate");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call after recreate");
    assert_eq!(v.as_f64(), Some(2.0));

    std::fs::remove_dir_all(&dir).ok();
}

/// A `.ts` file is always transpiled from its own source: a stale
/// pre-compiled `.js` sitting next to it must never shadow it — neither on
/// the initial load nor on hot reload.
#[test]
fn ts_entry_ignores_stale_sibling_js() {
    let dir = unique_temp_dir("stale_sibling_js");
    let ts_path = dir.join("main.ts");
    std::fs::write(&ts_path, "export function value(): number { return 2; }").unwrap();
    // Stale tsc output from before the edit — must NOT be loaded.
    std::fs::write(dir.join("main.js"), "export function value() { return 1; }").unwrap();

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    moonfield_script::load_module_entry(&mut runtime, &ts_path).expect("load ts entry");
    let v = runtime
        .call_module_export("value", &[])
        .expect("call value");
    assert_eq!(
        v.as_f64(),
        Some(2.0),
        "stale sibling .js must not shadow the .ts source"
    );

    // Editing the .ts hot-reloads the NEW source, not the stale .js.
    std::fs::write(&ts_path, "export function value(): number { return 3; }").unwrap();
    runtime
        .on_files_changed(std::slice::from_ref(&ts_path))
        .expect("hot reload");
    let v = runtime
        .call_module_export("value", &[])
        .expect("call after reload");
    assert_eq!(v.as_f64(), Some(3.0));

    std::fs::remove_dir_all(&dir).ok();
}

/// An extension-less import (`"./utils"`) with BOTH a fresh `.ts` and a
/// stale `.js` sibling on disk resolves to the `.ts` — on the initial load
/// and on hot reload, where a change event on the stale `.js` reloads the
/// shadowing `.ts` instead of resurrecting the stale code.
#[test]
fn extensionless_import_prefers_ts_over_stale_js() {
    let dir = unique_temp_dir("extensionless_ts_wins");
    let main_path = dir.join("main.ts");
    let ts_path = dir.join("utils.ts");
    let js_path = dir.join("utils.js");
    std::fs::write(&ts_path, "export function value(): number { return 2; }").unwrap();
    // Stale tsc output from before the edit — must NOT be loaded.
    std::fs::write(&js_path, "export function value() { return 1; }").unwrap();
    std::fs::write(
        &main_path,
        "import { value } from \"./utils\";\n\
         export function mainValue() { return value(); }",
    )
    .unwrap();

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    moonfield_script::load_module_entry(&mut runtime, &main_path).expect("load entry");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call mainValue");
    assert_eq!(
        v.as_f64(),
        Some(2.0),
        "extension-less import must resolve the .ts, not the stale .js"
    );

    // A change event on the stale .js reloads the shadowing .ts — the edit
    // to the .js must not resurrect it.
    std::fs::write(&js_path, "export function value() { return 3; }").unwrap();
    runtime
        .on_files_changed(std::slice::from_ref(&js_path))
        .expect("hot reload from shadowed .js event");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call after .js event");
    assert_eq!(
        v.as_f64(),
        Some(2.0),
        "shadowed .js change must reload the .ts source of truth"
    );

    // Editing the .ts itself hot-reloads the new source as usual.
    std::fs::write(&ts_path, "export function value(): number { return 4; }").unwrap();
    runtime
        .on_files_changed(std::slice::from_ref(&ts_path))
        .expect("hot reload from .ts event");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call after .ts event");
    assert_eq!(v.as_f64(), Some(4.0));

    std::fs::remove_dir_all(&dir).ok();
}

/// A `.js` module with no `.ts` sibling keeps working unchanged: the
/// extension-less import resolves to it and it hot-reloads normally.
#[test]
fn extensionless_import_js_only_resolves_and_hot_reloads() {
    let dir = unique_temp_dir("extensionless_js_only");
    let main_path = dir.join("main.js");
    let js_path = dir.join("utils.js");
    std::fs::write(&js_path, "export function value() { return 1; }").unwrap();
    std::fs::write(
        &main_path,
        "import { value } from \"./utils\";\n\
         export function mainValue() { return value(); }",
    )
    .unwrap();

    let mut runtime = Runtime::new(test_api()).expect("runtime");
    moonfield_script::load_module_entry(&mut runtime, &main_path).expect("load entry");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call mainValue");
    assert_eq!(v.as_f64(), Some(1.0));

    std::fs::write(&js_path, "export function value() { return 2; }").unwrap();
    runtime
        .on_files_changed(std::slice::from_ref(&js_path))
        .expect("hot reload");
    let v = runtime
        .call_module_export("mainValue", &[])
        .expect("call after reload");
    assert_eq!(v.as_f64(), Some(2.0));

    std::fs::remove_dir_all(&dir).ok();
}

/// A syntax error in the entry at startup must not kill scripting: the
/// plugin still installs its runtime state and file watcher, and fixing
/// the file on disk retries the initial load — the script's hooks become
/// callable without restarting the app.
#[test]
fn startup_load_failure_recovers_via_hot_reload() {
    let dir = unique_temp_dir("startup_recovery");
    let entry = dir.join("main.ts");
    std::fs::write(&entry, "export function broken( { not valid").unwrap();

    let reports = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = std::sync::Arc::clone(&reports);
    let mut api = test_api();
    api.register_closure("report", move |args| {
        sink.lock()
            .unwrap()
            .push(args[0].as_str().unwrap_or("").to_string());
        Ok(HostValue::Null)
    });

    let mut app = moonfield_app::App::new();
    app.add_plugin(moonfield_script::ScriptPlugin::new(api).with_entry(&entry));

    // Startup: the entry fails to load, but the plugin must stay alive.
    app.update();
    assert!(reports.lock().unwrap().is_empty());

    // Fix the file; the resulting file event drives a retry of the initial
    // load. Watcher delivery is asynchronous, so poll frames until it lands.
    std::fs::write(&entry, "export function on_update(dt) { report('alive'); }").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while reports.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.update();
    }

    assert_eq!(
        reports.lock().unwrap().last().map(String::as_str),
        Some("alive"),
        "fixed entry should load via hot reload and run on_update"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// The ScriptPlugin drives `on_fixed_update` with the configured fixed
/// delta (Godot `_physics_process` style) before `on_update` each frame.
#[test]
fn script_plugin_drives_fixed_and_variable_hooks() {
    let dir = unique_temp_dir("fixed_hook");
    let entry = dir.join("main.js");
    std::fs::write(
        &entry,
        "export function on_fixed_update(dt) { report('fixed', dt); }\n\
         export function on_update(dt) { report('update', dt); }",
    )
    .unwrap();

    // Count hook invocations through a host function (the plugin's runtime
    // is not directly reachable from the test).
    let counts = std::sync::Arc::new(std::sync::Mutex::new((0u32, 0u32, 0.0f64)));
    let shared = std::sync::Arc::clone(&counts);
    let mut api = test_api();
    api.register_closure("report", move |args| {
        let mut guard = shared.lock().unwrap();
        match args.first().and_then(|v| v.as_str()) {
            Some("fixed") => {
                guard.0 += 1;
                guard.2 = args.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
            }
            Some("update") => guard.1 += 1,
            _ => {}
        }
        Ok(HostValue::Null)
    });

    let mut app = moonfield_app::App::new();
    app.add_plugin(
        moonfield_script::ScriptPlugin::new(api)
            .with_entry(&entry)
            .with_fixed_timestep(std::time::Duration::from_millis(10)),
    );

    app.update(); // startup + first frame (tiny dt → likely 0 fixed steps)
    std::thread::sleep(std::time::Duration::from_millis(35));
    app.update(); // ~35ms at 10ms/step → 3..=5 fixed steps, 1 update

    {
        let guard = counts.lock().unwrap();
        assert!(
            (3..=5).contains(&guard.0),
            "expected 3..=5 fixed steps after a 35ms frame, got {}",
            guard.0
        );
        assert!(
            (guard.2 - 0.010).abs() < 1e-9,
            "fixed dt must be the configured timestep, got {}",
            guard.2
        );
    }

    app.update();
    let guard = counts.lock().unwrap();
    assert_eq!(guard.1, 3, "on_update must run once per frame");

    std::fs::remove_dir_all(&dir).ok();
}

/// End-to-end input flow: winit-side `InputState` events feed the
/// `on_input` hook and the `input_*` polling API, with `just_pressed`
/// delivered to exactly one fixed step (the Bevy-#6183 pitfall, solved
/// structurally by the step-latched edge view).
#[test]
fn input_hooks_and_polling_end_to_end() {
    let dir = unique_temp_dir("input_e2e");
    let entry = dir.join("main.js");
    std::fs::write(
        &entry,
        "let fixedJP = 0, updateJP = 0, actionJP = 0, held = 0, events = [];\n\
         export function main() { input_bind_action('jump', ['Space']); }\n\
         export function on_input(e) { events.push(e.type + ':' + (e.code || '')); }\n\
         export function on_fixed_update(dt) {\n\
             if (input_is_key_just_pressed('Space')) fixedJP++;\n\
             if (input_is_action_just_pressed('jump')) actionJP++;\n\
         }\n\
         export function on_update(dt) {\n\
             if (input_is_key_just_pressed('Space')) updateJP++;\n\
             if (input_is_key_pressed('Space')) held++;\n\
             report([fixedJP, updateJP, actionJP, held, events.join('|')].join(','));\n\
         }",
    )
    .unwrap();

    let reports = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = std::sync::Arc::clone(&reports);
    let input = moonfield_script::new_shared_input();
    let mut api = test_api();
    api.register_closure("report", move |args| {
        sink.lock()
            .unwrap()
            .push(args[0].as_str().unwrap_or("").to_string());
        Ok(HostValue::Null)
    });
    moonfield_script::register_input_api(&mut api, &input);

    let mut app = moonfield_app::App::new();
    app.insert_resource(moonfield_window::InputState::default());
    app.add_plugin(
        moonfield_script::ScriptPlugin::new(api)
            .with_entry(&entry)
            .with_input_state(input)
            .with_fixed_timestep(std::time::Duration::from_millis(10)),
    );

    // Frame 1: startup + a plain frame (no input).
    app.update();

    // Press Space and give the next frame real time so several fixed
    // steps run within it.
    {
        let mut res = app
            .world_mut()
            .get_resource_mut::<moonfield_window::InputState>()
            .unwrap();
        res.apply_event(moonfield_window::InputEvent::KeyPressed {
            code: "Space".into(),
        });
    }
    std::thread::sleep(std::time::Duration::from_millis(35));
    app.update(); // frame 2: on_input replay, N fixed steps, one update
                  // Mirror the backend's frame boundary (winit's about_to_wait).
    app.world_mut()
        .get_resource_mut::<moonfield_window::InputState>()
        .unwrap()
        .end_frame();

    // Frame 3: no new events, more fixed steps.
    std::thread::sleep(std::time::Duration::from_millis(35));
    app.update();

    let guard = reports.lock().unwrap();
    let last = guard.last().expect("a report from on_update");
    // fixedJP=1 (exactly one fixed step saw the edge, not N),
    // updateJP=1 (frame-scoped edge, cleared after the frame),
    // actionJP=1, held=2 (held across both frames),
    // one replayed event.
    assert_eq!(last, "1,1,1,2,key_pressed:Space", "reports: {:?}", *guard);

    std::fs::remove_dir_all(&dir).ok();
}

/// Window lifecycle events travel on their own channel to the
/// `on_window_event` hook, and the `app_*` control functions drive the
/// shared exit policy (Godot's `auto_accept_quit` model).
#[test]
fn window_events_and_exit_control_end_to_end() {
    let dir = unique_temp_dir("window_events");
    let entry = dir.join("main.js");
    std::fs::write(
        &entry,
        "let seen = [];\n\
         export function main() { app_set_auto_exit_on_close(false); }\n\
         export function on_window_event(e) {\n\
             seen.push(e.type + (e.width ? ':' + e.width + 'x' + e.height : ''));\n\
             if (e.type === 'close_requested') app_exit();\n\
         }\n\
         export function on_update(dt) { report(seen.join('|')); }",
    )
    .unwrap();

    let reports = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = std::sync::Arc::clone(&reports);
    let input = moonfield_script::new_shared_input();
    let control = moonfield_window::WindowControl::default();
    let window = moonfield_window::new_shared_window();
    let window_requests = moonfield_window::WindowRequests::default();
    let mut api = test_api();
    api.register_closure("report", move |args| {
        sink.lock()
            .unwrap()
            .push(args[0].as_str().unwrap_or("").to_string());
        Ok(HostValue::Null)
    });
    moonfield_script::register_input_api(&mut api, &input);
    moonfield_script::register_window_api(&mut api, &control, &window, &window_requests);

    let mut app = moonfield_app::App::new();
    app.insert_resource(moonfield_window::WindowEvents::default());
    app.add_plugin(
        moonfield_script::ScriptPlugin::new(api)
            .with_entry(&entry)
            .with_input_state(input),
    );

    // Startup runs main(): the script takes over close handling.
    app.update();
    assert!(!control.auto_exit_on_close());
    assert!(!control.exit_requested());

    // Queue lifecycle events and run a frame.
    {
        let mut res = app
            .world_mut()
            .get_resource_mut::<moonfield_window::WindowEvents>()
            .unwrap();
        res.push(moonfield_window::WindowEventKind::Resized {
            width: 1024,
            height: 768,
        });
        res.push(moonfield_window::WindowEventKind::FocusGained);
        res.push(moonfield_window::WindowEventKind::CloseRequested);
    }
    app.update();

    assert_eq!(
        reports.lock().unwrap().last().unwrap(),
        "resized:1024x768|focus_gained|close_requested"
    );
    // The close hook decided to quit.
    assert!(control.exit_requested());

    std::fs::remove_dir_all(&dir).ok();
}

/// Promises settled during a script call must have their `.then` callbacks
/// run before the call returns (microtask checkpoint / job queue drain).
#[test]
fn microtasks_run_after_call() {
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

/// Cyclic or pathologically deep values passed to a host function must not
/// crash the process: argument marshaling caps the recursion depth and
/// degrades over-deep containers to a placeholder instead of overflowing
/// the Rust stack (which no `catch_unwind` can stop).
#[test]
fn cyclic_values_do_not_crash_host_calls() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let sink = std::sync::Arc::clone(&seen);
    let mut api = test_api();
    api.register_closure("take_any", move |args| {
        *sink.lock().unwrap() = format!("{:?}", args[0]);
        Ok(HostValue::Null)
    });

    let mut runtime = Runtime::new(api).expect("runtime");
    runtime
        .load(
            "cyclic.js",
            "function cyclicObject() { const o = { a: 1 }; o.self = o; take_any(o); }\n\
             function cyclicArray() { const a = [1]; a.push(a); take_any(a); }\n\
             function deep() {\n\
                 let v = 0;\n\
                 for (let i = 0; i < 100; i++) v = [v];\n\
                 take_any(v);\n\
             }\n\
             function ok() { return 7; }",
        )
        .expect("load");

    for case in ["cyclicObject", "cyclicArray", "deep"] {
        seen.lock().unwrap().clear();
        runtime.call(case).expect(case);
        assert!(
            seen.lock().unwrap().contains("[max depth exceeded]"),
            "{case}: over-deep value must degrade to a placeholder"
        );
    }

    // The runtime stays usable afterwards.
    let v = runtime.call_with_args("ok", &[]).expect("call after");
    assert_eq!(v.as_f64(), Some(7.0));
}

/// Typed arrays crossing the JS→host boundary must arrive as typed arrays —
/// the QuickJS backend previously degraded a `Float32Array` argument to a
/// generic object with "0".."n" index keys (the V8 backend always had its
/// zero-copy branch). Mirrors `call_with_args_preserves_special_values`
/// in the opposite direction.
#[test]
fn typed_arrays_roundtrip_js_to_host() {
    let mut api = test_api();
    api.register_closure("sum_f32", |args| {
        let slice = args
            .first()
            .and_then(|v| v.as_f32_slice())
            .ok_or_else(|| "expected a Float32Array".to_string())?;
        Ok(HostValue::Number(slice.iter().map(|v| *v as f64).sum()))
    });
    api.register_closure("sum_bytes", |args| {
        let bytes = args
            .first()
            .and_then(|v| v.as_bytes())
            .ok_or_else(|| "expected a Uint8Array".to_string())?;
        Ok(HostValue::Number(bytes.iter().map(|b| *b as f64).sum()))
    });
    api.register_closure("echo_f32", |args| {
        let slice = args
            .first()
            .and_then(|v| v.as_f32_slice())
            .ok_or_else(|| "expected a Float32Array".to_string())?;
        Ok(HostValue::from(slice.to_vec()))
    });

    let mut runtime = Runtime::new(api).expect("runtime");
    runtime
        .load(
            "typed_arrays.js",
            "function sumF32() { return sum_f32(new Float32Array([1.5, 2.5, 3])); }\n\
             function sumU8() { return sum_bytes(new Uint8Array([1, 2, 3, 4])); }\n\
             function roundtripF32() {\n\
                 const echoed = echo_f32(new Float32Array([1.5, 2.5, 3]));\n\
                 let s = 0;\n\
                 for (const x of echoed) s += x;\n\
                 return s;\n\
             }",
        )
        .expect("load");

    for (case, want) in [("sumF32", 7.0), ("sumU8", 10.0), ("roundtripF32", 7.0)] {
        let v = runtime.call_with_args(case, &[]).expect(case);
        assert_eq!(v.as_f64(), Some(want), "{case}");
    }
}
