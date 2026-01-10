use std::sync::Arc;
use crate::{types::*, Instance, Adapter, Device, Surface, Swapchain, ShaderModule, Pipeline, Buffer, CommandPool, CommandBuffer, Queue, RhiError};
use winit::window::Window;

// Import Windows-specific DirectX 12 types
use windows::{
    core::*,
    Win32::Graphics::Direct3D12::*,
    Win32::Graphics::Dxgi::*,
    Win32::Foundation::*,
    Win32::System::LibraryLoader::GetModuleHandleW,
};
use windows::Win32::Graphics::Direct3D::D3D12SerializeRootSignature;

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

// Initialize the DirectX 12 Agility SDK
pub fn init_dx12_agility_sdk() -> StdResult<(), RhiError> {
    // Attempt to initialize the Agility SDK
    unsafe {
        // Load the Agility SDK DLL if available
        let _result = GetModuleHandleW(w!("D3D12Core.dll"));
        // Note: In a real implementation, you would dynamically load the Agility SDK DLL
        // This is a placeholder implementation
    }
    
    Ok(())
}

pub struct Dx12Instance {
    factory: IDXGIFactory4,
}

impl Dx12Instance {
    pub fn new() -> StdResult<Self, RhiError> {
        init_dx12_agility_sdk()?;
        
        unsafe {
            let factory: IDXGIFactory4 = create_factory()?;
            Ok(Dx12Instance { factory })
        }
    }

    pub fn new_with_window(&self, _window: &Window) -> StdResult<Self, RhiError> {
        // For now, just return a new instance
        Self::new()
    }
}

impl Instance for Dx12Instance {
    fn create_surface(&self, window: &winit::window::Window) -> StdResult<Arc<dyn Surface>, RhiError> {
        let dx12_surface = Dx12Surface::new(window)?;
        Ok(Arc::new(dx12_surface) as Arc<dyn Surface>)
    }

    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>> {
        let mut adapters = Vec::new();
        unsafe {
            let mut i = 0;
            loop {
                match self.factory.EnumAdapters1(i) {
                    Ok(adapter) => {
                        // Check if this is a D3D12-compatible adapter
                        let hr = D3D12CreateDevice(
                            &adapter,
                            D3D_FEATURE_LEVEL_11_0,
                            std::ptr::null_mut() as *mut Option<ID3D12Device>,
                        );

                        if hr.is_ok() {
                            if let Ok(dx12_adapter) = Dx12Adapter::new(adapter) {
                                adapters.push(Arc::new(dx12_adapter) as Arc<dyn Adapter>);
                            }
                        }
                    }
                    Err(_) => break, // No more adapters
                }
                i += 1;
            }
        }
        adapters
    }
}

unsafe fn create_factory() -> StdResult<IDXGIFactory4, RhiError> {
    // Create DXGI factory
    let mut factory4: Option<IDXGIFactory4> = None;
    
    // Try to create a debug factory first if we're in debug mode
    #[cfg(debug_assertions)]
    {
        let mut debug_controller: Option<ID3D12Debug> = None;
        let hr = D3D12GetDebugInterface(&mut debug_controller);
        if hr.is_ok() {
            if let Some(debug) = debug_controller {
                let _ = debug.EnableDebugLayer();
            }
        }
    }

    // Create the factory
    let hr = CreateDXGIFactory1(&mut factory4);
    if hr.is_err() {
        return Err(RhiError::InitializationFailed(format!("Failed to create DXGI factory: {}", hr.err().unwrap())));
    }
    
    if let Some(f) = factory4 {
        Ok(f)
    } else {
        Err(RhiError::InitializationFailed("Factory creation returned null".to_string()))
    }
}

pub struct Dx12Adapter {
    adapter: IDXGIAdapter1,
    properties: AdapterProperties,
}

impl Dx12Adapter {
    pub fn new(adapter: IDXGIAdapter1) -> StdResult<Self, RhiError> {
        let mut desc: DXGI_ADAPTER_DESC1 = unsafe { std::mem::zeroed() };
        unsafe {
            let hr = adapter.GetDesc1(&mut desc);
            hr.map_err(|e| 
                RhiError::InitializationFailed(format!("Failed to get adapter description: {}", e))
            )?;
        }

        let name = unsafe {
            std::ffi::CStr::from_ptr(desc.Description.as_ptr() as *const i8)
                .to_string_lossy()
                .to_string()
        };

        let properties = AdapterProperties {
            name,
            vendor_id: desc.VendorId as u32,
            device_id: desc.DeviceId as u32,
        };

        Ok(Dx12Adapter { adapter, properties })
    }
}

