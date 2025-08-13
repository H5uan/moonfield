use std::any::{Any, TypeId};

/// ComponentProvider allows you to dynamically look up inner components of an object with their type id
/// 
/// This trait provides a type-safe way to query whether an object contains components of specific types,
/// commonly used in Entity Component Systems (ECS) or other scenarios requiring dynamic type queries.
/// 
/// # Examples
/// 
/// ```rust
/// use std::any::Any;
/// use moonfield_core::type_traits::ComponentProvider;
/// use moonfield_core::impl_component_provider;
/// 
/// struct MyEntity {
///     position: Position,
///     velocity: Velocity,
/// }
/// 
/// struct Position { x: f32, y: f32 }
/// struct Velocity { dx: f32, dy: f32 }
/// 
/// // Use macro to implement ComponentProvider
/// impl_component_provider!(MyEntity, position: Position, velocity: Velocity);
/// 
/// let entity = MyEntity {
///     position: Position { x: 1.0, y: 2.0 },
///     velocity: Velocity { dx: 0.5, dy: -0.3 },
/// };
/// 
/// // Convert to trait object to use convenience methods
/// let provider: &dyn ComponentProvider = &entity;
/// 
/// // Query components
/// if let Some(pos) = provider.component_ref::<Position>() {
///     println!("Position: ({}, {})", pos.x, pos.y);
/// }
/// ```
pub trait ComponentProvider {
    /// Query an immutable reference to a component by type ID
    /// 
    /// # Parameters
    /// 
    /// * `type_id` - The TypeId of the component type to query
    /// 
    /// # Returns
    /// 
    /// Returns `Some(&dyn Any)` if a component of the corresponding type is found, otherwise `None`
    fn query_component_ref(&self, type_id: TypeId) -> Option<&dyn Any>;
    
    /// Query a mutable reference to a component by type ID
    /// 
    /// # Parameters
    /// 
    /// * `type_id` - The TypeId of the component type to query
    /// 
    /// # Returns
    /// 
    /// Returns `Some(&mut dyn Any)` if a component of the corresponding type is found, otherwise `None`
    fn query_component_mut(&mut self, type_id: TypeId) -> Option<&mut dyn Any>;
}

impl dyn ComponentProvider {
    /// Try to get an immutable reference to a component of the specified type
    /// 
    /// This is a convenience method that internally calls `query_component_ref` and performs type conversion.
    /// 
    /// # Type Parameters
    /// 
    /// * `T` - The component type to query, must implement the `Any` trait
    /// 
    /// # Returns
    /// 
    /// Returns `Some(&T)` if a component of the corresponding type is found, otherwise `None`
    #[inline]
    pub fn component_ref<T: Any>(&self) -> Option<&T> {
        self.query_component_ref(TypeId::of::<T>())
            .and_then(|component| component.downcast_ref())
    }

    /// Try to get a mutable reference to a component of the specified type
    /// 
    /// This is a convenience method that internally calls `query_component_mut` and performs type conversion.
    /// 
    /// # Type Parameters
    /// 
    /// * `T` - The component type to query, must implement the `Any` trait
    /// 
    /// # Returns
    /// 
    /// Returns `Some(&mut T)` if a component of the corresponding type is found, otherwise `None`
    #[inline]
    pub fn component_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.query_component_mut(TypeId::of::<T>())
            .and_then(|component| component.downcast_mut())
    }

    /// Check if a component of the specified type exists
    /// 
    /// # Type Parameters
    /// 
    /// * `T` - The component type to check, must implement the `Any` trait
    /// 
    /// # Returns
    /// 
    /// Returns `true` if a component of the specified type exists, otherwise `false`
    #[inline]
    pub fn has_component<T: Any>(&self) -> bool {
        self.query_component_ref(TypeId::of::<T>()).is_some()
    }
}

