use crate::World;

/// A unit of work that operates on a [`World`].
///
/// In a real engine systems are scheduled and parallelised; in this minimal
/// ECS a system is simply a function-like object that receives `&mut World`.
pub trait System: Send + Sync + 'static {
    fn run(&mut self, world: &mut World);
}

impl System for Box<dyn System> {
    fn run(&mut self, world: &mut World) {
        (**self).run(world);
    }
}

/// Trait for types that can be turned into a [`System`].
///
/// Implemented for function pointers `fn(&mut World)` and closures.
pub trait IntoSystem {
    fn system(self) -> impl System;
}

impl<F> IntoSystem for F
where
    F: FnMut(&mut World) + Send + Sync + 'static,
{
    fn system(self) -> impl System {
        FnSystem {
            f: self,
        }
    }
}

struct FnSystem<F> {
    f: F,
}

impl<F> System for FnSystem<F>
where
    F: FnMut(&mut World) + Send + Sync + 'static,
{
    fn run(&mut self, world: &mut World) {
        (self.f)(world);
    }
}
