//! A human-readable entity name, ported from `bevy_ecs::name::Name`.
//!
//! Purely informational: nothing in the ECS reads it, but the editor's
//! hierarchy panel displays it, and it is handy for debugging.

use std::fmt;

/// A human-readable name for an entity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

impl Name {
    /// Create a new name from any string-like type.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Change the name.
    pub fn set(&mut self, name: impl Into<String>) {
        self.0 = name.into();
    }
}

impl Default for Name {
    fn default() -> Self {
        Self::new("")
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_roundtrip() {
        let mut name = Name::new("Cube");
        assert_eq!(name.as_str(), "Cube");
        name.set("Parent Cube");
        assert_eq!(name.to_string(), "Parent Cube");
    }
}
