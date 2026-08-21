//! Typed construction templates.
//!
//! A [`Template`] is plain, serializable data that knows how to build its
//! runtime [`Output`](Template::Output) inside a [`World`]. This is a
//! synchronous miniature of Bevy 0.20's `Template` trait: there is no async
//! queue machinery, [`Template::build`] runs immediately against the world.

use crate::World;

/// Synchronous context passed to [`Template::build`].
///
/// Carries the world the template is built into, so templates can spawn
/// entities, read resources, or resolve handles while building.
pub struct TemplateContext<'w> {
    /// The world the template is built into.
    pub world: &'w mut World,
}

/// A typed template: plain data that builds a runtime value.
pub trait Template {
    /// The value produced by building this template.
    type Output;

    /// Build the output value, using `ctx` for world access.
    fn build(&self, ctx: &mut TemplateContext) -> Result<Self::Output, TemplateError>;
}

/// Every `Clone` type is its own template: building clones the value. This
/// covers the common case of plain-data components.
impl<T: Clone + 'static> Template for T {
    type Output = T;

    fn build(&self, _ctx: &mut TemplateContext) -> Result<Self::Output, TemplateError> {
        Ok(self.clone())
    }
}

/// Errors produced while building a [`Template`].
///
/// Kept deliberately small; downstream crates wrap this in their own error
/// types when they need more context.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    /// A resource required by the template was missing from the world.
    #[error("missing resource: {0}")]
    MissingResource(&'static str),
    /// Any other build failure, described by a message.
    #[error("template build failed: {0}")]
    Build(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[test]
    fn test_blanket_template_build_roundtrip() {
        let mut world = World::new();
        let template = Position { x: 1.0, y: 2.0 };

        let built = template
            .build(&mut TemplateContext { world: &mut world })
            .unwrap();

        assert_eq!(built, template);
    }
}
