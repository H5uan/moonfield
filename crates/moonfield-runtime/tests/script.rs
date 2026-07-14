//! Integration tests for the scripting runtime.

use moonfield_runtime::script::{load_script, transpile_typescript, ScriptApi, ScriptRuntime};

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

#[test]
fn transpile_strips_typescript_annotations() {
    let ts = "function main(): void {\n    const x: number = 42;\n    const y = x as number;\n    record_frame();\n}";
    let js = transpile_typescript(ts).expect("transpile");

    // Type annotations must be gone (this is what the old line-stripper
    // frequently failed at).
    assert!(!js.contains(": void"));
    assert!(!js.contains(": number"));
    assert!(!js.contains("as number"));

    let mut runtime = Runtime::new(ScriptApi::default()).expect("runtime");
    runtime
        .load("main.ts", &js)
        .expect("load transpiled script");
    runtime.call("main").expect("call transpiled main");
}

#[test]
fn console_binding_is_callable() {
    let mut runtime = Runtime::new(ScriptApi::default()).expect("runtime");
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
    let mut runtime = Runtime::new(ScriptApi::default()).expect("runtime");
    runtime
        .load("err.js", "function main() { callMissingThing(); }")
        .expect("load error script");

    let err = runtime.call("main");
    assert!(err.is_err(), "calling an undefined function must error");
    let msg = err.unwrap_err().to_string();
    // The message must carry the actual V8 exception text, not a generic
    // "failed to run script".
    assert!(
        msg.contains("not defined") || msg.contains("callMissingThing"),
        "error message should mention the missing symbol, got: {msg}"
    );
}
