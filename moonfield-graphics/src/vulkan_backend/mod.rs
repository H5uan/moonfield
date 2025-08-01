use std::{cell::RefCell, ffi::CString, rc::Weak};

use ash::{Entry, Instance, vk};

use crate::{
    backend::GraphicsBackend,
    error::{GraphicsError, VulkanError},
};

pub struct VulkanGraphicsBackend {
    entry: Entry,
    vk_instance: Instance,
    this: RefCell<Option<Weak<VulkanGraphicsBackend>>>,
}

impl VulkanGraphicsBackend {
    pub fn new(
        #[allow(unused_variables)] vsync: bool,
        #[allow(unused_variables)] msaa_sample_count: Option<u8>,
    ) -> Result<Self, GraphicsError> {
        unsafe {
            let entry = Entry::load().map_err(|e| {
                GraphicsError::VulkanError(VulkanError::LoadError(format!("{}", e)))
            })?;

            let instance = Self::create_instance(&entry).map_err(GraphicsError::VulkanError)?;

            Ok(Self {
                entry,
                vk_instance: instance,
                this: RefCell::new(None),
            })
        }
    }

    fn create_instance(entry: &Entry) -> Result<Instance, VulkanError> {
        unsafe {
            let app_name = CString::new("Moonfield").unwrap();
            let engine_name = CString::new("Moonfield Engine").unwrap();

            let app_info = vk::ApplicationInfo::default()
                .application_name(app_name.as_c_str())
                .application_version(vk::make_api_version(0, 0, 1, 0))
                .engine_name(engine_name.as_c_str())
                .engine_version(vk::make_api_version(0, 0, 1, 0))
                .api_version(vk::make_api_version(0, 1, 3, 0));

            let create_info = vk::InstanceCreateInfo::default().application_info(&app_info);

            entry
                .create_instance(&create_info, None)
                .map_err(|e| VulkanError::InstanceCreationError(format!("{}", e)))
        }
    }
}

impl GraphicsBackend for VulkanGraphicsBackend {}
