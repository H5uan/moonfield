use std::collections::HashMap;
use std::rc::{Rc, Weak};

use moonfield_core::array_as_u8_slice;
use tracing::error;

use crate::backend;
use crate::buffer::{BufferAccessPattern, BufferKind, GPUBufferDescriptor};
use crate::geometry_buffer::TriangleDefinition;
use crate::metal_backend::buffer::MetalBuffer;
use crate::{
    buffer::GPUBuffer,
    error::GraphicsError,
    geometry_buffer::{
        GeometryBuffer, GeometryBufferDescriptor, IndexAttributeKind,
    },
    metal_backend::MetalGraphicsBackend,
};

pub struct MetalGeometryBuffer {
    backend: Weak<MetalGraphicsBackend>,
    vertex_buffers: Vec<Option<MetalBuffer>>,

    index_buffer: Option<MetalBuffer>,
    index_format: Option<IndexAttributeKind>,
    vertex_count: u32,
    index_count: u32,
}

impl MetalGeometryBuffer {
    pub fn vertex_buffers(&self) -> &Vec<Option<MetalBuffer>> {
        &self.vertex_buffers
    }

    pub fn index_buffer(&self) -> &Option<MetalBuffer> {
        &self.index_buffer
    }

    pub fn vertex_count(&self) -> u32 {
        self.vertex_count
    }

    pub fn index_count(&self) -> u32 {
        self.index_count
    }

    pub fn new(
        backend: &MetalGraphicsBackend, desc: GeometryBufferDescriptor,
    ) -> Result<Self, GraphicsError> {
        let GeometryBufferDescriptor { name, kind, buffers, element } = desc;

        let mut vertex_buffers = Vec::new();
        for (index, buffer_desc) in buffers.iter().enumerate() {
            let buffer_size = if let Some(data) = buffer_desc.data.bytes {
                data.len()
            } else {
                0
            };

            let metal_buffer = if buffer_size > 0 {
                Some(MetalBuffer::new(
                    backend,
                    GPUBufferDescriptor {
                        name: &format!("{}_vertex_{}", name, index),
                        size: buffer_size,
                        kind: BufferKind::Vertex,
                        access_pattern: BufferAccessPattern::Stream,
                    },
                )?)
            } else {
                None
            };
            // Write data to the buffer if available
            if let Some(buffer) = &metal_buffer {
                if let Some(data) = buffer_desc.data.bytes {
                    if let Err(e) = buffer.write_data(data) {
                        error!("Failed to write vertex buffer data: {}", e);
                    }
                }
            }
            
            vertex_buffers.push(metal_buffer);
        }

        let (index_buffer, index_format, index_count) = match &element {
            crate::geometry_buffer::ElementsDescriptor::Triangles(
                triangles,
            ) => {
                let index_count = triangles.len() * 3;
                let buffer_size = index_count * 4;

                let index_buffer = if !triangles.is_empty() {
                    let buffer = MetalBuffer::new(
                        backend,
                        GPUBufferDescriptor {
                            name: &format!("{}_index", name),
                            size: buffer_size,
                            kind: BufferKind::Index,
                            access_pattern: BufferAccessPattern::Stream,
                        },
                    )?;
                    
                    // Write triangle indices to the buffer
                    let indices: Vec<u32> = triangles
                        .iter()
                        .flat_map(|tri| tri.indices().iter().copied())
                        .collect();
                    let index_data = array_as_u8_slice(&indices);
                    if let Err(e) = buffer.write_data(index_data) {
                        error!("Failed to write triangle index data: {}", e);
                    }
                    
                    Some(buffer)
                } else {
                    None
                };

                (
                    index_buffer,
                    Some(IndexAttributeKind::Uint32),
                    index_count as u32,
                )
            }
            crate::geometry_buffer::ElementsDescriptor::Lines(lines) => {
                let index_count = lines.len() * 2;
                let buffer_size = index_count * 4;

                let index_buffer = if !lines.is_empty() {
                    Some(MetalBuffer::new(
                        backend,
                        GPUBufferDescriptor {
                            name: &format!("{}_index", name),
                            size: buffer_size,
                            kind: BufferKind::Index,
                            access_pattern: BufferAccessPattern::Stream,
                        },
                    )?)
                } else {
                    None
                };

                (
                    index_buffer,
                    Some(IndexAttributeKind::Uint32),
                    index_count as u32,
                )
            }
            crate::geometry_buffer::ElementsDescriptor::Points(points) => {
                let index_count = points.len();
                let buffer_size = index_count * 4;

                let index_buffer = if !points.is_empty() {
                    Some(MetalBuffer::new(
                        backend,
                        GPUBufferDescriptor {
                            name: &format!("{}_index", name),
                            size: buffer_size,
                            kind: BufferKind::Index,
                            access_pattern: BufferAccessPattern::Stream,
                        },
                    )?)
                } else {
                    None
                };

                (
                    index_buffer,
                    Some(IndexAttributeKind::Uint32),
                    index_count as u32,
                )
            }
        };

        let vertex_count = buffers
            .iter()
            .find_map(|buffer_desc| {
                buffer_desc.data.bytes.map(|bytes| {
                    if buffer_desc.data.element_size > 0 {
                        bytes.len() / buffer_desc.data.element_size
                    } else {
                        0
                    }
                })
            })
            .unwrap_or(0) as u32;

        Ok(Self {
            backend: backend.weak(),
            vertex_buffers,
            index_buffer,
            index_format,
            vertex_count,
            index_count,
        })
    }
}

impl GeometryBuffer for MetalGeometryBuffer {
    fn set_buffer_data(&self, buffer: usize, data: &[u8]) {
        if let Some(Some(vertex_buffer)) = self.vertex_buffers.get(buffer) {
            if let Err(e) = vertex_buffer.write_data(data) {
                error!("Failed to write buffer data: {}", e);
            }
        }
    }

    fn element_count(&self) -> usize {
        if self.index_count > 0 {
            self.index_count as usize
        } else {
            self.vertex_count as usize
        }
    }

    fn set_triangles(&self, triangles: &[TriangleDefinition]) {
        if let Some(index_buffer) = &self.index_buffer {
            let indices: Vec<u32> = triangles
                .iter()
                .flat_map(|tri| tri.indices().iter().copied())
                .collect();

            let index_data = array_as_u8_slice(&indices);
            if let Err(e) = index_buffer.write_data(index_data) {
                error!("Failed to write triangle data: {}", e);
            }
        }
    }
}
