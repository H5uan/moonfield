//! Module system for the scripting runtime.
//!
//! Provides a backend-agnostic [`ModuleRegistry`] that maps module specifiers
//! to source code, and transforms ESModule syntax (`import`/`export`) into
//! CommonJS (`require`/`exports`) for evaluation by any JS engine.
//!
//! # Module loading strategy
//!
//! Instead of using V8's native ESModule API (which requires a complex
//! `ResolveModuleCallback` that cannot capture state), we transform each
//! module into a CommonJS-style IIFE:
//!
//! ```js
//! function(require, exports) {
//!   const { foo } = require("./bar");
//!   exports.main = function() { foo(); };
//! }
//! ```
//!
//! Modules are evaluated in topological order. The `require` function
//! resolves specifiers and returns cached exports. This approach works
//! across both V8 and QuickJS backends.
//!
//! # Resolution strategy
//!
//! Module specifiers are resolved in this order:
//! 1. Relative paths (`./foo`, `../bar/baz`) — resolved against the importer's
//!    directory.
//! 2. Absolute paths — used as-is.
//! 3. Bare specifiers (`"lodash"`) — not yet supported; returns an error.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use swc_common::sync::Lrc;
use swc_common::{FileName, Globals, Mark, SourceMap, GLOBALS};
use swc_ecma_ast::*;
use swc_ecma_codegen::{text_writer::JsWriter, Config as CodegenConfig, Emitter};
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax, TsSyntax};
use swc_ecma_transforms_module::common_js::{common_js, Config as CjsConfig, FeatureFlag};
use swc_ecma_transforms_module::path::Resolver;


/// A compiled module, holding its source and resolved dependency list.
#[derive(Clone)]
pub struct ModuleInfo {
    /// The module's canonical name (e.g. `"record_frame"` or `"./utils"`).
    pub name: String,
    /// The JavaScript source code (already transpiled if needed).
    pub source: String,
    /// Transformed CommonJS source (with `import`/`export` replaced).
    pub cjs_source: String,
    /// List of module specifiers this module imports.
    pub imports: Vec<String>,
}

/// A registry of modules keyed by canonical name.
///
/// The registry is populated by the runtime before instantiation. It does not
/// depend on any specific JS engine, so it lives in the backend-agnostic layer.
#[derive(Default)]
pub struct ModuleRegistry {
    modules: HashMap<String, ModuleInfo>,
    base_path: PathBuf,
}