impl Adapter for Dx12Adapter {
    fn request_device(&self) -> StdResult<Arc<dyn Device>, RhiError> {
        unsafe {
            let mut device: Option<ID3D12Device> = None;
            let hr = D3D12CreateDevice(
                &self.adapter,
                D3D_FEATURE_LEVEL_11_0,
                &mut device,
            );
            
            if hr.is_err() {
                return Err(RhiError::DeviceCreationFailed("Failed to create D3D12 device".to_string()));
            }

            if let Some(d3d12_device) = device {
                let dx12_device = Dx12Device::new(&d3d12_device)?;
                Ok(Arc::new(dx12_device) as Arc<dyn Device>)
            } else {
                Err(RhiError::DeviceCreationFailed("D3D12 device creation returned null".to_string()))
            }
        }
    }

    fn get_properties(&self) -> AdapterProperties {
        self.properties.clone()
    }
}

pub struct Dx12Device {
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
}

impl Dx12Device {
    pub fn new(d3d12_device: &ID3D12Device) -> StdResult<Self, RhiError> {
        // Create command queue
        let queue = unsafe {
            let queue_desc = D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: 0, // D3D12_COMMAND_QUEUE_PRIORITY_NORMAL
                Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
                NodeMask: 0,
            };
            
            let queue: ID3D12CommandQueue = d3d12_device.CreateCommandQueue(&queue_desc)
                .map_err(|e| RhiError::DeviceCreationFailed(format!("Failed to create command queue: {}", e)))?;
            
            queue
        };
        
        Ok(Dx12Device { 
            device: d3d12_device.clone(),
            queue,
        })
    }
    
    pub fn get_device(&self) -> &ID3D12Device {
        &self.device
    }
    
    pub fn get_queue(&self) -> &ID3D12CommandQueue {
        &self.queue
    }
}

impl Device for Dx12Device {
    fn create_swapchain(&self, desc: &SwapchainDescriptor) -> StdResult<Arc<dyn Swapchain>, RhiError> {
        // Extract the surface as Dx12Surface to get the HWND
        let surface_any = desc.surface.as_any();
        let dx12_surface = surface_any.downcast_ref::<Dx12Surface>()
            .ok_or_else(|| RhiError::SwapchainCreationFailed("Invalid surface type for DX12".to_string()))?;
            
        let dx12_swapchain = Dx12Swapchain::new(self, dx12_surface, desc)?;
        Ok(Arc::new(dx12_swapchain) as Arc<dyn Swapchain>)
    }

    fn create_shader_module(&self, desc: &ShaderModuleDescriptor) -> StdResult<Arc<dyn ShaderModule>, RhiError> {
        let dx12_shader = Dx12ShaderModule::new(desc)?;
        Ok(Arc::new(dx12_shader) as Arc<dyn ShaderModule>)
    }

    fn create_pipeline(&self, desc: &GraphicsPipelineDescriptor) -> StdResult<Arc<dyn Pipeline>, RhiError> {
        let dx12_pipeline = Dx12Pipeline::new(self, desc)?;
        Ok(Arc::new(dx12_pipeline) as Arc<dyn Pipeline>)
    }

    fn create_buffer(&self, desc: &BufferDescriptor) -> StdResult<Arc<dyn Buffer>, RhiError> {
        let dx12_buffer = Dx12Buffer::new(self, desc)?;
        Ok(Arc::new(dx12_buffer) as Arc<dyn Buffer>)
    }

    fn create_command_pool(&self, _swapchain: &Arc<dyn Swapchain>) -> StdResult<Arc<dyn CommandPool>, RhiError> {
        let dx12_command_pool = Dx12CommandPool::new(self)?;
        Ok(Arc::new(dx12_command_pool) as Arc<dyn CommandPool>)
    }

    fn get_queue(&self) -> Arc<dyn Queue> {
        Arc::new(Dx12Queue::new(&self.queue))
    }
}

use std::ptr;

