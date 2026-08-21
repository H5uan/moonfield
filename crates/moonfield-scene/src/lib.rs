//! Scene save/load for moonfield: a synchronous miniature of Bevy 0.20's
//! Template/ResolvedScene pipeline with a glTF 2.0 JSON text carrier.
//!
//! The pipeline has two layers:
//!
//! - **Typed templates.** A [`Template`](moonfield_ecs::Template) is plain
//!   data that builds a runtime value inside a [`World`](moonfield_ecs::World); a
//!   [`ResolvedScene`] bundles the templates of one entity subtree and
//!   [`apply`](ResolvedScene::apply)s them, spawning entities and linking
//!   `ChildOf`. There is no reflection at runtime and no async queue —
//!   everything happens on the calling thread.
//! - **The text layer.** [`save_scene`] / [`load_scene`] map a world onto a
//!   glTF 2.0 JSON document (`.gltf`, via `gltf-json`): the node tree carries
//!   the hierarchy, node TRS fields carry [`Transform`](moonfield_math::Transform),
//!   and the root `cameras` array carries [`Camera`](moonfield_render::Camera).
//!   Every other registered component
//!   rides the node's `extras.components.<name>` map as plain JSON.
//!
//! Which components participate — and under which stable short names
//! (`"transform"`, `"camera"`, `"mesh_renderer"`, …, never Rust type paths)
//! — is decided by a [`SceneRegistry`] world resource. Names are stable
//! across renames and releases; the file format is the registry's public
//! contract.

mod file;
mod registry;
mod resolved;
mod template_ext;

pub use file::{load_scene, load_scene_from_file, save_scene, save_scene_to_file, SceneError};
pub use registry::{LoadFn, SaveFn, SceneRegistry, CAMERA, HIERARCHY, NAME, TRANSFORM};
pub use resolved::{ResolvedScene, SceneTemplate};
pub use template_ext::HandleTemplate;
