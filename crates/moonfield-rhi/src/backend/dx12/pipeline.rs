use crate::{types::*, Pipeline, GraphicsPipelineDescriptor, VertexInputDescriptor, VertexFormat, VertexInputRate, Format, RhiError};

// Import Windows-specific DirectX 12 types
use windows::{
    core::*,
    Win32::Graphics::Direct3D12::*,
    Win32::Graphics::Dxgi::*, 
    Win32::Foundation::*,
};

// Import tracing for logging
use tracing;

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

pub struct Dx12Pipeline {
    pub pipeline_state: ID3D12PipelineState,
    pub input_layout: Vec<D3D12_INPUT_ELEMENT_DESC>,
    #[allow(dead_code)]
    pub root_signature: ID3D12RootSignature,
}

impl Dx12Pipeline {
    pub fn new(device: &super::device::Dx12Device, desc: &GraphicsPipelineDescriptor) -> StdResult<Self, RhiError> {
        unsafe {
            // Get shader modules
            let vertex_shader_any = desc.vertex_shader.as_any();
            let vertex_shader = vertex_shader_any.downcast_ref::<super::shader_module::Dx12ShaderModule>()
                .ok_or_else(|| RhiError::PipelineCreationFailed("Invalid vertex shader module type".to_string()))?;
                
            let fragment_shader_any = desc.fragment_shader.as_any();
            let fragment_shader = fragment_shader_any.downcast_ref::<super::shader_module::Dx12ShaderModule>()
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
    use windows::Win32::Graphics::Direct3D::D3D12SerializeRootSignature;
    
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