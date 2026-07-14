//! V8 backend for the scripting runtime.

use super::{Result, ScriptApi, ScriptError, ScriptRuntime};
use std::sync::Once;

static V8_INIT: Once = Once::new();

/// A script runtime backed by V8 (rusty_v8).
pub struct V8Runtime {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    // Box the API so its address remains stable when the runtime is moved
    // after `register_api` has stored a raw pointer to it in V8 externals.
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

    fn load(&mut self, _name: &str, source: &str) -> Result<()> {
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);

        let code = v8::String::new(scope, source)
            .ok_or_else(|| ScriptError::Execution("failed to create source string".to_string()))?;
        let script = v8::Script::compile(scope, code, None)
            .ok_or_else(|| ScriptError::Execution("failed to compile script".to_string()))?;
        script
            .run(scope)
            .map(|_| ())
            .ok_or_else(|| ScriptError::Execution("failed to run script".to_string()))
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

        let global = local_context.global(scope);
        let name = v8::String::new(scope, function).unwrap();
        let value = global
            .get(scope, name.into())
            .ok_or_else(|| ScriptError::Runtime(format!("function '{}' not found", function)))?;
        let func = v8::Local::<v8::Function>::try_from(value)
            .map_err(|_| ScriptError::Runtime(format!("'{}' is not a function", function)))?;
        let recv = v8::undefined(scope);
        func.call(scope, recv.into(), &[])
            .map(|_| ())
            .ok_or_else(|| ScriptError::Runtime(format!("failed to call '{}'", function)))
    }
}

impl V8Runtime {
    fn register_api(&mut self) -> Result<()> {
        v8::scope!(let handle_scope, &mut self.isolate);
        let local_context = v8::Local::new(handle_scope, &self.context);
        let scope = &mut v8::ContextScope::new(handle_scope, local_context);

        let global = local_context.global(scope);
        let name = v8::String::new(scope, "record_frame").unwrap();

        // Pass a pointer to the ScriptApi through V8's associated data so the
        // callback closure can stay stateless (zero-sized), which the v8 crate
        // requires for MapFnTo conversions.
        let api_ptr = self.api.as_ref() as *const ScriptApi as *mut std::ffi::c_void;
        let data = v8::External::new(scope, api_ptr);

        let func = v8::Function::builder(
            |_: &mut v8::PinScope,
             args: v8::FunctionCallbackArguments,
             mut retval: v8::ReturnValue| {
                let external =
                    v8::Local::<v8::External>::try_from(args.data()).unwrap();
                let api = unsafe { &*(external.value() as *const ScriptApi) };
                let ok = (api.record_frame)().is_ok();
                retval.set_bool(ok);
            },
        )
        .data(data.into())
        .build(scope);

        let func = func.ok_or_else(|| ScriptError::Runtime("failed to build record_frame".to_string()))?;
        global.set(scope, name.into(), func.into()).unwrap();

        Ok(())
    }
}
