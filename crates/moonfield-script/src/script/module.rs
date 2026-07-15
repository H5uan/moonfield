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
//! 3. Bare specifiers (`"lodash"`) — resolved via `node_modules/` lookup,
//!    `package.json` main field, and `index.js` fallback.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use swc_common::sync::Lrc;
use swc_common::{FileName, Globals, SourceMap, GLOBALS};
use swc_ecma_ast::*;
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax, TsSyntax};

#[cfg(feature = "quickjs-backend")]
use swc_common::Mark;
#[cfg(feature = "quickjs-backend")]
use swc_ecma_codegen::{text_writer::JsWriter, Config as CodegenConfig, Emitter};
#[cfg(feature = "quickjs-backend")]
use swc_ecma_transforms_module::common_js::{common_js, Config as CjsConfig, FeatureFlag};
#[cfg(feature = "quickjs-backend")]
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
#[derive(Clone)]
pub struct ModuleRegistry {
    modules: HashMap<String, ModuleInfo>,
    base_path: PathBuf,
    search_dirs: Vec<PathBuf>,
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleRegistry {
    /// Create a new, empty registry with default search directories.
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            base_path: PathBuf::from("."),
            search_dirs: vec![
                PathBuf::from("."),
                PathBuf::from("scripts"),
                PathBuf::from("target/scripts"),
            ],
        }
    }

    /// Set the base path for relative module resolution.
    pub fn with_base_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.base_path = path.into();
        self
    }

    /// Set the search directories for bare specifier resolution.
    pub fn with_search_dirs(mut self, dirs: Vec<PathBuf>) -> Self {
        self.search_dirs = dirs;
        self
    }

    /// Register a module by name with its source code.
    ///
    /// Uses regex-based import extraction (backends don't need AST parsing for this).
    /// For QuickJS, also transforms ESModule syntax to CommonJS.
    /// Returns the canonical name.
    pub fn register(&mut self, name: &str, source: String) -> String {
        let canonical = self.canonicalize(name);
        let imports = Self::extract_imports(&source);
        let cjs_source = self.transform_to_cjs(&source);
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

    /// Transform source to CommonJS, if the QuickJS backend is active.
    /// For V8 backend, returns the source as-is (native ESM support).
    fn transform_to_cjs(&self, source: &str) -> String {
        #[cfg(feature = "quickjs-backend")]
        return Self::transform_to_cjs_ast(source);
        #[cfg(not(feature = "quickjs-backend"))]
        source.to_string()
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

    /// Full resolution chain: supports relative paths, bare specifiers,
    /// `node_modules` lookup, `package.json` main field, and `index.js` fallback.
    ///
    /// Uses the registry's configured `search_dirs` for bare specifier resolution.
    /// Returns the resolved canonical name and the file path it was found at.
    pub fn resolve_full(&self, specifier: &str, base: &str) -> Option<(String, PathBuf)> {
        // 1. Relative specifier: resolve against the base module's directory.
        if specifier.starts_with('.') || specifier.starts_with('/') {
            let base_dir = if specifier.starts_with('/') {
                PathBuf::from(".")
            } else {
                let p = Path::new(base).parent().unwrap_or(Path::new("."));
                p.to_path_buf()
            };
            let resolved = Self::normalize_path(&base_dir.join(specifier));
            let dir = resolved.parent();
            let stem = resolved.file_name().and_then(|n| n.to_str()).unwrap_or("");
            return Self::resolve_file_or_dir(&resolved, &resolved, stem, dir);
        }

        // 2. Bare specifier: search search_dirs + node_modules walk.
        for search_dir in &self.search_dirs {
            // Check search_dir directly.
            let candidate = search_dir.join(specifier);
            let dir = candidate.parent();
            if let Some(result) = Self::resolve_file_or_dir(&candidate, &candidate, specifier, dir)
            {
                return Some(result);
            }

            // Search node_modules/ under this directory and its ancestors.
            if let Some(result) = Self::resolve_in_node_modules(specifier, search_dir) {
                return Some(result);
            }
        }

        // 3. Fallback: check if already registered.
        if self.modules.contains_key(specifier) {
            return Some((specifier.to_string(), PathBuf::from(specifier)));
        }
        // Try with .js and .ts extensions.
        for ext in &["js", "ts", "mjs", "cjs"] {
            let with_ext = format!("{}.{}", specifier, ext);
            if self.modules.contains_key(&with_ext) {
                return Some((with_ext.to_string(), PathBuf::from(with_ext)));
            }
        }

        None
    }

    /// Recursively resolve and register all dependencies of a module.
    ///
    /// Uses the full resolution chain: relative paths, bare specifiers,
    /// `node_modules` lookup, `package.json` main field, and `index.js` fallback.
    /// Newly discovered modules are loaded from disk and registered automatically.
    pub fn resolve_dependencies(&mut self, name: &str) -> Result<(), String> {
        let deps: Vec<String> = {
            let info = self
                .get(name)
                .ok_or_else(|| format!("module '{}' not found", name))?;
            info.imports.clone()
        };

        for dep in &deps {
            // Try the in-registry resolve first (fast path).
            let resolved = self.resolve(dep, name);

            if let Some(resolved) = resolved {
                if !self.contains(&resolved) {
                    // Found in registry metadata but not yet loaded — find it on disk.
                    let (_, path) = self
                        .resolve_full(dep, name)
                        .ok_or_else(|| format!("cannot resolve '{}' from '{}'", dep, name))?;
                    let source = Self::load_source(&path)?;
                    self.register(&resolved, source);
                    self.resolve_dependencies(&resolved)?;
                }
            } else {
                // Full resolution chain for bare specifiers and node_modules.
                let (canonical, path) = self
                    .resolve_full(dep, name)
                    .ok_or_else(|| format!("cannot resolve '{}' from '{}'", dep, name))?;

                if !self.contains(&canonical) {
                    let source = Self::load_source(&path)?;
                    self.register(&canonical, source);
                    self.resolve_dependencies(&canonical)?;
                }
            }
        }

        Ok(())
    }

    /// Read a source file from disk, dispatching to `load_script` for TS→JS resolution.
    fn load_source(path: &Path) -> Result<String, String> {
        crate::script::load_script(path)
            .map_err(|e| format!("failed to load '{}': {}", path.display(), e))
    }

    /// Try to resolve `specifier` within `node_modules/` directories,
    /// walking up from `start_dir` to the filesystem root.
    fn resolve_in_node_modules(specifier: &str, start_dir: &Path) -> Option<(String, PathBuf)> {
        let mut current = Some(start_dir);
        while let Some(dir) = current {
            let nm = dir.join("node_modules").join(specifier);
            if let Some(result) = Self::resolve_file_or_dir(&nm, &nm, specifier, nm.parent()) {
                return Some(result);
            }
            current = dir.parent();
        }
        None
    }

    /// Given a candidate path, try these in order:
    /// 1. exact path + `.js` / `.ts` / `.mjs` / `.cjs`
    /// 2. `package.json` → `main` field
    /// 3. `index.js` / `index.ts` / `index.mjs` / `index.cjs`
    fn resolve_file_or_dir(
        candidate: &Path,
        _display_path: &Path,
        _specifier: &str,
        _parent_dir: Option<&Path>,
    ) -> Option<(String, PathBuf)> {
        // 1. Direct file with extension.
        if candidate.is_file() {
            if let Some(name) = candidate.to_str() {
                return Some((name.to_string(), candidate.to_path_buf()));
            }
        }

        // 2. Try adding extensions.
        for ext in &["js", "ts", "mjs", "cjs"] {
            let with_ext = candidate.with_extension(ext);
            if with_ext.is_file() {
                let name = with_ext.to_string_lossy().replace('\\', "/");
                return Some((name, with_ext));
            }
        }

        // 3. If candidate is a directory, check package.json and index files.
        if candidate.is_dir() {
            // Check package.json main field.
            let pkg_json = candidate.join("package.json");
            if pkg_json.is_file() {
                if let Some(main) = Self::read_package_json_main(&pkg_json) {
                    let main_path = candidate.join(&main);
                    // Resolve main_path relative to the candidate directory.
                    let resolved = Self::normalize_path(&main_path);
                    let dir = resolved.parent();
                    let stem = resolved.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if let Some(result) = Self::resolve_file_or_dir(&resolved, &resolved, stem, dir)
                    {
                        return Some(result);
                    }
                }
            }

            // Check index files.
            for ext in &["js", "ts", "mjs", "cjs"] {
                let index = candidate.join(format!("index.{}", ext));
                if index.is_file() {
                    let name = index.to_string_lossy().replace('\\', "/");
                    return Some((name, index));
                }
            }
        }

        None
    }

    /// Read the `main` field from a `package.json` file.
    fn read_package_json_main(pkg_json: &Path) -> Option<String> {
        let content = fs::read_to_string(pkg_json).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
        parsed
            .get("main")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Find the canonical name of the module that corresponds to the given
    /// file path. Used by hot reload to match absolute file-watcher paths
    /// to relative canonical names in the registry.
    pub fn find_by_file_path(&self, path: &Path) -> Option<String> {
        let path_str = path.to_string_lossy().replace('\\', "/");
        // Strip file extension to get the path stem.
        let path_stem = match path_str.rfind('.') {
            Some(dot) if !path_str[dot..].contains('/') => &path_str[..dot],
            _ => &path_str,
        };

        for info in self.modules.values() {
            let name = &info.name;
            // Normalize: strip "./" prefix.
            let normalized = name.strip_prefix("./").unwrap_or(name);
            if path_stem.ends_with(normalized) || normalized == path_stem {
                return Some(info.name.clone());
            }
        }
        None
    }

    /// Compute the set of modules that transitively import `target`
    /// (including `target` itself). This is the "affected set" for
    /// incremental hot reload: only these modules need re-compilation
    /// when `target` changes.
    pub fn transitive_importers(&self, target: &str) -> std::collections::HashSet<String> {
        // Build reverse dependency map: for each module, who imports it?
        let mut reverse_deps: HashMap<String, Vec<String>> = HashMap::new();
        for (name, info) in &self.modules {
            for imp in &info.imports {
                // Resolve the import to a canonical name.
                if let Some(resolved) = self.resolve(imp, name) {
                    reverse_deps.entry(resolved).or_default().push(name.clone());
                }
            }
        }

        // BFS from target to find all transitive importers.
        let mut affected = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(target.to_string());
        while let Some(module) = queue.pop_front() {
            if affected.insert(module.clone()) {
                if let Some(importers) = reverse_deps.get(&module) {
                    for importer in importers {
                        queue.push_back(importer.clone());
                    }
                }
            }
        }
        affected
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
                    let base_dir = Path::new(name).parent().unwrap_or(Path::new("."));
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

    /// AST-based: extract import specifiers from JavaScript/TypeScript source.
    ///
    /// Uses swc to parse the source into an AST, then walks module-level
    /// declarations to find `import` and `export ... from` specifiers.
    /// More robust than regex — won't match import-like patterns inside
    /// strings, comments, or regex literals.
    fn extract_imports(source: &str) -> Vec<String> {
        let mut imports = Vec::new();
        let module = match parse_module(source) {
            Ok(m) => m,
            Err(_) => return imports,
        };

        for item in &module.body {
            if let ModuleItem::ModuleDecl(decl) = item {
                let src = match decl {
                    ModuleDecl::Import(import) => Some(&import.src),
                    ModuleDecl::ExportNamed(e) => e.src.as_ref(),
                    ModuleDecl::ExportAll(e) => Some(&e.src),
                    _ => None,
                };
                if let Some(src) = src {
                    let specifier = src.value.to_string_lossy();
                    if !specifier.starts_with("node:") {
                        imports.push(specifier.into_owned());
                    }
                }
            }
        }
        imports
    }

    /// AST-based: transform ESModule source to CommonJS using swc.
    ///
    /// Uses `swc_ecma_transforms_module`'s `common_js` pass to perform the
    /// transformation, then replaces `require` with `__require` to match the
    /// runtime's module loading convention.
    #[cfg(feature = "quickjs-backend")]
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
"#;
        let imports = ModuleRegistry::extract_imports(source);
        assert!(imports.contains(&"./foo".to_string()));
        assert!(imports.contains(&"side-effect".to_string()));
        assert!(imports.contains(&"./bar".to_string()));
        assert!(imports.contains(&"./baz".to_string()));
    }

    #[test]
    fn test_extract_imports_default_export() {
        let source = "import x from \"./foo\";\nimport * as y from \"./bar\";\n";
        let imports = ModuleRegistry::extract_imports(source);
        assert!(imports.contains(&"./foo".to_string()));
        assert!(imports.contains(&"./bar".to_string()));
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

    #[cfg(feature = "quickjs-backend")]
    #[test]
    fn test_transform_import_to_require() {
        let source = "import { foo } from \"./bar\";\nfoo();\n";
        let cjs = ModuleRegistry::transform_to_cjs_ast(source);
        assert!(cjs.contains("__require"));
        assert!(cjs.contains("./bar"));
    }

    #[cfg(feature = "quickjs-backend")]
    #[test]
    fn test_transform_export_function() {
        let source = "export function main() { return 42; }\n";
        let cjs = ModuleRegistry::transform_to_cjs_ast(source);
        assert!(cjs.contains("exports"), "CJS should contain exports");
        assert!(cjs.contains("main"));
    }

    #[cfg(feature = "quickjs-backend")]
    #[test]
    fn test_transform_export_default() {
        let source = "export default 42;\n";
        let cjs = ModuleRegistry::transform_to_cjs_ast(source);
        assert!(cjs.contains("exports"), "CJS should contain exports");
    }

    #[test]
    fn test_order_dependencies() {
        let mut reg = ModuleRegistry::new();
        reg.register(
            "main",
            "import { x } from \"./a\";\nexport function main() {}".to_string(),
        );
        reg.register(
            "./a",
            "import { y } from \"./b\";\nexport const x = 1;".to_string(),
        );
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