pub struct Dx12Surface {
    window: *mut std::ffi::c_void,  // HWND
}

impl Dx12Surface {
    pub fn new(window: &winit::window::Window) -> StdResult<Self, RhiError> {
        // Get the HWND from the winit window
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        
        let window_handle = window.window_handle()
            .map_err(|e| RhiError::InitializationFailed(format!("Failed to get window handle: {}", e)))?;
        
        match window_handle.as_raw() {
            #[cfg(windows)]
            raw_window_handle::RawWindowHandle::Win32(handle) => {
                Ok(Dx12Surface {
                    window: handle.hwnd.get() as *mut std::ffi::c_void,
                })
            }
            _ => Err(RhiError::InitializationFailed("Unsupported window handle type".to_string())),
        }
    }
}

impl Surface for Dx12Surface {
    fn get_capabilities(&self, _adapter: &dyn Adapter) -> SurfaceCapabilities {
        SurfaceCapabilities {
            formats: vec![Format::B8G8R8A8Unorm, Format::R8G8B8A8Unorm],
            present_modes: vec![PresentMode::Fifo, PresentMode::Immediate], // Basic support
            min_image_count: 2,
            max_image_count: 3,
        }
    }
}

pub struct Dx12Swapchain {
    swapchain: IDXGISwapChain3,
    current_back_buffer_index: u32,
    format: Format,
    extent: Extent2D,
}

impl Dx12Swapchain {
    pub fn new(device: &Dx12Device, surface: &Dx12Surface, desc: &SwapchainDescriptor) -> StdResult<Self, RhiError> {
        unsafe {
            let swapchain = create_dxgi_swapchain(device, surface, desc)?;
            
            let dx12_swapchain = Dx12Swapchain {
                swapchain,
                current_back_buffer_index: 0,
                format: desc.format,
                extent: desc.extent,
            };
            
            Ok(dx12_swapchain)
        }
    }
}

unsafe fn create_dxgi_swapchain(
    device: &Dx12Device,
    surface: &Dx12Surface,
    desc: &SwapchainDescriptor,
) -> StdResult<IDXGISwapChain3, RhiError> {
    // Convert format to DXGI format
    let dxgi_format = match desc.format {
        Format::B8G8R8A8Unorm | Format::B8G8R8A8Srgb => DXGI_FORMAT_B8G8R8A8_UNORM,
        Format::R8G8B8A8Unorm | Format::R8G8B8A8Srgb => DXGI_FORMAT_R8G8B8A8_UNORM,
    };

    // Create swap chain descriptor
    let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
        BufferCount: desc.image_count,
        Width: desc.extent.width,
        Height: desc.extent.height,
        Format: dxgi_format,
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
        Scaling: DXGI_SCALING_STRETCH,
        Stereo: false.into(),
    };

    // Get DXGI factory from the device
    let adapter = {
        let mut adapter: Option<IDXGIAdapter> = None;
        let hr = device.device.QueryInterface(&mut adapter);
        if hr.is_err() {
            return Err(RhiError::SwapchainCreationFailed("Failed to get adapter from device".to_string()));
        }
        adapter.unwrap()
    };
    
    let factory: IDXGIFactory4 = {
        let mut factory: Option<IDXGIFactory> = None;
        let hr = adapter.GetParent(&mut factory);
        if hr.is_err() {
            return Err(RhiError::SwapchainCreationFailed("Failed to get factory from adapter".to_string()));
        }
        
        factory.unwrap().cast::<IDXGIFactory4>()
            .map_err(|e| RhiError::SwapchainCreationFailed(format!("Failed to get IDXGIFactory4: {}", e)))?
    };

    // Create the swap chain
    let hwnd = HWND(surface.window as isize);
    let swapchain: IDXGISwapChain1 = factory.CreateSwapChainForHwnd(
        &device.queue,
        hwnd,
        &swapchain_desc,
        None, // Don't restrict fullscreen
        None, // Don't use restrict tearing
    ).map_err(|e| RhiError::SwapchainCreationFailed(format!("Failed to create swapchain: {}", e)))?;

    // Cast to IDXGISwapChain3 for more features
    let swapchain: IDXGISwapChain3 = swapchain.cast::<IDXGISwapChain3>()
        .map_err(|e| RhiError::SwapchainCreationFailed(format!("Failed to cast swapchain: {}", e)))?;

    Ok(swapchain)
}

