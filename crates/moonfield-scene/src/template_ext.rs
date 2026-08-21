//! [`HandleTemplate`]: a [`Template`] that resolves to an asset [`Handle`].
//!
//! Mirrors Bevy 0.20's `HandleTemplate`: a template is either a path to load
//! through the [`AssetServer`] or an already-resolved handle. Building is
//! synchronous — the file is read (or the path cache hit served) on the
//! calling thread.

use std::fmt;
use std::path::PathBuf;

use moonfield_asset::{AssetServer, Assets, Handle};
use moonfield_ecs::{Template, TemplateContext, TemplateError};

/// A template that builds into a [`Handle<T>`].
///
/// Only the [`Path`](Self::Path) variant is representable in scene files (as
/// a plain path string): the registry's custom save hooks resolve
/// [`Handle`](Self::Handle) variants back to their source path before
/// writing, so a resolved handle never appears in a document.
pub enum HandleTemplate<T: 'static> {
    /// Load (or serve from the path cache) the asset at this path via the
    /// world's [`AssetServer`] resource.
    Path(PathBuf),
    /// An already-resolved handle; building returns it unchanged.
    Handle(Handle<T>),
}

// Manual `Debug` so `HandleTemplate<T>` stays printable regardless of `T`
// (`Handle<T>` is `Copy` even when `T` is not). There is deliberately no
// `Clone`: moonfield-ecs blanket-implements `Template` for every
// `Clone + 'static` type, which would collide with the custom impl below
// (Bevy 0.20 dodges this with pseudo-specialization, which the miniature
// skips).
impl<T: 'static> fmt::Debug for HandleTemplate<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => f.debug_tuple("Path").field(path).finish(),
            Self::Handle(handle) => f.debug_tuple("Handle").field(handle).finish(),
        }
    }
}

impl<T: Send + Sync + 'static> Template for HandleTemplate<T> {
    type Output = Handle<T>;

    fn build(&self, ctx: &mut TemplateContext) -> Result<Self::Output, TemplateError> {
        match self {
            Self::Handle(handle) => Ok(*handle),
            Self::Path(path) => {
                // `AssetServer::load` needs `&mut AssetServer` and
                // `&mut Assets<T>` at once, but the world's resource storage
                // hands out one borrow per resource; take the server out,
                // use it, and put it back (also on the error path).
                let mut server = ctx.world.remove_resource::<AssetServer>().ok_or(
                    TemplateError::MissingResource(std::any::type_name::<AssetServer>()),
                )?;
                let result = match ctx.world.get_resource_mut::<Assets<T>>() {
                    Some(mut assets) => server
                        .load(&mut assets, path)
                        .map_err(|err| TemplateError::Build(format!("{err}"))),
                    None => Err(TemplateError::MissingResource(std::any::type_name::<
                        Assets<T>,
                    >())),
                };
                ctx.world.insert_resource(server);
                result
            }
        }
    }
}

impl<T: 'static> serde::Serialize for HandleTemplate<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Path(path) => serializer.serialize_str(&path.to_string_lossy()),
            Self::Handle(_) => Err(serde::ser::Error::custom(
                "HandleTemplate::Handle cannot appear in a scene file; \
                 resolve it to a path first",
            )),
        }
    }
}

impl<'de, T: 'static> serde::Deserialize<'de> for HandleTemplate<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::Path(PathBuf::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_ecs::World;
    use std::path::Path;

    #[test]
    fn test_handle_variant_builds_to_the_same_handle() {
        let mut assets = Assets::<String>::default();
        let handle = assets.add("payload".to_string());
        let template = HandleTemplate::<String>::Handle(handle);

        let mut world = World::new();
        let built = template
            .build(&mut TemplateContext { world: &mut world })
            .unwrap();
        assert_eq!(built, handle);
    }

    #[test]
    fn test_path_variant_reports_missing_resources() {
        let template = HandleTemplate::<String>::Path(PathBuf::from("cloud.fake"));

        // No AssetServer at all.
        let mut world = World::new();
        let err = template
            .build(&mut TemplateContext { world: &mut world })
            .unwrap_err();
        assert!(matches!(err, TemplateError::MissingResource(_)));

        // Server present, but no Assets<String>: the server must be put back
        // even though the build failed.
        let mut world = World::new();
        world.insert_resource(AssetServer::default());
        let err = template
            .build(&mut TemplateContext { world: &mut world })
            .unwrap_err();
        assert!(matches!(err, TemplateError::MissingResource(_)));
        assert!(world.contains_resource::<AssetServer>());
    }

    #[test]
    fn test_serialization_is_path_string_only() {
        let template = HandleTemplate::<String>::Path(PathBuf::from("cloud.fake"));
        let json = serde_json::to_string(&template).unwrap();
        assert_eq!(json, "\"cloud.fake\"");

        let parsed: HandleTemplate<String> = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, HandleTemplate::Path(p) if p == Path::new("cloud.fake")));

        // A resolved handle is never written to a file.
        let mut assets = Assets::<String>::default();
        let handle = assets.add("payload".to_string());
        assert!(serde_json::to_string(&HandleTemplate::<String>::Handle(handle)).is_err());
    }
}
