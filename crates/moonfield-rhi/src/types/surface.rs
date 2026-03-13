use crate::types::format::Format;

#[derive(Debug, Clone, Copy)]
pub struct Extent2D {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct SurfaceCapabilities {
    pub formats: Vec<Format>,
    pub present_modes: Vec<PresentMode>,
    pub min_image_count: u32,
    pub max_image_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentMode {
    Immediate,
    Mailbox,
    Fifo,
}

pub struct SwapchainDescriptor {
    pub surface: std::sync::Arc<dyn crate::Surface>,
    pub format: Format,
    pub extent: Extent2D,
    pub present_mode: PresentMode,
    pub image_count: u32,
}

pub struct SwapchainImage {
    pub index: u32,
    pub(crate) image_view: usize,
    pub wait_semaphore: u64,
    pub signal_semaphore: u64,
}