impl Swapchain for Dx12Swapchain {
    fn acquire_next_image(&self) -> StdResult<SwapchainImage, RhiError> {
        let current_index = unsafe {
            self.swapchain.GetCurrentBackBufferIndex()
        };
        
        Ok(SwapchainImage {
            index: current_index,
            image_view: current_index as usize,
            wait_semaphore: 0,
            signal_semaphore: 0,
        })
    }

    fn present(&self, _image: SwapchainImage) -> StdResult<(), RhiError> {
        unsafe {
            // Present the swapchain
            let hr = self.swapchain.Present(1, 0); // 1 for sync interval, 0 for present flags
            if hr.is_err() {
                return Err(RhiError::PresentFailed("Failed to present swapchain".to_string()));
            }
        }
        Ok(())
    }

    fn get_format(&self) -> Format {
        self.format
    }

    fn get_extent(&self) -> Extent2D {
        self.extent
    }
}

use std::sync::Mutex;

pub struct Dx12ShaderModule {
    #[allow(dead_code)]
    bytecode: Vec<u8>,
    stage: ShaderStage,
}

impl Dx12ShaderModule {
    pub fn new(desc: &ShaderModuleDescriptor) -> StdResult<Self, RhiError> {
        // In a real implementation, we would validate the shader bytecode
        // For now, we'll just store the bytecode
        let bytecode = desc.code.to_vec();
        
        Ok(Dx12ShaderModule {
            bytecode,
            stage: desc.stage.clone(),
        })
    }
    
    pub fn get_bytecode(&self) -> &[u8] {
        &self.bytecode
    }
    
    pub fn get_stage(&self) -> &ShaderStage {
        &self.stage
    }
}

impl ShaderModule for Dx12ShaderModule {}

pub struct Dx12Pipeline {
    pipeline_state: ID3D12PipelineState,
    input_layout: Vec<D3D12_INPUT_ELEMENT_DESC>,
    #[allow(dead_code)]
    root_signature: ID3D12RootSignature,
}

