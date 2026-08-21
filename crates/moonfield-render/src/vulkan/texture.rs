use ash::vk;

/// Owned image + view + sampler for bindless texture usage.
#[allow(dead_code)] // integration point for the bindless texture heap
pub struct Texture {
    device: ash::Device,
    image: vk::Image,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
}

impl Texture {
    pub fn new(
        device: ash::Device,
        image: vk::Image,
        image_view: vk::ImageView,
        sampler: vk::Sampler,
    ) -> Self {
        Self {
            device,
            image,
            image_view,
            sampler,
        }
    }
}
