//! Best-effort source map support.
//!
//! When a script is loaded from disk and a sibling `.js.map` source map is
//! available (via a `//# sourceMappingURL=` comment or conventional naming),
//! error locations are remapped from compiled JavaScript positions back to
//! the original TypeScript positions.
//!
//! Only the V8 backend formats locations this way; the QuickJS backend
//! evaluates a bundled module graph whose positions carry no per-file
//! identity.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Caches parsed source maps keyed by script/module name — the name passed
/// as `ScriptOrigin`, which is also what appears in stack traces.
pub(crate) struct SourceMapCache {
    maps: HashMap<String, CachedMap>,
}

struct CachedMap {
    map: sourcemap::SourceMap,
    /// Directory containing the map file; the map's relative `sources`
    /// entries are resolved against it.
    dir: PathBuf,
}

impl SourceMapCache {
    pub(crate) fn new() -> Self {
        Self {
            maps: HashMap::new(),
        }
    }

    /// Try to load a source map for a script named `script_name` (a
    /// path-like name) with the given `source`. Silently does nothing when
    /// no usable map exists — remapping is strictly best-effort.
    ///
    /// Resolution order:
    /// 1. `//# sourceMappingURL=<file>` comment in the source, resolved
    ///    relative to the script's directory.
    /// 2. `<script_name>.map` on disk.
    /// 3. `<script_name>.js.map` on disk.
    pub(crate) fn load_for(&mut self, script_name: &str, source: &str) {
        let script_path = Path::new(script_name);
        let dir = script_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(url) = find_source_mapping_url(source) {
            // Inline data-URI maps are not produced by our tsc config.
            if !url.starts_with("data:") {
                candidates.push(dir.join(url));
            }
        }
        candidates.push(PathBuf::from(format!("{}.map", script_name)));
        candidates.push(PathBuf::from(format!("{}.js.map", script_name)));

        for candidate in &candidates {
            let Ok(bytes) = std::fs::read(candidate) else {
                continue;
            };
            let Ok(map) = sourcemap::SourceMap::from_reader(&bytes[..]) else {
                continue;
            };
            let map_dir = candidate.parent().unwrap_or(Path::new(".")).to_path_buf();
            self.maps
                .insert(script_name.to_string(), CachedMap { map, dir: map_dir });
            return;
        }
    }

    /// Remap a `file:line:col` position (1-based, as V8 reports it) to the
    /// original source position. Returns `None` when no map or mapping
    /// exists for the position.
    pub(crate) fn remap(&self, file: &str, line: u32, col: u32) -> Option<(String, u32, u32)> {
        let cached = self.maps.get(file)?;
        let token = cached
            .map
            .lookup_token(line.checked_sub(1)?, col.checked_sub(1)?)?;
        let source = token.get_source()?;
        let resolved = normalize_path(&cached.dir.join(source));
        Some((
            resolved.to_string_lossy().replace('\\', "/"),
            token.get_src_line() + 1,
            token.get_src_col() + 1,
        ))
    }

    /// Drop maps for modules no longer in the dependency graph.
    pub(crate) fn retain(&mut self, active: &HashSet<&str>) {
        self.maps.retain(|name, _| active.contains(name.as_str()));
    }

    /// Drop all cached maps (used on full runtime reload).
    pub(crate) fn clear(&mut self) {
        self.maps.clear();
    }
}

/// Find the last `//# sourceMappingURL=` (or legacy `//@`) comment.
fn find_source_mapping_url(source: &str) -> Option<&str> {
    source.lines().rev().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("//# sourceMappingURL=")
            .or_else(|| line.strip_prefix("//@ sourceMappingURL="))
            .map(str::trim)
            .filter(|url| !url.is_empty())
    })
}

/// Normalize a path lexically, removing `.` and `..` components (no I/O).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal source map JSON via `SourceMapBuilder` and exercise
    /// discovery + remapping through real files in a temp dir.
    #[test]
    fn test_load_and_remap() {
        let dir =
            std::env::temp_dir().join(format!("moonfield_sourcemap_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let js_path = dir.join("main.js");
        std::fs::write(
            &js_path,
            "function main() {\n    throw new Error('boom');\n}\n\
             //# sourceMappingURL=main.js.map\n",
        )
        .unwrap();

        // Map generated line 2 (0-based 1), col 5 (0-based 4) back to
        // main.ts line 42 (0-based 41), col 5 (0-based 4).
        let mut builder = sourcemap::SourceMapBuilder::new(Some("main.js"));
        builder.add(1, 4, 41, 4, Some("main.ts"), None, false);
        let map = builder.into_sourcemap();
        let mut map_bytes = Vec::new();
        map.to_writer(&mut map_bytes).unwrap();
        std::fs::write(dir.join("main.js.map"), &map_bytes).unwrap();

        let mut cache = SourceMapCache::new();
        let source = std::fs::read_to_string(&js_path).unwrap();
        let name = js_path.to_string_lossy().replace('\\', "/");
        cache.load_for(&name, &source);

        let remapped = cache.remap(&name, 2, 5).expect("position should remap");
        assert!(remapped.0.ends_with("main.ts"), "got: {}", remapped.0);
        assert_eq!((remapped.1, remapped.2), (42, 5));

        // Unmapped script names do not remap.
        assert!(cache.remap("other.js", 2, 5).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