impl ModuleRegistry {
    /// Create a new, empty registry.
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            base_path: PathBuf::from("."),
        }
    }

    /// Set the base path for relative module resolution.
    pub fn with_base_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.base_path = path.into();
        self
    }

    /// Register a module by name with its source code.
    ///
    /// Transforms ESModule syntax to CommonJS. Returns the canonical name.
    pub fn register(&mut self, name: &str, source: String) -> String {
        let canonical = self.canonicalize(name);
        let imports = Self::extract_imports_ast(&source);
        let cjs_source = Self::transform_to_cjs_ast(&source);
        self.modules.insert(
            canonical.clone(),
            ModuleInfo {
                name: canonical.clone(),
                source,
                cjs_source,
                imports,
            },
        );
        canonical
    }

    /// Get a module's info by canonical name.
    pub fn get(&self, name: &str) -> Option<&ModuleInfo> {
        self.modules.get(name)
    }

    /// Check if a module is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.modules.contains_key(name)
    }

    /// Iterate over all registered modules.
    pub fn iter(&self) -> impl Iterator<Item = &ModuleInfo> {
        self.modules.values()
    }

    /// Number of registered modules.
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Resolve a module specifier against a base name.
    ///
    /// Returns the canonical name of the resolved module.
    pub fn resolve(&self, specifier: &str, base: &str) -> Option<String> {
        if specifier.starts_with('.') {
            let base_dir = Path::new(base).parent().unwrap_or(Path::new("."));
            let resolved = base_dir.join(specifier);
            let normalized = Self::normalize_path(&resolved);
            let canonical = normalized.to_string_lossy().replace('\\', "/");

            if self.modules.contains_key(&canonical) {
                return Some(canonical);
            }
            let with_js = format!("{}.js", canonical);
            if self.modules.contains_key(&with_js) {
                return Some(with_js);
            }
            let with_ts = format!("{}.ts", canonical);
            if self.modules.contains_key(&with_ts) {
                return Some(with_ts);
            }
        }

        if self.modules.contains_key(specifier) {
            return Some(specifier.to_string());
        }

        None
    }

    /// Topologically sort modules by dependency order.
    ///
    /// Returns module names in evaluation order (dependencies first).
    pub fn order_dependencies(&self, entry: &str) -> Result<Vec<String>, String> {
        let mut visited = HashMap::new(); // false = visiting, true = done
        let mut order = Vec::new();

        fn visit<'a>(
            name: &str,
            modules: &'a HashMap<String, ModuleInfo>,
            visited: &mut HashMap<String, bool>,
            order: &mut Vec<String>,
        ) -> Result<(), String> {
            match visited.get(name) {
                Some(&true) => return Ok(()),
                Some(&false) => {
                    return Err(format!("circular dependency: {}", name));
                }
                None => {}
            }

            visited.insert(name.to_string(), false);

            let info = modules
                .get(name)
                .ok_or_else(|| format!("module '{}' not found", name))?;

            // Resolve and visit dependencies.
            for dep_spec in &info.imports {
                // Resolve the specifier against the current module's name.
                let resolved = if dep_spec.starts_with('.') {
                    let base_dir = Path::new(name)
                        .parent()
                        .unwrap_or(Path::new("."));
                    let resolved = ModuleRegistry::normalize_path(&base_dir.join(dep_spec));
                    let c = resolved.to_string_lossy().replace('\\', "/");
                    // Try to find it in the module map.
                    if modules.contains_key(&c) {
                        c
                    } else {
                        let with_js = format!("{}.js", c);
                        if modules.contains_key(&with_js) {
                            with_js
                        } else {
                            dep_spec.clone()
                        }
                    }
                } else {
                    dep_spec.clone()
                };

                visit(&resolved, modules, visited, order)?;
            }

            visited.insert(name.to_string(), true);
            order.push(name.to_string());
            Ok(())
        }

        visit(entry, &self.modules, &mut visited, &mut order)?;
        Ok(order)
    }

    /// Canonicalize a module name: strip file extension, normalize path.
    fn canonicalize(&self, name: &str) -> String {
        let name = name.replace('\\', "/");
        let stem = if let Some(dot) = name.rfind('.') {
            if name[dot..].contains('/') {
                name.clone()
            } else {
                name[..dot].to_string()
            }
        } else {
            name
        };
        stem
    }

    /// Normalize a path, removing `.` and `..` components.
    fn normalize_path(path: &Path) -> PathBuf {
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    components.pop();
                }
                std::path::Component::CurDir => {}
                other => components.push(other.as_os_str().to_os_string()),
            }
        }
        components.iter().collect()
    }

    /// AST-based: extract import specifiers from JavaScript source.
    fn extract_imports_ast(source: &str) -> Vec<String> {
        let mut imports = Vec::new();
        let module = match parse_module(source) {
            Ok(m) => m,
            Err(_) => return imports,
        };

        for item in &module.body {
            match item {
                ModuleItem::ModuleDecl(decl) => {
                    let src = match decl {
                        ModuleDecl::Import(import) => Some(&import.src),
                        ModuleDecl::ExportNamed(e) => e.src.as_ref(),
                        ModuleDecl::ExportAll(e) => Some(&e.src),
                        ModuleDecl::ExportDefaultExpr(_)
                        | ModuleDecl::ExportDefaultDecl(_)
                        | ModuleDecl::ExportDecl(_)
                        | ModuleDecl::TsImportEquals(_)
                        | ModuleDecl::TsExportAssignment(_)
                        | ModuleDecl::TsNamespaceExport(_) => None,
                    };
                    if let Some(src) = src {
                        let specifier = src.value.to_string_lossy();
                        if !specifier.starts_with("node:") {
                            imports.push(specifier.into_owned());
                        }
                    }
                }
                _ => {}
            }
        }
        imports
    }

    /// AST-based: transform ESModule source to CommonJS using swc.
    ///
    /// Uses `swc_ecma_transforms_module`'s `common_js` pass to perform the
    /// transformation, then replaces `require` with `__require` to match the
    /// runtime's module loading convention.
    fn transform_to_cjs_ast(source: &str) -> String {
        let module = match parse_module(source) {
            Ok(m) => m,
            Err(e) => return format!("/* transform error: {} */\n{}", e, source),
        };

        let mut program = Program::Module(module);

        GLOBALS.set(&Globals::default(), || {
            let unresolved_mark = Mark::new();

            let mut pass = common_js(
                Resolver::default(),
                unresolved_mark,
                CjsConfig {
                    allow_top_level_this: true,
                    strict: false,
                    strict_mode: false,
                    ..Default::default()
                },
                FeatureFlag {
                    support_block_scoping: true,
                    support_arrow: true,
                },
            );

            // Apply the transform pass
            pass.process(&mut program);

            // Generate code from the transformed AST
            let cm: Lrc<SourceMap> = Default::default();
            let mut buf = Vec::new();
            {
                let mut emitter = Emitter {
                    cfg: CodegenConfig::default(),
                    cm: cm.clone(),
                    comments: None,
                    wr: JsWriter::new(cm, "\n", &mut buf, None),
                };
                let _ = emitter.emit_program(&program);
            }

            let output = String::from_utf8(buf).unwrap_or_default();
            // Replace `require` with `__require` to match the runtime convention
            output.replace("require(", "__require(")
        })
    }
}

