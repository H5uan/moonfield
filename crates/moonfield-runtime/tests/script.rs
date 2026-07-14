//! Integration tests for the scripting runtime.

use moonfield_runtime::script::{load_script, ScriptApi, ScriptRuntime};

#[cfg(feature = "v8-backend")]
use moonfield_runtime::script::V8Runtime as Runtime;

#[cfg(feature = "quickjs-backend")]
use moonfield_runtime::script::QuickJsRuntime as Runtime;

use std::path::PathBuf;

fn script_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("scripts")
}

#[test]
fn load_and_call_record_frame() {
    let path = script_dir().join("record_frame.js");
    let source = load_script(&path).expect("load script");

    let mut runtime = Runtime::new(ScriptApi::default()).expect("runtime");
    runtime
        .load(path.to_string_lossy().as_ref(), &source)
        .expect("load into runtime");
    runtime.call("main").expect("call main");
}

#[test]
fn reload_changes_behavior() {
    let path = script_dir().join("record_frame.js");
    let source = load_script(&path).expect("load script");

    let mut runtime = Runtime::new(ScriptApi::default()).expect("runtime");
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
