//! Plugin wiring the ECS hierarchy into an [`App`].

use crate::{App, Plugin, Update};
use moonfield_ecs::{ensure_global_transforms, propagate_transforms, IntoSystemConfigs};

/// Registers the `ChildOf`/`Children` relationship (lifecycle hooks incl.
/// cycle prevention and linked-spawn despawn) and schedules transform
/// propagation in the [`Update`] schedule:
/// [`ensure_global_transforms`] before [`propagate_transforms`].
pub struct HierarchyPlugin;

impl Plugin for HierarchyPlugin {
    fn name(&self) -> &str {
        "moonfield_app::HierarchyPlugin"
    }

    fn build(&self, app: &mut App) {
        app.world_mut().register_hierarchy();
        app.add_systems(
            Update,
            (
                ensure_global_transforms,
                propagate_transforms.after(&ensure_global_transforms),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_ecs::{ChildOf, Children, RelationshipTarget};
    use moonfield_math::{GlobalTransform, Transform, Vec3};

    #[test]
    fn test_hierarchy_plugin_registers_hooks_and_propagates_on_update() {
        let mut app = App::new();
        app.add_plugin(HierarchyPlugin);

        let parent = app.world_mut().spawn((Transform::from_xyz(1.0, 0.0, 0.0),));
        let child = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, 2.0, 0.0), ChildOf(parent)));

        // The plugin-registered hooks maintain the link.
        assert_eq!(
            app.world()
                .get_component::<Children>(parent)
                .unwrap()
                .entities(),
            &[child]
        );

        // One update runs the scheduled propagation systems.
        app.update();
        let global = app.world().get_component::<GlobalTransform>(child).unwrap();
        assert!((global.translation() - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);
    }
}
