use std::{cell::RefCell, rc::Weak, ffi::CString};

use ash::{Entry, Instance, khr, vk};
use winit::window;

use crate::{backend::SharedGraphicsBackend, error::GraphicsError};

pub struct VulkanGraphicsBackend {
    entry: Entry,
    vk_instance: Instance,
    this: RefCell<Option<Weak<VulkanGraphicsBackend>>>,
}

impl VulkanGraphicsBackend {
    pub fn new() -> Result<Self, GraphicsError> {
        unsafe {
            let entry =
                Entry::load().map_err(|e| GraphicsError::VulkanLoadError(format!("{}", e)))?;

            let instance = Self::create_instance(&entry)?;

            Ok(Self {
                entry,
                vk_instance: instance,
                this: RefCell::new(None),
            })
        }
    }

    fn create_instance(entry: &Entry) -> Result<Instance, GraphicsError> {
        unsafe {
            let app_name = CString::new("Moonfield").unwrap();
            let engine_name = CString::new("Moonfield Engine").unwrap();

            let app_info = vk::ApplicationInfo::default()
                .application_name(app_name.as_c_str())
                .application_version(vk::make_api_version(0, 0, 1, 0))
                .engine_name(engine_name.as_c_str())
                .engine_version(vk::make_api_version(0, 0, 1, 0))
                .api_version(vk::make_api_version(0, 1, 0, 0));

            let create_info = vk::InstanceCreateInfo::default()
                .application_info(&app_info);

            entry
                .create_instance(&create_info, None)
                .map_err(|e| GraphicsError::VulkanInstanceCreationError(format!("{}", e)))
        }
    }
}