impl Dx12Pipeline {
    pub fn new(device: &Dx12Device, desc: &GraphicsPipelineDescriptor) -> StdResult<Self, RhiError> {
        unsafe {
            // Get shader modules
            let vertex_shader_any = desc.vertex_shader.as_any();
            let vertex_shader = vertex_shader_any.downcast_ref::<Dx12ShaderModule>()
                .ok_or_else(|| RhiError::PipelineCreationFailed("Invalid vertex shader module type".to_string()))?;
                
            let fragment_shader_any = desc.fragment_shader.as_any();
            let fragment_shader = fragment_shader_any.downcast_ref::<Dx12ShaderModule>()
                .ok_or_else(|| RhiError::PipelineCreationFailed("Invalid fragment shader module type".to_string()))?;
            
            // Create root signature (simplified)
            let root_signature = create_default_root_signature(&device.device)?;
            
            // Convert vertex input layout
            let input_layout = convert_vertex_input_to_d3d12_layout(&desc.vertex_input)?;
            
            // Determine render target format
            let dxgi_format = match desc.render_pass_format {
                Format::B8G8R8A8Unorm | Format::B8G8R8A8Srgb => DXGI_FORMAT_B8G8R8A8_UNORM,
                Format::R8G8B8A8Unorm | Format::R8G8B8A8Srgb => DXGI_FORMAT_R8G8B8A8_UNORM,
            };
            
            // Create pipeline state object
            let pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
                pRootSignature: Some(root_signature.clone()),
                VS: D3D12_SHADER_BYTECODE {
                    pShaderBytecode: vertex_shader.get_bytecode().as_ptr() as *const std::ffi::c_void,
                    BytecodeLength: vertex_shader.get_bytecode().len(),
                },
                PS: D3D12_SHADER_BYTECODE {
                    pShaderBytecode: fragment_shader.get_bytecode().as_ptr() as *const std::ffi::c_void,
                    BytecodeLength: fragment_shader.get_bytecode().len(),
                },
                BlendState: D3D12_BLEND_DESC {
                    AlphaToCoverageEnable: false.into(),
                    IndependentBlendEnable: false.into(),
                    RenderTarget: [D3D12_RENDER_TARGET_BLEND_DESC {
                        BlendEnable: false.into(),
                        LogicOpEnable: false.into(),
                        SrcBlend: D3D12_BLEND_ONE,
                        DestBlend: D3D12_BLEND_ZERO,
                        BlendOp: D3D12_BLEND_OP_ADD,
                        SrcBlendAlpha: D3D12_BLEND_ONE,
                        DestBlendAlpha: D3D12_BLEND_ZERO,
                        BlendOpAlpha: D3D12_BLEND_OP_ADD,
                        LogicOp: D3D12_LOGIC_OP_NOOP,
                        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
                    }; 8],
                },
                SampleMask: u32::MAX,
                RasterizerState: D3D12_RASTERIZER_DESC {
                    FillMode: D3D12_FILL_MODE_SOLID,
                    CullMode: D3D12_CULL_MODE_BACK,
                    FrontCounterClockwise: false.into(),
                    DepthBias: D3D12_DEFAULT_DEPTH_BIAS,
                    DepthBiasClamp: D3D12_DEFAULT_DEPTH_BIAS_CLAMP,
                    SlopeScaledDepthBias: D3D12_DEFAULT_SLOPE_SCALED_DEPTH_BIAS,
                    DepthClipEnable: true.into(),
                    MultisampleEnable: false.into(),
                    AntialiasedLineEnable: false.into(),
                    ForcedSampleCount: 0,
                    ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
                },
                DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
                    DepthEnable: false.into(),
                    DepthWriteMask: D3D12_DEPTH_WRITE_MASK_ALL,
                    DepthFunc: D3D12_COMPARISON_FUNC_LESS,
                    StencilEnable: false.into(),
                    StencilReadMask: D3D12_DEFAULT_STENCIL_READ_MASK,
                    StencilWriteMask: D3D12_DEFAULT_STENCIL_WRITE_MASK,
                    FrontFace: D3D12_DEPTH_STENCILOP_DESC {
                        StencilFailOp: D3D12_STENCIL_OP_KEEP,
                        StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
                        StencilPassOp: D3D12_STENCIL_OP_KEEP,
                        StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
                    },
                    BackFace: D3D12_DEPTH_STENCILOP_DESC {
                        StencilFailOp: D3D12_STENCIL_OP_KEEP,
                        StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
                        StencilPassOp: D3D12_STENCIL_OP_KEEP,
                        StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
                    },
                },
                InputLayout: D3D12_INPUT_LAYOUT_DESC {
                    pInputElementDescs: input_layout.as_ptr(),
                    NumElements: input_layout.len() as u32,
                },
                IBStripCutValue: D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED,
                PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
                NumRenderTargets: 1,
                RTVFormats: [dxgi_format, DXGI_FORMAT_UNKNOWN, DXGI_FORMAT_UNKNOWN, DXGI_FORMAT_UNKNOWN, DXGI_FORMAT_UNKNOWN, DXGI_FORMAT_UNKNOWN, DXGI_FORMAT_UNKNOWN, DXGI_FORMAT_UNKNOWN],
                DSVFormat: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                NodeMask: 0,
                CachedPSO: D3D12_CACHED_PIPELINE_STATE {
                    pCachedBlob: std::ptr::null(),
                    CachedBlobSizeInBytes: 0,
                },
                Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
            };
            
            let pipeline_state = device.device.CreateGraphicsPipelineState(&pso_desc)
                .map_err(|e| RhiError::PipelineCreationFailed(format!("Failed to create graphics pipeline state: {}", e)))?;
            
            Ok(Dx12Pipeline {
                pipeline_state,
                input_layout,
                root_signature,
            })
        }
    }
}

unsafe fn create_default_root_signature(device: &ID3D12Device) -> StdResult<ID3D12RootSignature, RhiError> {
    // Create a simple root signature with no parameters (for now)
    let root_signature_desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 0,
        pParameters: std::ptr::null(),
        NumStaticSamplers: 0,
        pStaticSamplers: std::ptr::null(),
        Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
    };
    
    let mut signature_blob: Option<IUnknown> = None;
    let mut error_blob: Option<IUnknown> = None;
    
    let hr = D3D12SerializeRootSignature(
        &root_signature_desc,
        D3D_ROOT_SIGNATURE_VERSION_1_0,
        &mut signature_blob,
        &mut error_blob,
    );
    
    if hr.is_err() {
        return Err(RhiError::PipelineCreationFailed("Failed to serialize root signature".to_string()));
    }
    
    let signature_blob = signature_blob.ok_or_else(|| 
        RhiError::PipelineCreationFailed("Failed to create root signature blob".to_string())
    )?;
    
    let signature_data = std::slice::from_raw_parts(
        signature_blob.as_raw() as *const u8,
        signature_blob.GetSize() // Correct way to get the size
    );
    
    let root_signature = device.CreateRootSignature(0, signature_data)
        .map_err(|e| RhiError::PipelineCreationFailed(format!("Failed to create root signature: {}", e)))?;
    
    Ok(root_signature)
}

