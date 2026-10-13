//! Render phases turn extracted scene entities into sorted, drawable items.
//!
//! A phase is a per-view, per-frame collection of [`PhaseItem`]s. Items are
//! pure data queued by feature systems; stateless [`RenderCommand`]s —
//! registered once per phase type in a [`DrawFunctions`] resource — know how
//! to record each item's GPU work through a
//! [`TrackedRenderPass`](crate::TrackedRenderPass). Pass systems iterate the
//! phase and dispatch the item's registered command, so a pass never names
//! the renderable types it draws.

use moonfield_app::prelude::{App, IntoSystemConfigs, Plugin, Query, Render, World};
use moonfield_ecs::{Entity, SystemParam, SystemParamItem, SystemState};
use std::any::TypeId;
use std::marker::PhantomData;

use crate::TrackedRenderPass;
use crate::scene::ExtractedView;
use crate::schedule::{PhaseSort, Queue};

/// A drawable queued by a feature for one view's phase.
///
/// Pure data: extraction and preparation produce it, the phase sorts it, and
/// the item's registered draw function records it. [`PhaseItem::draw_function`]
/// selects the draw function from the phase's [`DrawFunctions`] registry.
pub trait PhaseItem: Sized + Send + Sync + 'static {
    /// Key used to sort items within the phase.
    type SortKey: Ord;

    /// The phase-relative sort key (e.g. camera-space depth).
    fn sort_key(&self) -> Self::SortKey;

    /// The registered draw function that records this item.
    fn draw_function(&self) -> DrawFunctionId<Self>;
}

/// A stateless draw command for phase `P`: `render` records one item's GPU
/// work, fetching its inputs once through `Param` instead of re-reading the
/// world per statement. Registered per phase type in [`DrawFunctions`].
pub trait RenderCommand<P: PhaseItem>: Send + Sync + 'static {
    /// The command's inputs, fetched from the render world per item.
    type Param: SystemParam;

    /// Record `item`'s draws into `pass`.
    fn render(
        world: &World,
        item: &P,
        pass: &mut TrackedRenderPass,
        param: SystemParamItem<Self::Param>,
    );
}

/// How one registered command records an item: the object-safe wrapper the
/// [`DrawFunctions`] registry stores. Holds the command's [`SystemState`],
/// so params persist across items the way a function system's do.
pub trait DrawFunction<P: PhaseItem>: Send + Sync {
    /// Record `item`'s draws into `pass`, reading prepared data from the
    /// render world.
    fn draw(&mut self, world: &World, item: &P, pass: &mut TrackedRenderPass);
}

/// One entry in a phase's command registry: the command's index, bound to
/// its phase at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawFunctionId<P: PhaseItem> {
    index: u32,
    _phase: PhantomData<fn() -> P>,
}

/// Registered commands for one phase type (`P`), a render-world resource.
/// Features register their commands once; pass systems dispatch items by
/// the id the queue system put on the item.
pub struct DrawFunctions<P: PhaseItem> {
    entries: Vec<(TypeId, Box<dyn DrawFunction<P>>)>,
}

impl<P: PhaseItem> Default for DrawFunctions<P> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<P: PhaseItem> DrawFunctions<P> {
    /// Register command `C` for this phase and return its id.
    pub fn register<C: RenderCommand<P>>(&mut self) -> DrawFunctionId<P> {
        let id = DrawFunctionId {
            index: self.entries.len() as u32,
            _phase: PhantomData,
        };
        self.entries.push((
            TypeId::of::<C>(),
            Box::new(RenderCommandState::<P, C>::new()),
        ));
        id
    }

    /// The id command `C` is registered under, if it is.
    pub fn id<C: RenderCommand<P>>(&self) -> Option<DrawFunctionId<P>> {
        let type_id = TypeId::of::<C>();
        self.entries
            .iter()
            .position(|(registered, _)| *registered == type_id)
            .map(|index| DrawFunctionId {
                index: index as u32,
                _phase: PhantomData,
            })
    }

    /// The command registered under `id`, if any. Registered commands are
    /// `'static`, so the returned trait object carries the `'static` bound.
    pub fn get_mut(
        &mut self,
        id: DrawFunctionId<P>,
    ) -> Option<&mut (dyn DrawFunction<P> + 'static)> {
        self.entries
            .get_mut(id.index as usize)
            .map(|(_, function)| function.as_mut())
    }
}

/// The [`DrawFunction`] wrapper stored in a [`DrawFunctions`] registry: the
/// command's [`SystemState`] next to its [`RenderCommand::render`].
pub struct RenderCommandState<P: PhaseItem, C: RenderCommand<P>> {
    state: SystemState<C::Param>,
    _marker: PhantomData<fn() -> (P, C)>,
}

