//! QuickJS backend for the scripting runtime.

use super::{HostFn, Result, ScriptApi, ScriptError, ScriptRuntime};
use moonfield_base::{error, info, warn};
use rquickjs::function::Func;
use rquickjs::{CaughtError, Context, Runtime};

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
                    global.set(
                        name,
                        Func::from(move || {
                            if let Err(e) = func(&[]) {
                                eprintln!("{} error: {}", name, e);
                            }
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
