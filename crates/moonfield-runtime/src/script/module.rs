//! Module system for the scripting runtime.
//!
//! Provides a backend-agnostic [`ModuleRegistry`] that maps module specifiers
//! to source code. Each backend implements its own compilation and linking
//! using the registry as the source of truth.
//!
//! # Resolution strategy
//!
//! Module specifiers are resolved in this order:
//! 1. Relative paths (`./foo`, `../bar/baz`) — resolved against the importer's
//!    directory.
//! 2. Absolute paths — used as-is.
//! 3. Bare specifiers (`"lodash"`) — not yet supported; returns an error.
//!
//! The registry caches source code so that repeated `load()` calls for the
//! same module return the cached source. This enables hot-reload to trigger
//! re-compilation only when the source changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A compiled module, holding its source and resolved dependency list.
#[derive(Clone)]
pub struct ModuleInfo {
    /// The module's canonical name (e.g. `"record_frame"` or `"./utils"`).
    pub name: String,
    /// The JavaScript source code (already transpiled if needed).
    pub source: String,
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
    /// Returns the canonical name for the module (after resolution).
    pub fn register(&mut self, name: &str, source: String) -> String {
        let canonical = self.canonicalize(name);
        let imports = Self::extract_imports(&source);
        self.modules.insert(
            canonical.clone(),
            ModuleInfo {
                name: canonical.clone(),
                source,
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
            // Relative path: resolve against base's directory.
            let base_dir = Path::new(base).parent().unwrap_or(Path::new("."));
            let resolved = base_dir.join(specifier);
            // Normalize the path (remove `./`, `../`, etc.)
            let normalized = Self::normalize_path(&resolved);
            let canonical = normalized.to_string_lossy().replace('\\', "/");

            // Check if the module exists with or without extension.
            if self.modules.contains_key(&canonical) {
                return Some(canonical);
            }
            // Try with .js extension.
            let with_js = format!("{}.js", canonical);
            if self.modules.contains_key(&with_js) {
                return Some(with_js);
            }
            // Try with .ts extension.
            let with_ts = format!("{}.ts", canonical);
            if self.modules.contains_key(&with_ts) {
                return Some(with_ts);
            }
        }

        // Exact match.
        if self.modules.contains_key(specifier) {
            return Some(specifier.to_string());
        }

        None
    }

    /// Canonicalize a module name: strip file extension, normalize path.
    fn canonicalize(&self, name: &str) -> String {
        let name = name.replace('\\', "/");
        // Strip the extension if present.
        let stem = if let Some(dot) = name.rfind('.') {
            if name[dot..].contains('/') {
                // The dot is part of a directory name, not an extension.
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

    /// Extract import specifiers from JavaScript source.
    ///
    /// This is a simple regex-based extraction that handles:
    /// - `import ... from "specifier"`
    /// - `import "specifier"`
    /// - `export ... from "specifier"`
    ///
    /// For production use, a proper parser (swc) would be more accurate, but
    /// this is sufficient for the current use case.
    fn extract_imports(source: &str) -> Vec<String> {
        let mut imports = Vec::new();
        // Match `import ... from "..."` or `import "..."` or `export ... from "..."`
        // Also handles single quotes.
        for line in source.lines() {
            let trimmed = line.trim();
            // Skip comments.
            if trimmed.starts_with("//") {
                continue;
            }

            // Match `import ... from "..."` or `import ... from '...'`
            if let Some(from_pos) = trimmed.find("from") {
                let before_from = &trimmed[..from_pos].trim();
                if before_from.starts_with("import") || before_from.starts_with("export") {
                    // Extract the specifier after `from`.
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

            // Match `import "..."` or `import '...'` (side-effect imports).
            if trimmed.starts_with("import ") && !trimmed.contains("from") {
                for quote in ['"', '\''] {
                    if let Some(start) = trimmed.find(quote) {
                        let rest = &trimmed[start + 1..];
                        if let Some(end) = rest.find(quote) {
                            let specifier = &rest[..end];
                            imports.push(specifier.to_string());
                            break;
                        }
                    }
                }
            }
        }

        imports
    }
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

        // Non-existent module.
        let resolved = reg.resolve("./nonexistent", "main.js");
        assert_eq!(resolved, None);
    }

    #[test]
    fn test_normalize_path() {
        let path = Path::new("a/b/../c/./d");
        let normalized = ModuleRegistry::normalize_path(path);
        assert_eq!(normalized, Path::new("a/c/d"));
    }
}