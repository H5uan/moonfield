use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::backend;
use crate::buffer::{BufferAccessPattern, BufferKind, GPUBufferDescriptor};
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
                        access_pattern: BufferAccessPattern::GpuReadOnly,
                    },
                )?)
            } else {
                None
            };
            vertex_buffers.push(metal_buffer);
        }

        let (index_buffer, index_format, index_count) = match &element {
            crate::geometry_buffer::ElementsDescriptor::Triangles(
                triangles,
            ) => {
                let index_count = triangles.len() * 3;
                let buffer_size = index_count * 4;

                let index_buffer = if !triangles.is_empty() {
                    Some(MetalBuffer::new(
                        backend,
                        GPUBufferDescriptor {
                            name: &format!("{}_index", name),
                            size: buffer_size,
                            kind: BufferKind::Index,
                            access_pattern: BufferAccessPattern::GpuReadOnly,
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
                            access_pattern: BufferAccessPattern::GpuReadOnly,
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
                            access_pattern: BufferAccessPattern::GpuReadOnly,
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