/// Macro for quickly implementing the ComponentProvider trait
/// 
/// This macro supports two usage modes:
/// 
/// 1. **Simple object mode**: Only allows querying the object's own type
///    ```rust
///    use moonfield_core::impl_component_provider;
///    
///    struct MyStruct;
///    impl_component_provider!(MyStruct);
///    ```
/// 
/// 2. **Component mode**: Allows querying the object's own type and specified component fields
///    ```rust
///    use moonfield_core::impl_component_provider;
///    
///    struct MyStruct {
///        field1: ComponentType1,
///        field2: NestedStruct,
///    }
///    
///    struct ComponentType1;
///    struct NestedStruct {
///        subfield: ComponentType2,
///    }
///    struct ComponentType2;
///    
///    impl_component_provider!(MyStruct, field1: ComponentType1, field2.subfield: ComponentType2);
///    ```
/// 
/// # Parameters
/// 
/// * `$target_type` - The target type to implement ComponentProvider for
/// * `$component_field` - Component field path (supports nested fields like `field.subfield`)
/// * `$component_type` - The type of the component field
/// 
/// # Examples
/// 
/// ```rust
/// use moonfield_core::impl_component_provider;
/// 
/// struct Entity {
///     transform: Transform,
///     health: Health,
/// }
/// 
/// struct Transform {
///     x: f32,
///     y: f32,
/// }
/// 
/// struct Health {
///     current: i32,
///     max: i32,
/// }
/// 
/// // Implement ComponentProvider, allowing queries for Transform and Health
/// impl_component_provider!(
///     Entity,
///     transform: Transform,
///     health: Health
/// );
/// 
/// // Can also query nested fields
/// impl_component_provider!(
///     Transform,
///     x: f32,
///     y: f32
/// );
/// ```
#[macro_export]
macro_rules! impl_component_provider {
    // Simple mode: only allows querying the object's own type
    ($target_type:ty) => {
        impl $crate::type_traits::ComponentProvider for $target_type {
            #[inline]
            fn query_component_ref(
                &self, 
                type_id: std::any::TypeId,
            ) -> Option<&dyn std::any::Any> {
                if type_id == std::any::TypeId::of::<Self>() {
                    Some(self)
                } else {
                    None
                }
            }

            #[inline]
            fn query_component_mut(
                &mut self, 
                type_id: std::any::TypeId,
            ) -> Option<&mut dyn std::any::Any> {
                if type_id == std::any::TypeId::of::<Self>() {
                    Some(self)
                } else {
                    None
                }
            }
        }
    };

    // Component mode: allows querying the object's own type and specified component fields
    ($target_type:ty, $($($component_field:ident).+ : $component_type:ty),+ $(,)?) => {
        impl $crate::type_traits::ComponentProvider for $target_type {
            fn query_component_ref(
                &self, 
                type_id: std::any::TypeId,
            ) -> Option<&dyn std::any::Any> {
                // First check if querying the object's own type
                if type_id == std::any::TypeId::of::<Self>() {
                    return Some(self);
                }

                // Then check each component field
                $(
                    if type_id == std::any::TypeId::of::<$component_type>() {
                        return Some(&self.$($component_field).+);
                    }
                )+

                None
            }

            fn query_component_mut(
                &mut self, 
                type_id: std::any::TypeId,
            ) -> Option<&mut dyn std::any::Any> {
                // First check if querying the object's own type
                if type_id == std::any::TypeId::of::<Self>() {
                    return Some(self);
                }

                // Then check each component field
                $(
                    if type_id == std::any::TypeId::of::<$component_type>() {
                        return Some(&mut self.$($component_field).+);
                    }
                )+

                None
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Position {
        x: f32,
        y: f32,
    }

    #[derive(Debug, PartialEq)]
    struct Velocity {
        dx: f32,
        dy: f32,
    }

    #[derive(Debug)]
    struct Transform {
        position: Position,
        scale: f32,
    }

    #[derive(Debug)]
    struct Entity {
        transform: Transform,
        velocity: Velocity,
    }

    impl_component_provider!(Entity, transform: Transform, velocity: Velocity);
    impl_component_provider!(Transform, position: Position);

    #[test]
    fn test_component_query() {
        let mut entity = Entity {
            transform: Transform {
                position: Position { x: 1.0, y: 2.0 },
                scale: 1.5,
            },
            velocity: Velocity { dx: 0.5, dy: -0.3 },
        };

        // Convert concrete type to trait object to use convenience methods
        let provider: &dyn ComponentProvider = &entity;

        // Test querying the object's own type
        assert!(provider.component_ref::<Entity>().is_some());
        assert!(provider.has_component::<Entity>());

        // Test querying components
        let transform = provider.component_ref::<Transform>().unwrap();
        assert_eq!(transform.scale, 1.5);

        let velocity = provider.component_ref::<Velocity>().unwrap();
        assert_eq!(velocity.dx, 0.5);
        assert_eq!(velocity.dy, -0.3);

        // Test mutable references
        let provider_mut: &mut dyn ComponentProvider = &mut entity;
        let velocity_mut = provider_mut.component_mut::<Velocity>().unwrap();
        velocity_mut.dx = 1.0;
        assert_eq!(entity.velocity.dx, 1.0);

        // Test non-existent components
        let provider: &dyn ComponentProvider = &entity;
        assert!(provider.component_ref::<Position>().is_none());
        assert!(!provider.has_component::<Position>());
    }

    #[test]
    fn test_nested_component_query() {
        let mut transform = Transform {
            position: Position { x: 10.0, y: 20.0 },
            scale: 2.0,
        };

        // Convert concrete type to trait object to use convenience methods
        let provider: &dyn ComponentProvider = &transform;

        // Test nested component queries
        let position = provider.component_ref::<Position>().unwrap();
        assert_eq!(position.x, 10.0);
        assert_eq!(position.y, 20.0);

        // Test mutable references
        let provider_mut: &mut dyn ComponentProvider = &mut transform;
        let position_mut = provider_mut.component_mut::<Position>().unwrap();
        position_mut.x = 15.0;
        assert_eq!(transform.position.x, 15.0);
    }

    // Test simple mode
    #[allow(dead_code)]
    struct SimpleStruct {
        value: i32,
    }

    impl_component_provider!(SimpleStruct);

    #[test]
    fn test_simple_component_provider() {
        let simple = SimpleStruct { value: 42 };
        let provider: &dyn ComponentProvider = &simple;
        
        // Can query the object's own type
        assert!(provider.component_ref::<SimpleStruct>().is_some());
        assert!(provider.has_component::<SimpleStruct>());
        
        // Cannot query other types
        assert!(provider.component_ref::<Position>().is_none());
        assert!(!provider.has_component::<Position>());
    }

    #[test]
    fn test_direct_trait_methods() {
        let mut entity = Entity {
            transform: Transform {
                position: Position { x: 1.0, y: 2.0 },
                scale: 1.5,
            },
            velocity: Velocity { dx: 0.5, dy: -0.3 },
        };

        // Test direct trait method calls
        let type_id = TypeId::of::<Transform>();
        let transform_any = entity.query_component_ref(type_id).unwrap();
        let transform = transform_any.downcast_ref::<Transform>().unwrap();
        assert_eq!(transform.scale, 1.5);

        // Test mutable references
        let velocity_type_id = TypeId::of::<Velocity>();
        let velocity_any = entity.query_component_mut(velocity_type_id).unwrap();
        let velocity = velocity_any.downcast_mut::<Velocity>().unwrap();
        velocity.dx = 2.0;
        assert_eq!(entity.velocity.dx, 2.0);
    }
}
