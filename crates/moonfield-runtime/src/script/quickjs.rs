//! QuickJS backend for the scripting runtime.

use super::{Result, ScriptApi, ScriptError, ScriptRuntime};
use rquickjs::{Context, Runtime};
use rquickjs::function::Func;

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
        self.context
            .with(|ctx| ctx.eval::<(), _>(source))
            .map_err(|e| ScriptError::Execution(format!("{:?}", e)))?;
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.context = Context::full(&self.runtime)
            .map_err(|e| ScriptError::Runtime(format!("failed to recreate context: {:?}", e)))?;
        self.register_api()
    }

    fn call(&mut self, function: &str) -> Result<()> {
        let expr = format!("{}()", function);
        self.context
            .with(|ctx| ctx.eval::<(), _>(expr))
            .map_err(|e| ScriptError::Runtime(format!("call '{}': {:?}", function, e)))
    }
}

impl QuickJsRuntime {
    fn register_api(&mut self) -> Result<()> {
        let record_frame = self.api.record_frame;
        self.context
            .with(|ctx| {
                let global = ctx.globals();
                global.set(
                    "record_frame",
                    Func::from(move || {
                        if let Err(e) = record_frame() {
                            eprintln!("record_frame error: {}", e);
                        }
                    }),
                )?;
                Ok(())
            })
            .map_err(|e: rquickjs::Error| ScriptError::Runtime(format!("{:?}", e)))
    }
}
