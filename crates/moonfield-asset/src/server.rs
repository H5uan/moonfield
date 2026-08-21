//! Synchronous [`AssetServer`]: extension-dispatched loading with a
//! path cache on top of [`Assets<T>`].
//!
//! Deliberately minimal, in the crate's spirit: loading happens on the
//! calling thread, the cache is a plain map from `(type, path)` to
//! [`AssetId`], and there is no task pool, no async, no hot reload. Those
//! stay roadmap known-debts.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::{AssetId, Assets, Handle};

/// Loads asset payloads from files of one or more extensions.
///
/// Loaders are stored boxed inside [`AssetServer`] and dispatched by file
/// extension. `load` returns the payload as a type-erased box; the caller
/// ([`AssetServer::load`]) downcasts it to the requested asset type.
///
/// `Send + Sync` because [`AssetServer`] is a world resource, and the
/// blanket `Resource` impl requires it.
pub trait AssetLoader: Send + Sync {
    /// File extensions this loader handles, without the dot (e.g.
    /// `&["ply", "txt"]`).
    fn extensions(&self) -> &'static [&'static str];

    /// Load the asset at `path`, returning the payload type-erased.
    fn load(&self, path: &Path) -> Result<Box<dyn Any>, AssetError>;
}

/// Errors from [`AssetServer::load`] and [`AssetLoader::load`].
#[derive(Debug)]
pub enum AssetError {
    /// Underlying I/O failure while reading the file.
    Io(std::io::Error),
    /// No registered loader handles the path's extension (or the path has
    /// no extension at all).
    UnknownExtension(PathBuf),
    /// The loader's payload did not downcast to the requested asset type.
    TypeMismatch {
        path: PathBuf,
        expected: &'static str,
    },
    /// The loader itself failed (parse errors and the like).
    Loader(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "asset io error: {err}"),
            Self::UnknownExtension(path) => {
                write!(f, "no asset loader for extension of {}", path.display())
            }
            Self::TypeMismatch { path, expected } => write!(
                f,
                "asset loader for {} did not produce a {expected}",
                path.display()
            ),
            Self::Loader(msg) => write!(f, "asset loader failed: {msg}"),
        }
    }
}

impl std::error::Error for AssetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AssetError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Extension-dispatching, path-caching asset loader.
///
/// Stored as a world resource. The cache keys on `(TypeId, PathBuf)` so the
/// same file loaded as two different asset types stays distinct, and so a
/// path is loaded at most once per type.
#[derive(Default)]
pub struct AssetServer {
    loaders: Vec<Box<dyn AssetLoader>>,
    cache: HashMap<(TypeId, PathBuf), AssetId>,
}

impl AssetServer {
    /// Register a loader. Later registrations shadow earlier ones for the
    /// same extension.
    pub fn register_loader<L: AssetLoader + 'static>(&mut self, loader: L) {
        self.loaders.push(Box::new(loader));
    }

    /// Load `path` as a `T`, inserting the payload into `assets`.
    ///
    /// A cached `(T, path)` hit rebuilds the handle without touching the
    /// loader, provided the slot still resolves in `assets`; an asset that
    /// was removed since is loaded again and the cache entry replaced.
    pub fn load<T: 'static>(
        &mut self,
        assets: &mut Assets<T>,
        path: &Path,
    ) -> Result<Handle<T>, AssetError> {
        let key = (TypeId::of::<T>(), path.to_path_buf());
        if let Some(&id) = self.cache.get(&key) {
            let handle = Handle::from_id(id);
            if assets.contains(&handle) {
                return Ok(handle);
            }
        }

        let extension = path.extension().and_then(|ext| ext.to_str());
        let loader = extension.and_then(|ext| {
            self.loaders
                .iter()
                .rev()
                .find(|loader| loader.extensions().contains(&ext))
        });
        let Some(loader) = loader else {
            return Err(AssetError::UnknownExtension(path.to_path_buf()));
        };

        let payload = loader.load(path)?;
        let asset = payload
            .downcast::<T>()
            .map_err(|_| AssetError::TypeMismatch {
                path: path.to_path_buf(),
                expected: std::any::type_name::<T>(),
            })?;
        let handle = assets.add(*asset);
        self.cache.insert(key, handle.id());
        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Loads `.txt` files into `String` assets, counting invocations.
    struct TxtLoader {
        calls: Arc<AtomicUsize>,
    }

    impl AssetLoader for TxtLoader {
        fn extensions(&self) -> &'static [&'static str] {
            &["txt"]
        }

        fn load(&self, path: &Path) -> Result<Box<dyn Any>, AssetError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(std::fs::read_to_string(path)?))
        }
    }

    /// A `.txt` loader whose payload is a `u32`, for type-mismatch tests.
    struct WrongTypeLoader;

    impl AssetLoader for WrongTypeLoader {
        fn extensions(&self) -> &'static [&'static str] {
            &["txt"]
        }

        fn load(&self, _path: &Path) -> Result<Box<dyn Any>, AssetError> {
            Ok(Box::new(7u32))
        }
    }

    fn temp_file(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moonfield-asset-test-{}-{name}",
            std::process::id()
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn test_register_and_dispatch_by_extension() {
        let path = temp_file("dispatch.txt", "hello");
        let mut server = AssetServer::default();
        server.register_loader(TxtLoader {
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut assets = Assets::<String>::default();

        let handle = server.load(&mut assets, &path).unwrap();
        assert_eq!(assets.get(&handle).map(String::as_str), Some("hello"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_load_dedups_path_and_type() {
        let path = temp_file("dedup.txt", "once");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut server = AssetServer::default();
        server.register_loader(TxtLoader {
            calls: Arc::clone(&calls),
        });
        let mut assets = Assets::<String>::default();

        let first = server.load(&mut assets, &path).unwrap();
        let second = server.load(&mut assets, &path).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.id(), second.id());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(assets.len(), 1);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_unknown_extension_errors() {
        let mut server = AssetServer::default();
        server.register_loader(TxtLoader {
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let mut assets = Assets::<String>::default();

        let err = server
            .load(&mut assets, Path::new("cloud.ply"))
            .unwrap_err();
        assert!(matches!(err, AssetError::UnknownExtension(_)));
        // A path with no extension at all is likewise unknown.
        let err = server.load(&mut assets, Path::new("noext")).unwrap_err();
        assert!(matches!(err, AssetError::UnknownExtension(_)));
    }

    #[test]
    fn test_loader_type_mismatch_errors() {
        let path = temp_file("mismatch.txt", "not-a-string");
        let mut server = AssetServer::default();
        server.register_loader(WrongTypeLoader);
        let mut assets = Assets::<String>::default();

        let err = server.load(&mut assets, &path).unwrap_err();
        match err {
            AssetError::TypeMismatch { expected, .. } => {
                assert_eq!(expected, std::any::type_name::<String>());
            }
            other => panic!("expected TypeMismatch, got {other:?}"),
        }
        assert!(assets.is_empty());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_removed_asset_triggers_reload() {
        let path = temp_file("reload.txt", "again");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut server = AssetServer::default();
        server.register_loader(TxtLoader {
            calls: Arc::clone(&calls),
        });
        let mut assets = Assets::<String>::default();

        let first = server.load(&mut assets, &path).unwrap();
        assets.remove(&first);

        // The cached id is stale: the server must reload instead of
        // resurrecting the removed asset.
        let second = server.load(&mut assets, &path).unwrap();
        assert_ne!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(assets.get(&second).map(String::as_str), Some("again"));
        std::fs::remove_file(&path).unwrap();
    }
}