fn convert_vertex_input_to_d3d12_layout(vertex_input: &VertexInputDescriptor) -> StdResult<Vec<D3D12_INPUT_ELEMENT_DESC>, RhiError> {
    let mut elements = Vec::new();
    
    for attr in &vertex_input.attributes {
        let format = match attr.format {
            VertexFormat::Float32x2 => DXGI_FORMAT_R32G32_FLOAT,
            VertexFormat::Float32x3 => DXGI_FORMAT_R32G32B32_FLOAT,
            VertexFormat::Float32x4 => DXGI_FORMAT_R32G32B32A32_FLOAT,
        };
        
        elements.push(D3D12_INPUT_ELEMENT_DESC {
            SemanticName: "TEXCOORD\0".as_ptr() as *const i8, // This is a simplification
            SemanticIndex: attr.location,
            Format: format,
            InputSlot: attr.binding,
            AlignedByteOffset: attr.offset,
            InputSlotClass: if vertex_input.bindings.get(attr.binding as usize)
                .map_or(D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, |binding| {
                    match binding.input_rate {
                        VertexInputRate::Vertex => D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                        VertexInputRate::Instance => D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA,
                    }
                }) == D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA {
                D3D12_INPUT_CLASSIFICATION_PER_INSTANCE_DATA
            } else {
                D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA
            },
            InstanceDataStepRate: if vertex_input.bindings.get(attr.binding as usize)
                .map_or(VertexInputRate::Vertex, |binding| binding.input_rate) == VertexInputRate::Instance {
                1
            } else {
                0
            },
        });
    }
    
    Ok(elements)
}

impl Pipeline for Dx12Pipeline {}

pub struct Dx12Buffer {
    buffer: ID3D12Resource,
    size: u64,
    usage: BufferUsage,
    memory_location: MemoryLocation,
    mapped_ptr: Mutex<Option<*mut u8>>,
}

impl Dx12Buffer {
    pub fn new(device: &Dx12Device, desc: &BufferDescriptor) -> StdResult<Self, RhiError> {
        unsafe {
            let heap_properties = match desc.memory_location {
                MemoryLocation::GpuOnly => D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_DEFAULT,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: 0,
                    VisibleNodeMask: 0,
                },
                MemoryLocation::CpuToGpu | MemoryLocation::GpuToCpu => D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD, // For CPU to GPU transfer
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: 0,
                    VisibleNodeMask: 0,
                },
            };

            let resource_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Alignment: 0,
                Width: desc.size,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAG_NONE,
            };

            let mut buffer: Option<ID3D12Resource> = None;
            let hr = device.device.CreateCommittedResource(
                &heap_properties,
                D3D12_HEAP_FLAG_NONE,
                &resource_desc,
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &mut buffer,
            );

            if hr.is_err() {
                return Err(RhiError::BufferCreationFailed(format!("Failed to create buffer: {}", hr.err().unwrap())));
            }

            let buffer = buffer.ok_or_else(|| RhiError::BufferCreationFailed("Buffer creation returned null".to_string()))?;

            Ok(Dx12Buffer {
                buffer,
                size: desc.size,
                usage: desc.usage.clone(),
                memory_location: desc.memory_location.clone(),
                mapped_ptr: Mutex::new(None),
            })
        }
    }
    
    pub fn get_resource(&self) -> &ID3D12Resource {
        &self.buffer
    }
}

