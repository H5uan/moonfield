//! QuickJS backend for the scripting runtime.

use super::{HostValue, Result, ScriptApi, ScriptError, ScriptRuntime};
use moonfield_base::{error, info, warn};
use rquickjs::function::{Func, Rest};
use rquickjs::{CaughtError, Context, Runtime, Value};

/// JavaScript shim that wires a `console` object to the `__mf_log` host
/// function registered below. Stringifying via `String(...)` matches browser
/// `console.log` semantics.
const CONSOLE_SHIM: &str = r#"
globalThis.console = {
  log:   function() { __mf_log(0, Array.prototype.map.call(arguments, String).join(" ")); },
  info:  function() { __mf_log(0, Array.prototype.map.call(arguments, String).join(" ")); },
  warn:  function() { __mf_log(1, Array.prototype.map.call(arguments, String).join(" ")); },
  error: function() { __mf_log(2, Array.prototype.map.call(arguments, String).join(" ")); }
};
"#;

/// A script runtime backed by QuickJS.
pub struct QuickJsRuntime {
    runtime: Runtime,
    context: Context,
    api: ScriptApi,
}

impl ScriptRuntime for QuickJsRuntime {
    fn new(api: ScriptApi) -> Result<Self> {
        let runtime = Runtime::new()
            .map_err(|e| ScriptError::BackendNotAvailable(format!("quickjs: {:?}", e)))?;
        // Give QuickJS a generous stack limit so that host functions (which may
        // call into Vulkan drivers) do not overflow the JS engine's C stack.
        runtime.set_max_stack_size(8 * 1024 * 1024);
        let context = Context::full(&runtime)
            .map_err(|e| ScriptError::BackendNotAvailable(format!("quickjs context: {:?}", e)))?;

        let mut rt = Self {
            runtime,
            context,
            api,
        };
        rt.register_api()?;
        Ok(rt)
    }

    fn load(&mut self, _name: &str, source: &str) -> Result<()> {
        self.context.with(
            |ctx| match CaughtError::catch(&ctx, ctx.eval::<(), _>(source)) {
                Ok(()) => Ok(()),
                Err(ce) => Err(ScriptError::Execution(format_caught_error(ce))),
            },
        )?;
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.context = Context::full(&self.runtime)
            .map_err(|e| ScriptError::Runtime(format!("failed to recreate context: {:?}", e)))?;
        self.register_api()
    }

    fn call(&mut self, function: &str) -> Result<()> {
        let expr = format!("{}()", function);
        self.context.with(
            |ctx| match CaughtError::catch(&ctx, ctx.eval::<(), _>(expr)) {
                Ok(()) => Ok(()),
                Err(ce) => Err(ScriptError::Runtime(format!(
                    "call '{}': {}",
                    function,
                    format_caught_error(ce)
                ))),
            },
        )?;
        Ok(())
    }
}

impl QuickJsRuntime {
    fn register_api(&mut self) -> Result<()> {
        self.context
            .with(|ctx| {
                let global = ctx.globals();
                for &(name, func) in self.api.iter() {
                    // Use a non-capturing wrapper to avoid borrow issues with `name`.
                    let wrapper = ApiFuncWrapper { name, func };
                    global.set(
                        name,
                        Func::from(move |args: Rest<Value>| -> rquickjs::Result<()> {
                            let mut host_args: Vec<HostValue> = Vec::with_capacity(args.0.len());
                            for arg in args.0.iter() {
                                host_args.push(quickjs_value_to_host(arg));
                            }
                            if let Err(e) = (wrapper.func)(&host_args) {
                                eprintln!("{} error: {}", wrapper.name, e);
                            }
                            Ok(())
                        }),
                    )?;
                }
                // Host sink for console output: (level, message).
                global.set(
                    "__mf_log",
                    Func::from(|level: i32, msg: String| match level {
                        0 => info!("{}", msg),
                        1 => warn!("{}", msg),
                        _ => error!("{}", msg),
                    }),
                )?;
                ctx.eval::<(), _>(CONSOLE_SHIM)?;
                Ok(())
            })
            .map_err(|e: rquickjs::Error| ScriptError::Runtime(format!("{:?}", e)))?;
        Ok(())
    }
}

/// Helper to hold a (name, func) pair without lifetime issues.
struct ApiFuncWrapper {
    name: &'static str,
    func: super::HostFn,
}

/// Convert a QuickJS `Value` to a `HostValue`.
fn quickjs_value_to_host(value: &Value) -> HostValue {
    if value.is_undefined() || value.is_null() {
        return HostValue::Null;
    }
    if let Some(b) = value.as_bool() {
        return HostValue::Bool(b);
    }
    if let Some(n) = value.as_float() {
        return HostValue::Number(n);
    }
    if let Some(n) = value.as_int() {
        return HostValue::Number(n as f64);
    }
    if let Some(s) = value.as_string() {
        if let Ok(s) = s.to_string() {
            return HostValue::String(s);
        }
    }
    if let Some(arr) = value.as_array() {
        let mut items = Vec::new();
        for item in arr.iter() {
            if let Ok(item) = item {
                items.push(quickjs_value_to_host(&item));
            }
        }
        return HostValue::Array(items);
    }
    if let Some(obj) = value.as_object() {
        let mut map = std::collections::HashMap::new();
        for key in obj.keys::<String>() {
            if let Ok(key) = key {
                if let Ok(val) = obj.get::<_, Value>(&key) {
                    map.insert(key, quickjs_value_to_host(&val));
                }
            }
        }
        return HostValue::Object(map);
    }
    // Fallback: stringify
    if let Some(s) = value.as_string() {
        if let Ok(s) = s.to_string() {
            return HostValue::String(s);
        }
    }
    HostValue::String(format!("{:?}", value))
}

/// Turn a caught QuickJS error into a human-readable string with message,
/// location, and (if present) stack trace.
fn format_caught_error<'js>(ce: CaughtError<'js>) -> String {
    match ce {
        CaughtError::Exception(e) => {
            let mut s = e.message().unwrap_or_else(|| "exception".to_string());
            if let (Some(file), Some(line)) = (e.file(), e.line()) {
                s.push_str(&format!("\n  at {}:{}", file, line));
            }
            if let Some(stack) = e.stack() {
                let stack = stack.trim();
                if !stack.is_empty() {
                    s.push_str(&format!("\n{}", stack));
                }
            }
            s
        }
        CaughtError::Value(v) => format!("thrown value: {:?}", v),
        CaughtError::Error(e) => format!("{:?}", e),
    }
}
