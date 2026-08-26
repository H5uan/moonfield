//! Render phases turn extracted scene entities into sorted, drawable items.
//!
//! A phase is a per-view, per-frame collection of [`PhaseItem`]s. Items are
//! pure data queued by feature systems; draw functions — registered once per
//! phase type in a [`DrawFunctions`] resource — know how to record each
//! item's GPU work. Pass systems iterate the phase and dispatch the item's
//! registered draw function, so a pass never names the renderable types it
//! draws.

use moonfield_app::prelude::World;
use moonfield_rhi::CommandBuffer;
use std::collections::HashMap;

/// A drawable queued by a feature for one view's phase.
///
/// Pure data: extraction and preparation produce it, the phase sorts it, and
/// the item's registered draw function records it. [`PhaseItem::draw_function`]
/// selects the draw function from the phase's [`DrawFunctions`] registry.
pub trait PhaseItem: Send + Sync + 'static {
    /// Key used to sort items within the phase.
    type SortKey: Ord;

    /// The phase-relative sort key (e.g. camera-space depth).
    fn sort_key(&self) -> Self::SortKey;

    /// The registered draw function that records this item.
    fn draw_function(&self) -> DrawFunctionId;
}

/// How to record one phase item's GPU work.
pub trait DrawFunction<P: PhaseItem>: Send + Sync {
    /// Record `item`'s draws into `command_buffer`, reading prepared data
    /// from the render world.
    fn draw(&self, world: &World, item: &P, command_buffer: &CommandBuffer);
}

/// One entry in a phase's draw-function registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawFunctionId(u32);

/// Registered draw functions for one phase type (`P`), a render-world
/// resource. Features register their draw functions once; pass systems look
/// items' ids up here to dispatch.
pub struct DrawFunctions<P: PhaseItem> {
    functions: HashMap<u32, Box<dyn DrawFunction<P>>>,
    next_id: u32,
}

impl<P: PhaseItem> Default for DrawFunctions<P> {
    fn default() -> Self {
        Self {
            functions: HashMap::new(),
            next_id: 0,
        }
    }
}

impl<P: PhaseItem> DrawFunctions<P> {
    /// Register a draw function and return its id.
    pub fn register(&mut self, function: impl DrawFunction<P> + 'static) -> DrawFunctionId {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.functions.insert(id, Box::new(function));
        DrawFunctionId(id)
    }

    /// The draw function registered under `id`, if any.
    pub fn get(&self, id: DrawFunctionId) -> Option<&dyn DrawFunction<P>> {
        self.functions.get(&id.0).map(|function| function.as_ref())
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