impl<P: PhaseItem, C: RenderCommand<P>> RenderCommandState<P, C> {
    /// Initialize the command's param state.
    pub fn new() -> Self {
        Self {
            state: SystemState::new(),
            _marker: PhantomData,
        }
    }
}

impl<P: PhaseItem, C: RenderCommand<P>> Default for RenderCommandState<P, C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: PhaseItem, C: RenderCommand<P>> DrawFunction<P> for RenderCommandState<P, C> {
    fn draw(&mut self, world: &World, item: &P, pass: &mut TrackedRenderPass) {
        let param = self.state.get(world);
        C::render(world, item, pass, param);
    }
}

/// `Queue` set system: attach an empty phase `P` to every extracted view;
/// feature queue systems fill the phases afterwards.
pub fn prepare_phase<P: PhaseItem>(world: &mut World) {
    let views: Vec<Entity> = world
        .query::<&ExtractedView>()
        .map(|(entity, _)| entity)
        .collect();
    for entity in views {
        world.insert_component(entity, RenderPhase::<P>::default());
    }
}

/// System: sort every [`RenderPhase`] holding items of type `P`. Register
/// one instantiation per phase in the `PhaseSort` set.
pub fn sort_phase<P: PhaseItem>(mut phases: Query<&mut RenderPhase<P>>) {
    for (_, mut phase) in phases.iter_mut() {
        phase.sort();
    }
}

/// Registers the per-phase plumbing every sorted phase needs: phase
/// components attached to views in `Queue`, sorting in `PhaseSort`. The
/// feature still registers its own queue systems and commands.
pub struct SortedPhasePlugin<P: PhaseItem>(PhantomData<fn() -> P>);

impl<P: PhaseItem> Default for SortedPhasePlugin<P> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<P: PhaseItem> Plugin for SortedPhasePlugin<P> {
    fn name(&self) -> &str {
        "moonfield_render_core::SortedPhasePlugin"
    }

    fn build(&self, app: &mut App) {
        app.add_render_systems(Render, prepare_phase::<P>.in_set::<Queue>());
        app.add_render_systems(Render, sort_phase::<P>.in_set::<PhaseSort>());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackedRenderPass;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct TestItem(f32);

    impl PhaseItem for TestItem {
        type SortKey = OrderedFloat;

        fn sort_key(&self) -> Self::SortKey {
            OrderedFloat(self.0)
        }

        fn draw_function(&self) -> DrawFunctionId<Self> {
            unreachable!("not queued in this test")
        }
    }

    struct CommandA;
    struct CommandB;

    impl RenderCommand<TestItem> for CommandA {
        type Param = ();

        fn render(
            _world: &World,
            _item: &TestItem,
            _pass: &mut TrackedRenderPass,
            _param: SystemParamItem<Self::Param>,
        ) {
        }
    }

    impl RenderCommand<TestItem> for CommandB {
        type Param = ();

        fn render(
            _world: &World,
            _item: &TestItem,
            _pass: &mut TrackedRenderPass,
            _param: SystemParamItem<Self::Param>,
        ) {
        }
    }

    #[test]
    fn test_registry_ids_are_typed_and_indexed() {
        let mut functions = DrawFunctions::<TestItem>::default();
        let a = functions.register::<CommandA>();
        let b = functions.register::<CommandB>();

        // Type-directed lookup matches the registration ids.
        assert_eq!(functions.id::<CommandA>(), Some(a));
        assert_eq!(functions.id::<CommandB>(), Some(b));
        assert_ne!(a, b);
        // Ids resolve to their entries.
        assert!(functions.get_mut(a).is_some());
        assert!(functions.get_mut(b).is_some());
    }
}

/// `f32` wrapper usable as an [`Ord`] sort key (camera-space depth and other
/// measured distances are not natively `Ord`).
#[derive(Debug, Clone, Copy)]
pub struct OrderedFloat(pub f32);

impl PartialEq for OrderedFloat {
    fn eq(&self, other: &Self) -> bool {
        self.0.total_cmp(&other.0) == std::cmp::Ordering::Equal
    }
}

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// One view's sorted collection of phase items, rebuilt every frame.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPhase<P: PhaseItem> {
    items: Vec<P>,
}

impl<P: PhaseItem> Default for RenderPhase<P> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<P: PhaseItem> RenderPhase<P> {
    /// Queue `item` for this phase. Items are unsorted until [`RenderPhase::sort`].
    pub fn add(&mut self, item: P) {
        self.items.push(item);
    }

    /// Sort items by their sort key.
    pub fn sort(&mut self) {
        self.items.sort_by_key(|item| item.sort_key());
    }

    /// Sorted phase items.
    pub fn items(&self) -> &[P] {
        &self.items
    }

    /// Whether this phase contains no draw items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
