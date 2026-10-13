//! The `Shader` asset: a Slang module's source text, loaded through the
//! asset layer.
//!
//! [`Shader`] is plain source data — the path it was loaded from plus the
//! source text. Compilation, reflection, and root binding stay in
//! `moonfield-rhi`; this crate never touches the compile machinery. Entry
//! points are discovered by Slang reflection at compile time, so the asset
//! carries no entry-point metadata.

use std::any::Any;
use std::path::Path;

use moonfield_asset::{AssetError, AssetLoader};

/// A loaded Slang shader module asset.
#[derive(Debug, Clone)]
pub struct Shader {
    source: String,
    /// The file the source was loaded from; compilers use it as the module
    /// name for diagnostics.
    path: String,
}

impl Shader {
    /// Wrap Slang source text with the path it came from.
    pub fn new(source: String, path: String) -> Self {
        Self { source, path }
    }

    /// The Slang source text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// What the shader was loaded from, for diagnostics.
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Loads `.slang` files into [`Shader`] assets. A synchronous `std::fs` read
/// — the asset layer is sync-only by design.
pub struct SlangLoader;

impl AssetLoader for SlangLoader {
    fn extensions(&self) -> &'static [&'static str] {
        &["slang"]
    }

    fn load(&self, path: &Path) -> Result<Box<dyn Any>, AssetError> {
        let source = std::fs::read_to_string(path)?;
        Ok(Box::new(Shader::new(source, path.display().to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_asset::{AssetServer, Assets};

    fn temp_file(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moonfield-shader-test-{}-{name}",
            std::process::id()
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn test_slang_loader_reads_source_and_path() {
        let path = temp_file("basic.slang", "void main() {}");
        let mut server = AssetServer::default();
        server.register_loader(SlangLoader);
        let mut assets = Assets::<Shader>::default();

        let handle = server.load(&mut assets, &path).unwrap();
        let shader = assets.get(&handle).unwrap();
        assert_eq!(shader.source(), "void main() {}");
        assert_eq!(shader.path(), path.display().to_string());

        // The path cache serves the second load without re-reading.
        let again = server.load(&mut assets, &path).unwrap();
        assert_eq!(handle, again);
        assert_eq!(assets.len(), 1);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_slang_loader_missing_file_errors() {
        let mut server = AssetServer::default();
        server.register_loader(SlangLoader);
        let mut assets = Assets::<Shader>::default();

        let result = server.load(&mut assets, Path::new("does/not/exist.slang"));
        assert!(matches!(result, Err(AssetError::Io(_))));
        assert!(assets.is_empty());
    }
}