impl Buffer for Dx12Buffer {
    fn map(&self) -> StdResult<*mut u8, RhiError> {
        if self.memory_location == MemoryLocation::GpuOnly {
            return Err(RhiError::MapFailed("Cannot map GPU-only buffer".to_string()));
        }
        
        unsafe {
            let mut mapped_ptr = self.mapped_ptr.lock().unwrap();
            if mapped_ptr.is_none() {
                let range = D3D12_RANGE {
                    Begin: 0,
                    End: self.size as usize,
                };
                
                let ptr = self.buffer.Map(0, Some(&range))
                    .map_err(|e| RhiError::MapFailed(format!("Failed to map buffer: {}", e)))? as *mut u8;
                
                *mapped_ptr = Some(ptr);
            }
            
            Ok(mapped_ptr.unwrap())
        }
    }

    fn unmap(&self) {
        unsafe {
            let mut mapped_ptr = self.mapped_ptr.lock().unwrap();
            if let Some(ptr) = *mapped_ptr {
                let range = D3D12_RANGE {
                    Begin: 0,
                    End: self.size as usize,
                };
                
                self.buffer.Unmap(0, Some(&range));
                *mapped_ptr = None;
            }
        }
    }
}

use std::sync::RwLock;

pub struct Dx12CommandPool {
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
}

impl Dx12CommandPool {
    pub fn new(device: &Dx12Device) -> StdResult<Self, RhiError> {
        unsafe {
            let command_allocator = device.device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .map_err(|e| RhiError::CommandPoolCreationFailed(format!("Failed to create command allocator: {}", e)))?;
            
            Ok(Dx12CommandPool {
                device: device.device.clone(),
                queue: device.queue.clone(),
                command_allocator,
            })
        }
    }
}

impl CommandPool for Dx12CommandPool {
    fn allocate_command_buffer(&self) -> StdResult<Arc<dyn CommandBuffer>, RhiError> {
        let command_buffer = Dx12CommandBuffer::new(&self.device, &self.command_allocator)?;
        Ok(Arc::new(command_buffer) as Arc<dyn CommandBuffer>)
    }
}

pub struct Dx12CommandBuffer {
    command_list: RwLock<Option<ID3D12GraphicsCommandList>>,
    command_allocator: ID3D12CommandAllocator,
    device: ID3D12Device,
    is_recording: RwLock<bool>,
}

impl Dx12CommandBuffer {
    pub fn new(device: &ID3D12Device, command_allocator: &ID3D12CommandAllocator) -> StdResult<Self, RhiError> {
        unsafe {
            let command_list = device.CreateCommandList(
                0,                                    // node mask
                D3D12_COMMAND_LIST_TYPE_DIRECT,       // command list type
                command_allocator,                     // initial allocator
                None,                                  // initial pipeline state (none for now)
            ).map_err(|e| RhiError::CommandBufferAllocationFailed(format!("Failed to create command list: {}", e)))?;
            
            // Close the command list since new command lists are created in recording state
            command_list.Close().ok(); // It's okay if closing fails here
            
            Ok(Dx12CommandBuffer {
                command_list: RwLock::new(Some(command_list)),
                command_allocator: command_allocator.clone(),
                device: device.clone(),
                is_recording: RwLock::new(false),
            })
        }
    }
    
    pub fn get_command_list(&self) -> Result<std::sync::RwLockReadGuard<Option<ID3D12GraphicsCommandList>>, RhiError> {
        self.command_list.read().map_err(|_| RhiError::CommandBufferAllocationFailed("Failed to lock command list".to_string()))
    }
    
    pub fn get_command_list_mut(&self) -> Result<std::sync::RwLockWriteGuard<Option<ID3D12GraphicsCommandList>>, RhiError> {
        self.command_list.write().map_err(|_| RhiError::CommandBufferAllocationFailed("Failed to lock command list".to_string()))
    }
}

impl CommandBuffer for Dx12CommandBuffer {
    fn begin(&self) -> StdResult<(), RhiError> {
        let mut is_recording = self.is_recording.write().unwrap();
        if *is_recording {
            return Err(RhiError::CommandBufferAllocationFailed("Command buffer already recording".to_string()));
        }
        
        unsafe {
            let mut cmd_list_guard = self.command_list.write().unwrap();
            
            // Reset the command allocator
            cmd_list_guard.as_ref().unwrap().Reset(&self.command_allocator, None)
                .map_err(|e| RhiError::CommandBufferAllocationFailed(format!("Failed to reset command list: {}", e)))?;
        }
        
        *is_recording = true;
        Ok(())
    }

