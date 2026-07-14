//! V8 backend for the scripting runtime.

use super::{HostFn, Result, ScriptApi, ScriptError, ScriptRuntime};
use moonfield_base::{error, info, warn};
use std::sync::Once;

static V8_INIT: Once = Once::new();

/// A script runtime backed by V8 (rusty_v8).
pub struct V8Runtime {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    // Boxed so the registry vector's storage is never moved while V8 externals
    // hold raw pointers into it.
    api: Box<ScriptApi>,
}

impl ScriptRuntime for V8Runtime {
    fn new(api: ScriptApi) -> Result<Self> {
        V8_INIT.call_once(|| {
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });

        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        let context = {
            v8::scope!(let handle_scope, &mut isolate);
            let context = v8::Context::new(handle_scope, Default::default());
            v8::Global::new(handle_scope, context)
        };

        let mut rt = Self {
            isolate,
            context,
            api: Box::new(api),
        };
        rt.register_api()?;
        Ok(rt)
    }

    fn load(&mut self, name: &str, source: &str) -> Result<()> {
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        v8::tc_scope!(let tc, scope);

        let name_str = v8::String::new(tc, name).unwrap();
        let origin = v8::ScriptOrigin::new(
            tc,
            name_str.into(),
            0,
            0,
            false,
            0,
            None,
            false,
            false,
            false,
            None,
        );
        let code = v8::String::new(tc, source)
            .ok_or_else(|| ScriptError::Execution("failed to create source string".into()))?;
        let script = match v8::Script::compile(tc, code, Some(&origin)) {
            Some(s) => s,
            None => return Err(ScriptError::Execution(v8_exception!(tc))),
        };
        match script.run(tc) {
            Some(_) => Ok(()),
            None => Err(ScriptError::Execution(v8_exception!(tc))),
        }
    }

    fn reload(&mut self) -> Result<()> {
        let context = {
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Context::new(handle_scope, Default::default());
            v8::Global::new(handle_scope, context)
        };
        self.context = context;
        self.register_api()
    }

    fn call(&mut self, function: &str) -> Result<()> {
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        v8::tc_scope!(let tc, scope);

        let global = local_context.global(tc);
        let name = v8::String::new(tc, function).unwrap();
        let value = global
            .get(tc, name.into())
            .ok_or_else(|| ScriptError::Runtime(format!("function '{}' not found", function)))?;
        let func = v8::Local::<v8::Function>::try_from(value)
            .map_err(|_| ScriptError::Runtime(format!("'{}' is not a function", function)))?;
        let recv = v8::undefined(tc);
        if func.call(tc, recv.into(), &[]).is_none() {
            return Err(ScriptError::Runtime(format!(
                "{}: {}",
                function,
                v8_exception!(tc)
            )));
        }
        Ok(())
    }
}

impl V8Runtime {
    fn register_api(&mut self) -> Result<()> {
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);
        let global = local_context.global(scope);

        for entry in self.api.iter() {
            // Stable pointer into the boxed registry vector.
            let ptr = entry as *const (&'static str, HostFn) as *mut std::ffi::c_void;
            let data = v8::External::new(scope, ptr);
            let js_name = v8::String::new(scope, entry.0).unwrap();
            let func = v8::Function::builder(
                |_scope: &mut v8::PinScope,
                 args: v8::FunctionCallbackArguments,
                 mut retval: v8::ReturnValue| {
                    let external = v8::Local::<v8::External>::try_from(args.data()).unwrap();
                    let entry = unsafe { &*(external.value() as *const (&'static str, HostFn)) };
                    let ok = (entry.1)().is_ok();
                    retval.set_bool(ok);
                },
            )
            .data(data.into())
            .build(scope)
            .ok_or_else(|| ScriptError::Runtime(format!("failed to build {}", entry.0)))?;
            global.set(scope, js_name.into(), func.into()).unwrap();
        }

        Self::register_console(scope, local_context)
    }

    /// Register a `console` object with `log`/`info`/`warn`/`error` that forward
    /// to the host logger, stringifying every argument the way browsers do.
    fn register_console(scope: &mut v8::PinScope, context: v8::Local<v8::Context>) -> Result<()> {
        let console = v8::Object::new(scope);
        let global = context.global(scope);

        let log = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                info!("{}", collect_console_args(s, &a));
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build console.log".into()))?;
        let info_fn = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                info!("{}", collect_console_args(s, &a));
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build console.info".into()))?;
        let warn_fn = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                warn!("{}", collect_console_args(s, &a));
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build console.warn".into()))?;
        let err_fn = v8::Function::new(
            scope,
            |s: &mut v8::PinScope, a: v8::FunctionCallbackArguments, mut r: v8::ReturnValue| {
                error!("{}", collect_console_args(s, &a));
                r.set_undefined();
            },
        )
        .ok_or_else(|| ScriptError::Runtime("failed to build console.error".into()))?;

        for (name, func) in [
            ("log", log),
            ("info", info_fn),
            ("warn", warn_fn),
            ("error", err_fn),
        ] {
            let n = v8::String::new(scope, name).unwrap();
            console.set(scope, n.into(), func.into());
        }
        let cname = v8::String::new(scope, "console").unwrap();
        global.set(scope, cname.into(), console.into());

        Ok(())
    }
}

/// Stringify all arguments passed to a `console.*` call, joined by spaces.
fn collect_console_args(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> String {
    let n = args.length();
    let mut parts: Vec<String> = Vec::with_capacity(n.max(0) as usize);
    for i in 0..n {
        parts.push(args.get(i).to_rust_string_lossy(scope));
    }
    parts.join(" ")
}

/// Extract a human-readable exception (message + location + stack frames) from
/// a V8 `TryCatch` scope. Used as a macro because the `tc` scope type is an
/// opaque pinned projection that is awkward to name generically.
macro_rules! v8_exception {
    ($tc:expr) => {{
        let tc = $tc;
        if !tc.has_caught() {
            String::from("unknown error")
        } else {
            let mut out = String::new();
            if let Some(exc) = tc.exception() {
                out.push_str(&exc.to_rust_string_lossy(tc));
            }
            if let Some(msg) = tc.message() {
                let mut loc = String::new();
                if let Some(res) = msg.get_script_resource_name(tc) {
                    let r = res.to_rust_string_lossy(tc);
                    if !r.is_empty() && r != "undefined" {
                        loc.push_str(&r);
                    }
                }
                if let Some(line) = msg.get_line_number(tc) {
                    if !loc.is_empty() {
                        loc.push(':');
                    }
                    loc.push_str(&line.to_string());
                }
                if !loc.is_empty() {
                    out.push_str(&format!("\n  at {}", loc));
                }
                if let Some(st) = msg.get_stack_trace(tc) {
                    let count = st.get_frame_count().min(10);
                    for i in 0..count {
                        if let Some(frame) = st.get_frame(tc, i) {
                            let mut f = String::new();
                            let has_name = if let Some(name) = frame.get_function_name(tc) {
                                let n = name.to_rust_string_lossy(tc);
                                if !n.is_empty() {
                                    f.push_str(&n);
                                    f.push_str(" (");
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            if let Some(sname) = frame.get_script_name_or_source_url(tc) {
                                f.push_str(&sname.to_rust_string_lossy(tc));
                            }
                            f.push_str(&format!(
                                ":{}:{}",
                                frame.get_line_number(),
                                frame.get_column()
                            ));
                            if has_name {
                                f.push(')');
                            }
                            if !f.is_empty() {
                                out.push_str(&format!("\n    at {}", f));
                            }
                        }
                    }
                }
            }
            out
        }
    }};
}
use v8_exception;