/// Parse JavaScript/TypeScript source into a Module AST.
fn parse_module(source: &str) -> Result<Module, String> {
    GLOBALS.set(&Globals::default(), || {
        let cm: Lrc<SourceMap> = Default::default();
        let fm = cm.new_source_file(
            Lrc::new(FileName::Custom("module.ts".into())),
            source.to_string(),
        );

        let lexer = Lexer::new(
            Syntax::Typescript(TsSyntax::default()),
            EsVersion::Es2022,
            StringInput::from(&*fm),
            None,
        );
        let mut parser = Parser::new_from(lexer);

        parser
            .parse_module()
            .map_err(|e| format!("parse error: {:?}", e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_imports() {
        let source = r#"
import { foo } from "./foo";
import "side-effect";
import './bar';
export { baz } from './baz';
// import { ignored } from "./ignored";
"#;
        let imports = ModuleRegistry::extract_imports_ast(source);
        assert!(imports.contains(&"./foo".to_string()));
        assert!(imports.contains(&"side-effect".to_string()));
        assert!(imports.contains(&"./bar".to_string()));
        assert!(imports.contains(&"./baz".to_string()));
        assert!(!imports.contains(&"./ignored".to_string()));
    }

    #[test]
    fn test_resolve_relative() {
        let mut reg = ModuleRegistry::new();
        reg.register("./foo", "export const x = 1;".to_string());
        reg.register("./bar", "export const y = 2;".to_string());

        let resolved = reg.resolve("./foo", "main.js");
        assert_eq!(resolved, Some("./foo".to_string()));

        let resolved = reg.resolve("./nonexistent", "main.js");
        assert_eq!(resolved, None);
    }

    #[test]
    fn test_transform_import_to_require() {
        let source = "import { foo } from \"./bar\";\nfoo();\n";
        let cjs = ModuleRegistry::transform_to_cjs_ast(source);
        assert!(cjs.contains("__require"));
        assert!(cjs.contains("./bar"));
    }

    #[test]
    fn test_transform_export_function() {
        let source = "export function main() { return 42; }\n";
        let cjs = ModuleRegistry::transform_to_cjs_ast(source);
        assert!(cjs.contains("exports"), "CJS should contain exports");
        assert!(cjs.contains("main"));
    }

    #[test]
    fn test_transform_export_default() {
        let source = "export default 42;\n";
        let cjs = ModuleRegistry::transform_to_cjs_ast(source);
        assert!(cjs.contains("exports"), "CJS should contain exports");
    }

    #[test]
    fn test_order_dependencies() {
        let mut reg = ModuleRegistry::new();
        reg.register("main", "import { x } from \"./a\";\nexport function main() {}".to_string());
        reg.register("./a", "import { y } from \"./b\";\nexport const x = 1;".to_string());
        reg.register("./b", "export const y = 2;".to_string());

        let order = reg.order_dependencies("main").unwrap();
        assert_eq!(order.len(), 3);
        // ./b should come before ./a, ./a before main
        let b_pos = order.iter().position(|n| n == "./b").unwrap();
        let a_pos = order.iter().position(|n| n == "./a").unwrap();
        let main_pos = order.iter().position(|n| n == "main").unwrap();
        assert!(b_pos < a_pos);
        assert!(a_pos < main_pos);
    }

    #[test]
    fn test_normalize_path() {
        let path = Path::new("a/b/../c/./d");
        let normalized = ModuleRegistry::normalize_path(path);
        assert_eq!(normalized, Path::new("a/c/d"));
    }
}