    fn end(&self) -> StdResult<(), RhiError> {
        let mut is_recording = self.is_recording.write().unwrap();
        if !*is_recording {
            return Err(RhiError::CommandBufferAllocationFailed("Command buffer not recording".to_string()));
        }
        
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            cmd_list_guard.as_ref().unwrap().Close()
                .map_err(|e| RhiError::CommandBufferAllocationFailed(format!("Failed to close command list: {}", e)))?;
        }
        
        *is_recording = false;
        Ok(())
    }

    fn begin_render_pass(&self, _desc: &RenderPassDescriptor, _image: &SwapchainImage) {
        // In DX12, render passes are handled differently than in Vulkan/Metal
        // We set up render targets here
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                // TODO: Actually set up render targets here
                // This would involve creating and binding RTVs based on the image
            }
        }
    }
    
    fn end_render_pass(&self) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                // In DX12, we typically don't need an explicit end_render_pass
                // but we can add any necessary barriers here
            }
        }
    }
    
    fn set_viewport(&self, width: f32, height: f32) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                let viewport = D3D12_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: width,
                    Height: height,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                };
                
                command_list.RSSetViewports(&[viewport]);
            }
        }
    }
    
    fn set_scissor(&self, width: u32, height: u32) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                let rect = D3D12_RECT {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                };
                
                command_list.RSSetScissorRects(&[rect]);
            }
        }
    }
    
    fn bind_pipeline(&self, pipeline: &dyn Pipeline) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                // Downcast the pipeline to Dx12Pipeline to get the actual D3D12 object
                let pipeline_any = pipeline.as_any();
                if let Some(dx12_pipeline) = pipeline_any.downcast_ref::<Dx12Pipeline>() {
                    command_list.SetPipelineState(&dx12_pipeline.pipeline_state);
                }
            }
        }
    }
    
    fn bind_vertex_buffer(&self, buffer: &dyn Buffer) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                // Downcast the buffer to Dx12Buffer to get the actual D3D12 resource
                let buffer_any = buffer.as_any();
                if let Some(dx12_buffer) = buffer_any.downcast_ref::<Dx12Buffer>() {
                    let gpu_virtual_address = dx12_buffer.buffer.GetGPUVirtualAddress();
                    
                    let vertex_buffer_view = D3D12_VERTEX_BUFFER_VIEW {
                        BufferLocation: gpu_virtual_address,
                        SizeInBytes: dx12_buffer.size as u32,
                        StrideInBytes: 0, // This would normally come from the vertex layout
                    };
                    
                    command_list.IASetVertexBuffers(0, &[vertex_buffer_view]);
                }
            }
        }
    }
    
    fn draw(&self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
        unsafe {
            let cmd_list_guard = self.command_list.read().unwrap();
            if let Some(ref command_list) = *cmd_list_guard {
                command_list.DrawInstanced(vertex_count, instance_count, first_vertex, first_instance);
            }
        }
    }
}

pub struct Dx12Queue {
    queue: ID3D12CommandQueue,
}

impl Dx12Queue {
    pub fn new(queue: &ID3D12CommandQueue) -> Self {
        Dx12Queue {
            queue: queue.clone(),
        }
    }
}

impl Queue for Dx12Queue {
    fn submit(&self, _command_buffers: &[Arc<dyn CommandBuffer>], _wait_semaphore: Option<u64>, _signal_semaphore: Option<u64>) -> StdResult<(), RhiError> {
        // For now, just return OK. In a real implementation, we would execute the command lists
        Ok(())
    }

    fn wait_idle(&self) -> StdResult<(), RhiError> {
        unsafe {
            // Create a fence to wait for the queue to be idle
            let fence: ID3D12Fence = self.queue.CreateFence(
                0,
                D3D12_FENCE_FLAG_NONE,
            ).map_err(|e| RhiError::SubmitFailed(format!("Failed to create fence: {}", e)))?;
            
            // Signal the fence
            self.queue.Signal(&fence, 1)
                .map_err(|e| RhiError::SubmitFailed(format!("Failed to signal fence: {}", e)))?;
            
            // Wait for the fence to reach the signaled value
            while fence.GetCompletedValue() < 1 {
                // Simple spin-wait - in a real implementation, use event-based waiting
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(())
    }
}