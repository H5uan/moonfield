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
        let imports = Self::extract_imports(&source);
        let cjs_source = Self::transform_to_cjs(&source, &imports);
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

    /// Transform ESModule source to CommonJS.
    ///
    /// Handles:
    /// - `import { x } from "./foo"` → `const { x } = require("./foo")`
    /// - `import x from "./foo"` → `const { default: x } = require("./foo")`
    /// - `import * as x from "./foo"` → `const x = require("./foo")`
    /// - `import "./foo"` → `require("./foo")`
    /// - `export function foo() {}` → `function foo() {} exports.foo = foo`
    /// - `export default x` → `exports.default = x`
    /// - `export { x, y }` → `exports.x = x; exports.y = y`
    /// - `export { x as y }` → `exports.y = x`
    /// - `export const x = 1` → `const x = 1; exports.x = x`
    /// - `export class X {}` → `class X {} exports.X = X`
    fn transform_to_cjs(source: &str, _imports: &[String]) -> String {
        let mut output = String::new();
        let mut export_names = Vec::new();

        for line in source.lines() {
            let trimmed = line.trim();

            // Skip comments.
            if trimmed.starts_with("//") {
                output.push_str(line);
                output.push('\n');
                continue;
            }

            // Handle `import x from "..."` and `import { x } from "..."`
            if trimmed.starts_with("import ") && trimmed.contains("from") {
                let after_import = trimmed[6..].trim().to_string();
                let (bindings, specifier) = if let Some(from_pos) = after_import.rfind(" from ") {
                    let bind = &after_import[..from_pos];
                    let spec = &after_import[from_pos + 6..];
                    (bind.trim(), Self::extract_string_literal(spec.trim()))
                } else {
                    // Keep the line as-is.
                    output.push_str(line);
                    output.push('\n');
                    continue;
                };

                if let Some(spec) = specifier {
                    output.push_str(&format!("    const {} = __require(\"{}\");\n", 
                        transform_import_bindings(bindings), spec));
                } else {
                    output.push_str(line);
                    output.push('\n');
                }
                continue;
            }

            // Handle `import "..."` (side-effect imports)
            if trimmed.starts_with("import ") && !trimmed.contains("from") {
                if let Some(spec) = Self::extract_string_literal(&trimmed[6..].trim()) {
                    output.push_str(&format!("    __require(\"{}\");\n", spec));
                } else {
                    output.push_str(line);
                    output.push('\n');
                }
                continue;
            }

            // Handle `export default ...`
            if trimmed.starts_with("export default ") {
                let expr = &trimmed[14..].trim();
                output.push_str(line);
                // Add the export assignment if it's a declaration.
                if !expr.starts_with('{') && !expr.starts_with('[') && !expr.starts_with('`') {
                    output.push_str("\nexports.default = default_export;\n");
                } else {
                    output.push_str("\nexports.default = ");
                    output.push_str(expr);
                    output.push_str(";\n");
                }
                continue;
            }

            // Handle `export { x, y }` and `export { x as y }`
            if trimmed.starts_with("export {") && trimmed.ends_with('}') {
                let inner = &trimmed[7..trimmed.len() - 1].trim();
                for item in inner.split(',') {
                    let item = item.trim();
                    if let Some(as_pos) = item.find(" as ") {
                        let local = &item[..as_pos].trim();
                        let exported = &item[as_pos + 4..].trim();
                        output.push_str(&format!("exports.{} = {};\n", exported, local));
                    } else {
                        output.push_str(&format!("exports.{} = {};\n", item, item));
                    }
                }
                continue;
            }

            // Handle `export { x, y } from "..."` (re-exports)
            if trimmed.starts_with("export {") && trimmed.contains("from") {
                if let Some(from_pos) = trimmed.rfind(" from ") {
                    let inner = &trimmed[7..from_pos].trim();
                    let spec = &trimmed[from_pos + 6..].trim();
                    if let Some(spec) = Self::extract_string_literal(spec) {
                        for item in inner.split(',') {
                            let item = item.trim();
                            if let Some(as_pos) = item.find(" as ") {
                                let local = &item[..as_pos].trim();
                                let exported = &item[as_pos + 4..].trim();
                                output.push_str(&format!(
                                    "exports.{} = __require(\"{}\").{};\n",
                                    exported, spec, local
                                ));
                            } else {
                                output.push_str(&format!(
                                    "exports.{} = __require(\"{}\").{};\n",
                                    item, spec, item
                                ));
                            }
                        }
                    }
                }
                continue;
            }

            // Handle `export function name(...)` or `export async function name(...)`
            if trimmed.starts_with("export function ") || trimmed.starts_with("export async function ") {
                let after_export = if trimmed.starts_with("export async function ") {
                    &trimmed[22..]
                } else {
                    &trimmed[15..]
                };
                // Extract function name.
                if let Some(paren) = after_export.find('(') {
                    let name = &after_export[..paren].trim();
                    export_names.push(name.to_string());
                }
                // Remove "export " prefix.
                output.push_str(&line.replacen("export ", "", 1));
                output.push('\n');
                continue;
            }

            // Handle `export class Name ...`
            if trimmed.starts_with("export class ") {
                let after_export = &trimmed[12..].trim();
                if let Some(brace) = after_export.find('{') {
                    let name = &after_export[..brace].trim();
                    export_names.push(name.to_string());
                } else if let Some(extends_pos) = after_export.find("extends ") {
                    let name = &after_export[..extends_pos].trim();
                    export_names.push(name.to_string());
                }
                // Remove "export " prefix.
                output.push_str(&line.replacen("export ", "", 1));
                output.push('\n');
                continue;
            }

            // Handle `export const/let/var ...`
            if trimmed.starts_with("export const ") || trimmed.starts_with("export let ") || trimmed.starts_with("export var ") {
                let after_export = &trimmed[6..].trim(); // remove "export "
                // Remove the const/let/var keyword.
                let decl_keyword_end = after_export.find(' ').unwrap_or(after_export.len());
                let after_keyword = &after_export[decl_keyword_end + 1..].trim();
                // Extract the first declared name.
                if let Some(assign) = after_keyword.find('=') {
                    let name = &after_keyword[..assign].trim();
                    // Handle destructuring.
                    if name.starts_with('{') || name.starts_with('[') {
                        output.push_str(&line.replacen("export ", "", 1));
                        output.push('\n');
                    } else {
                        let name = name.trim_end_matches(';').trim();
                        output.push_str(&line.replacen("export ", "", 1));
                        output.push('\n');
                        export_names.push(name.to_string());
                    }
                } else {
                    // Just a declaration without assignment, e.g. `export let x;`
                    output.push_str(&line.replacen("export ", "", 1));
                    output.push('\n');
                    // Extract the name (it's the first word after const/let/var).
                    let decl_part = after_keyword.trim_end_matches(';').trim();
                    if let Some(comma) = decl_part.find(',') {
                        let name = &decl_part[..comma].trim();
                        export_names.push(name.to_string());
                    } else {
                        export_names.push(decl_part.to_string());
                    }
                }
                continue;
            }

            // Handle `export * from "..."` (barrel re-export)
            if trimmed.starts_with("export * from ") {
                if let Some(spec) = Self::extract_string_literal(&trimmed[13..].trim()) {
                    output.push_str(&format!(
                        "const __barrel = __require(\"{}\");\n\
                         Object.keys(__barrel).forEach(k => exports[k] = __barrel[k]);\n",
                        spec
                    ));
                }
                continue;
            }

            // Handle `export * as name from "..."`
            if trimmed.starts_with("export * as ") && trimmed.contains("from") {
                if let Some(from_pos) = trimmed.rfind(" from ") {
                    let ns_name = &trimmed[11..from_pos].trim();
                    if let Some(spec) = Self::extract_string_literal(&trimmed[from_pos + 6..].trim()) {
                        output.push_str(&format!(
                            "exports.{} = __require(\"{}\");\n",
                            ns_name, spec
                        ));
                    }
                }
                continue;
            }

            // Handle `export = ...` (TypeScript)
            if trimmed.starts_with("export = ") {
                let expr = &trimmed[8..].trim();
                output.push_str(&format!("module.exports = {};\n", expr));
                continue;
            }

            // Pass through all other lines.
            output.push_str(line);
            output.push('\n');
        }

        // Add exports assignments for tracked names.
        for name in &export_names {
            output.push_str(&format!("exports.{} = {};\n", name, name));
        }

        output
    }

    /// Extract a string literal from a source fragment (e.g. `"foo"` or `'foo'`).
    /// Handles trailing characters like `";` or `');`.
    fn extract_string_literal(s: &str) -> Option<String> {
        let s = s.trim();
        for quote in ['"', '\''] {
            if s.starts_with(quote) {
                // Find the closing quote, handling trailing characters.
                if let Some(end) = s[1..].find(quote) {
                    return Some(s[1..=end].to_string());
                }
            }
        }
        None
    }

    /// Extract import specifiers from JavaScript source.
    fn extract_imports(source: &str) -> Vec<String> {
        let mut imports = Vec::new();
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }

            if let Some(from_pos) = trimmed.find("from") {
                let before_from = &trimmed[..from_pos].trim();
                if before_from.starts_with("import") || before_from.starts_with("export") {
                    let after_from = &trimmed[from_pos + 4..].trim();
                    for quote in ['"', '\''] {
                        if let Some(start) = after_from.find(quote) {
                            let rest = &after_from[start + 1..];
                            if let Some(end) = rest.find(quote) {
                                let specifier = &rest[..end];
                                if !specifier.starts_with("node:") {
                                    imports.push(specifier.to_string());
                                }
                                break;
                            }
                        }
                    }
                }
            }

            if trimmed.starts_with("import ") && !trimmed.contains("from") {
                for quote in ['"', '\''] {
                    if let Some(start) = trimmed.find(quote) {
                        let rest = &trimmed[start + 1..];
                        if let Some(end) = rest.find(quote) {
                            imports.push(rest[..end].to_string());
                            break;
                        }
                    }
                }
            }
        }
        imports
    }
}

