//! [`ResolvedScene`]: the type-erased, ready-to-apply form of a scene.
//!
//! This is the second stage of the two-stage pipeline (templates → resolved
//! scene → spawned entities), a synchronous miniature of Bevy 0.20's
//! `ResolvedScene`: a node holds the templates of one entity plus the
//! resolved scenes of its children, and [`apply`](ResolvedScene::apply)
//! spawns the whole subtree into a [`World`].

use moonfield_ecs::{ChildOf, Component, Entity, Template, TemplateContext, World};

use crate::SceneError;

/// A type-erased template that inserts its built output onto an entity.
///
/// Blanket-implemented for every [`Template`] whose output is a
/// [`Component`], so plain `Clone` components and [`HandleTemplate`](crate::HandleTemplate)
/// both work without any wrapper.
///
/// The method is named `insert_into_world`, not `insert`: with the blanket
/// impl, every `Clone + 'static` type (including `HashMap`/`serde_json::Map`)
/// implements this trait, and a `&self` method named `insert` would shadow
/// their inherent `&mut self` `insert` wherever the trait is in scope.
pub trait SceneTemplate {
    /// Build the template's output and insert it onto `entity`.
    fn insert_into_world(&self, world: &mut World, entity: Entity) -> Result<(), SceneError>;
}

impl<T> SceneTemplate for T
where
    T: Template,
    T::Output: Component,
{
    fn insert_into_world(&self, world: &mut World, entity: Entity) -> Result<(), SceneError> {
        let output = self.build(&mut TemplateContext { world })?;
        world.insert_component(entity, output).ok_or_else(|| {
            SceneError::Invalid(format!("entity {entity:?} vanished while applying a scene"))
        })?;
        Ok(())
    }
}

/// A fully resolved scene subtree: the templates of one entity and the
/// resolved scenes of its children.
///
/// Construct it directly (for code-authored scenes) or let
/// [`load_scene`](crate::load_scene) build it from a document.
pub struct ResolvedScene {
    templates: Vec<Box<dyn SceneTemplate>>,
    children: Vec<ResolvedScene>,
}

impl ResolvedScene {
    /// A resolved scene from an entity's templates and its child subtrees.
    pub fn new(templates: Vec<Box<dyn SceneTemplate>>, children: Vec<ResolvedScene>) -> Self {
        Self {
            templates,
            children,
        }
    }

    /// Spawn the subtree into `world`, returning the new root entity.
    ///
    /// Every child subtree is applied recursively and linked to its parent
    /// with a [`ChildOf`] component. Precondition: the world must have
    /// called [`World::register_hierarchy`], otherwise the link hooks do not
    /// fire and `Children` is never maintained.
    pub fn apply(&self, world: &mut World) -> Result<Entity, SceneError> {
        let entity = world.spawn_empty();
        for template in &self.templates {
            template.insert_into_world(world, entity)?;
        }
        for child in &self.children {
            let child_entity = child.apply(world)?;
            world
                .insert_component(child_entity, ChildOf(entity))
                .ok_or_else(|| {
                    SceneError::Invalid(format!(
                        "entity {child_entity:?} vanished while applying a scene"
                    ))
                })?;
        }
        Ok(entity)
    }

    /// Flatten the subtree into a list of childless scenes (this node's
    /// templates first, then every descendant). Used when the hierarchy
    /// mapping is not registered: every node in the document still spawns,
    /// but no `ChildOf` links are created.
    pub fn flatten(self) -> Vec<ResolvedScene> {
        let mut out = vec![ResolvedScene::new(self.templates, Vec::new())];
        for child in self.children {
            out.extend(child.flatten());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_ecs::{Children, Name};

    #[derive(Debug, Clone, PartialEq)]
    struct Marker(u32);

    #[test]
    fn test_apply_inserts_templates_and_links_children() {
        let scene = ResolvedScene::new(
            vec![Box::new(Name::new("root")), Box::new(Marker(1))],
            vec![ResolvedScene::new(
                vec![Box::new(Name::new("child")), Box::new(Marker(2))],
                vec![ResolvedScene::new(
                    vec![Box::new(Name::new("grand"))],
                    vec![],
                )],
            )],
        );

        let mut world = World::new();
        world.register_hierarchy();
        let root = scene.apply(&mut world).unwrap();

        assert_eq!(world.get_component::<Name>(root).unwrap().as_str(), "root");
        assert_eq!(world.get_component::<Marker>(root).unwrap().0, 1);

        let children = world.get_component::<Children>(root).unwrap();
        assert_eq!(children.len(), 1);
        let child = children[0];
        assert_eq!(
            world.get_component::<ChildOf>(child).unwrap().parent(),
            root
        );
        assert_eq!(world.get_component::<Marker>(child).unwrap().0, 2);

        let grandchildren = world.get_component::<Children>(child).unwrap();
        assert_eq!(grandchildren.len(), 1);
        assert_eq!(
            world
                .get_component::<Name>(grandchildren[0])
                .unwrap()
                .as_str(),
            "grand"
        );
    }

    #[test]
    fn test_flatten_drops_links_but_keeps_templates() {
        let scene = ResolvedScene::new(
            vec![Box::new(Name::new("root"))],
            vec![ResolvedScene::new(
                vec![Box::new(Name::new("child"))],
                vec![],
            )],
        );

        let mut world = World::new();
        world.register_hierarchy();
        let entities: Vec<Entity> = scene
            .flatten()
            .iter()
            .map(|s| s.apply(&mut world).unwrap())
            .collect();
        let mut names: Vec<String> = entities
            .iter()
            .map(|&e| world.get_component::<Name>(e).unwrap().as_str().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["child", "root"]);
        // No ChildOf links were created.
        assert!(world.query::<&ChildOf>().next().is_none());
    }
}
