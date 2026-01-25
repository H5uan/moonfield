use ash::vk;
use tracing::{debug, error};

use crate::types::RhiError;

#[macro_export]
macro_rules! vk_check {
    ($vk_call:expr) => {
        $crate::backend::vulkan::utils::check_vk_result(
            $vk_call,
            stringify!($vk_call),
            file!(),
            line!(),
            None,
        )
    };
    ($vk_call:expr, $context:expr) => {
        $crate::backend::vulkan::utils::check_vk_result(
            $vk_call,
            stringify!($vk_call),
            file!(),
            line!(),
            Some($context),
        )
    };
}

/// 内部函数，执行Vulkan结果检查
fn check_vk_result(
    result: vk::Result, function_name: &str, file: &str, line: u32,
    context: Option<&str>,
) -> Result<(), RhiError> {
    if result != vk::Result::SUCCESS {
        let error_name = format_vk_result(result);
        let message = if let Some(ctx) = context {
            format!(
                "Vulkan function failed: {} at {}:{} - {}. Result: {}",
                function_name, file, line, ctx, error_name
            )
        } else {
            format!(
                "Vulkan function failed: {} at {}:{}. Result: {}",
                function_name, file, line, error_name
            )
        };

        error!("{}", message);

        // 根据错误类型映射到合适的RhiError
        let rhi_error = match result {
            vk::Result::ERROR_OUT_OF_HOST_MEMORY
            | vk::Result::ERROR_OUT_OF_DEVICE_MEMORY => {
                RhiError::OutOfMemory(message)
            }
            vk::Result::ERROR_DEVICE_LOST => RhiError::DriverError(message),
            vk::Result::ERROR_INITIALIZATION_FAILED => {
                RhiError::InitializationFailed(message)
            }
            _ => RhiError::ValidationError(message),
        };

        return Err(rhi_error);
    }

    // 在调试模式下记录成功的调用
    debug!("Vulkan function succeeded: {} at {}:{}", function_name, file, line);

    Ok(())
}

/// 将VkResult转换为可读的错误名称
fn format_vk_result(result: vk::Result) -> String {
    match result {
        vk::Result::SUCCESS => "SUCCESS".to_string(),
        vk::Result::NOT_READY => "NOT_READY".to_string(),
        vk::Result::TIMEOUT => "TIMEOUT".to_string(),
        vk::Result::EVENT_SET => "EVENT_SET".to_string(),
        vk::Result::EVENT_RESET => "EVENT_RESET".to_string(),
        vk::Result::INCOMPLETE => "INCOMPLETE".to_string(),
        vk::Result::ERROR_OUT_OF_HOST_MEMORY => {
            "ERROR_OUT_OF_HOST_MEMORY".to_string()
        }
        vk::Result::ERROR_OUT_OF_DEVICE_MEMORY => {
            "ERROR_OUT_OF_DEVICE_MEMORY".to_string()
        }
        vk::Result::ERROR_INITIALIZATION_FAILED => {
            "ERROR_INITIALIZATION_FAILED".to_string()
        }
        vk::Result::ERROR_DEVICE_LOST => "ERROR_DEVICE_LOST".to_string(),
        vk::Result::ERROR_MEMORY_MAP_FAILED => {
            "ERROR_MEMORY_MAP_FAILED".to_string()
        }
        vk::Result::ERROR_LAYER_NOT_PRESENT => {
            "ERROR_LAYER_NOT_PRESENT".to_string()
        }
        vk::Result::ERROR_EXTENSION_NOT_PRESENT => {
            "ERROR_EXTENSION_NOT_PRESENT".to_string()
        }
        vk::Result::ERROR_FEATURE_NOT_PRESENT => {
            "ERROR_FEATURE_NOT_PRESENT".to_string()
        }
        vk::Result::ERROR_INCOMPATIBLE_DRIVER => {
            "ERROR_INCOMPATIBLE_DRIVER".to_string()
        }
        vk::Result::ERROR_TOO_MANY_OBJECTS => {
            "ERROR_TOO_MANY_OBJECTS".to_string()
        }
        vk::Result::ERROR_FORMAT_NOT_SUPPORTED => {
            "ERROR_FORMAT_NOT_SUPPORTED".to_string()
        }
        vk::Result::ERROR_FRAGMENTED_POOL => {
            "ERROR_FRAGMENTED_POOL".to_string()
        }
        vk::Result::ERROR_UNKNOWN => "ERROR_UNKNOWN".to_string(),
        _ => format!("UNKNOWN_ERROR({:?})", result),
    }
}

#[macro_export]
macro_rules! vk_check_ret {
    ($vk_call:expr) => {
        match $crate::backend::vulkan::utils::check_vk_result(
            $vk_call,
            stringify!($vk_call),
            file!(),
            line!(),
            None,
        ) {
            Ok(_) => {}
            Err(e) => return Err(e),
        }
    };
    ($vk_call:expr, $context:expr) => {
        match $crate::backend::vulkan::utils::check_vk_result(
            $vk_call,
            stringify!($vk_call),
            file!(),
            line!(),
            Some($context),
        ) {
            Ok(_) => {}
            Err(e) => return Err(e),
        }
    };
}

#[macro_export]
macro_rules! vk_assert {
    ($vk_call:expr) => {
        if cfg!(debug_assertions) {
            match $crate::backend::vulkan::utils::check_vk_result(
                $vk_call,
                stringify!($vk_call),
                file!(),
                line!(),
                None,
            ) {
                Ok(_) => {}
                Err(e) => panic!("Vulkan assertion failed: {:?}", e),
            }
        } else {
            let _ = $vk_call;
        }
    };
    ($vk_call:expr, $context:expr) => {
        if cfg!(debug_assertions) {
            match $crate::backend::vulkan::utils::check_vk_result(
                $vk_call,
                stringify!($vk_call),
                file!(),
                line!(),
                Some($context),
            ) {
                Ok(_) => {}
                Err(e) => panic!("Vulkan assertion failed: {:?}", e),
            }
        } else {
            let _ = $vk_call;
        }
    };
}