/// Transform import bindings like `{ x, y as z }` or `* as name` or `defaultName`.
///
/// Returns the right-hand side of the `const ... = __require(...)` assignment.
/// For named imports, this includes the braces, e.g. `{ value }` or `{ x, y: z }`.
/// For default imports, this is `{ default: name }`.
/// For namespace imports, this is just the name.
fn transform_import_bindings(bindings: &str) -> String {
    let bindings = bindings.trim();

    // `import * as name from "..."` → `const name = require("...")`
    if bindings.starts_with("* as ") {
        return bindings[5..].trim().to_string();
    }

    // `import defaultName from "..."` → `const { default: defaultName } = require("...")`
    if !bindings.starts_with('{') {
        let name = bindings.trim();
        if name.contains(',') {
            // `import defaultName, { x } from "..."` → `const { default: defaultName, x } = require("...")`
            let parts: Vec<&str> = name.splitn(2, ',').collect();
            let default_name = parts[0].trim();
            let rest = parts[1].trim();
            return format!("{{ default: {}, {} }}", default_name, &rest[1..rest.len() - 1]);
        }
        return format!("{{ default: {} }}", name);
    }

    // `import { x, y as z } from "..."` → `const { x, y: z } = require("...")`
    let inner = &bindings[1..bindings.len() - 1].trim();
    let mut parts = Vec::new();
    for item in inner.split(',') {
        let item = item.trim();
        if let Some(as_pos) = item.find(" as ") {
            let local = &item[..as_pos].trim();
            let exported = &item[as_pos + 4..].trim();
            parts.push(format!("{}: {}", exported, local));
        } else {
            parts.push(item.to_string());
        }
    }
    format!("{{ {} }}", parts.join(", "))
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
        let imports = ModuleRegistry::extract_imports(source);
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
        let imports = ModuleRegistry::extract_imports(source);
        let cjs = ModuleRegistry::transform_to_cjs(source, &imports);
        assert!(cjs.contains("__require"));
        assert!(cjs.contains("./bar"));
    }

    #[test]
    fn test_transform_export_function() {
        let source = "export function main() { return 42; }\n";
        let imports = ModuleRegistry::extract_imports(source);
        let cjs = ModuleRegistry::transform_to_cjs(source, &imports);
        assert!(cjs.contains("exports.main"));
        assert!(cjs.contains("function main"));
    }

    #[test]
    fn test_transform_export_default() {
        let source = "export default 42;\n";
        let imports = ModuleRegistry::extract_imports(source);
        let cjs = ModuleRegistry::transform_to_cjs(source, &imports);
        assert!(cjs.contains("exports.default"));
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

