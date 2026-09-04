//! Training data sources.
//!
//! A dataset is an ordered set of training views. What a view is stays
//! method-defined: for Gaussian Splatting it is a COLMAP-registered camera
//! plus its image (the COLMAP loader lives in
//! `moonfield_render_feature::splat::io::colmap`).

/// An ordered source of training views, sampled by index each step.
pub trait Dataset {
    /// One training view, as defined by the method.
    type View;

    /// Number of views in the dataset.
    fn len(&self) -> usize;

    /// Whether the dataset holds no views.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the view at `index` (`index < self.len()`).
    fn view(&self, index: usize) -> Self::View;
}
