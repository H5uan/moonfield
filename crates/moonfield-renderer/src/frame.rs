//! Frame orchestration: the `RenderAlgorithm` phase abstraction.

/// A rendering algorithm driven through three per-frame phases.
///
/// This is a deliberately simplified take on Bevy's
/// extract → prepare → queue → render pipeline:
///
/// - [`extract`](RenderAlgorithm::extract) reads the CPU-side world and pulls
///   out just the data this frame needs.
/// - [`prepare`](RenderAlgorithm::prepare) turns extracted data into GPU
///   buffers / descriptor updates.
/// - [`render`](RenderAlgorithm::render) records the actual draw/dispatch
///   commands.
///
/// There is no render graph yet; phases run sequentially on one thread. A
/// graph scheduler can be layered on later without changing this trait.
pub trait RenderAlgorithm {
    /// The CPU-side scene view the algorithm extracts from.
    ///
    /// TODO(M1): once the editor/runtime wiring lands, this becomes the
    /// `moonfield-ecs` `World` (or a read-only view of it). Kept as an
    /// associated type for now so this crate does not depend on the ECS.
    type World;

    /// Extract this frame's data from the world.
    fn extract(&mut self, world: &Self::World);

    /// Upload extracted data to GPU buffers before recording.
    fn prepare(&mut self);

    /// Record the frame's rendering commands.
    ///
    /// TODO(M2): takes a Lunar Mare command buffer / pass once the first
    /// algorithm (`splat::rasterize`) actually records into the RHI.
    fn render(&self);
}
