use std::{
    hash::{Hash, Hasher},
    mem::size_of,
    ops::{Index, IndexMut},
};

use bytemuck::{Pod, Zeroable};
use moonfield_core::{any_ext_for, array_as_u8_slice};

use crate::buffer::BufferKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VertexAttributeKind {
    Float32,   // f32
    Float32x2, // [f32; 2]
    Float32x3, // [f32; 3]
    Float32x4, // [f32; 4]
    Uint32,    // u32
    Uint32x2,  // [u32; 2]
    Uint32x3,  // [u32; 3]
    Uint32x4,  // [u32; 4]
    Int32,     // i32
    Int32x2,   // [i32; 2]
    Int32x3,   // [i32; 3]
    Int32x4,   // [i32; 4]
    Uint16x2,  // [u16; 2]
    Uint16x4,  // [u16; 4]
}

impl VertexAttributeKind {
    pub fn size_in_bytes(&self) -> u32 {
        match self {
            VertexAttributeKind::Float32 => 4,
            VertexAttributeKind::Float32x2 => 8,
            VertexAttributeKind::Float32x3 => 12,
            VertexAttributeKind::Float32x4 => 16,
            VertexAttributeKind::Uint32 => 4,
            VertexAttributeKind::Uint32x2 => 8,
            VertexAttributeKind::Uint32x3 => 12,
            VertexAttributeKind::Uint32x4 => 16,
            VertexAttributeKind::Int32 => 4,
            VertexAttributeKind::Int32x2 => 8,
            VertexAttributeKind::Int32x3 => 12,
            VertexAttributeKind::Int32x4 => 16,
            VertexAttributeKind::Uint16x2 => 4,
            VertexAttributeKind::Uint16x4 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexAttributeKind {
    Uint16,
    Uint32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VertexAttributeDefinition {
    pub location: u32,
    pub kind: VertexAttributeKind,
    pub component_count: usize,
    pub normalized: bool,
    pub divisor: u32,
}

/// Untyped vertex buffer data
#[derive(Debug)]
pub struct VertexBufferData<'a> {
    /// Vertex size
    pub element_size: usize,
    /// Vertex buffer data
    pub bytes: Option<&'a [u8]>,
}

impl<'a> VertexBufferData<'a> {
    pub fn new<T: Pod>(vertices: Option<&'a [T]>) -> Self {
        Self {
            element_size: size_of::<T>(),
            bytes: vertices.map(|v| array_as_u8_slice(v)),
        }
    }
}
#[derive(Debug)]
pub struct VertexBufferDescriptor<'a> {
    pub kind: BufferKind,
    pub attributes: &'a [VertexAttributeDefinition],
    pub data: VertexBufferData<'a>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash, Pod, Zeroable)]
#[repr(C)]
pub struct TriangleDefinition(pub [u32; 3]);

impl TriangleDefinition {
    #[inline]
    pub fn indices(&self) -> &[u32] {
        &self.0
    }

    #[inline]
    pub fn indices_mut(&mut self) -> &mut [u32] {
        &mut self.0
    }

    #[inline]
    pub fn edges(&self) -> [TriangleEdge; 3] {
        [
            TriangleEdge { a: self.0[0], b: self.0[1] },
            TriangleEdge { a: self.0[1], b: self.0[2] },
            TriangleEdge { a: self.0[2], b: self.0[0] },
        ]
    }

    #[inline]
    pub fn add(&self, i: u32) -> Self {
        Self([self.0[0] + i, self.0[1] + i, self.0[2] + i])
    }
}

impl AsRef<[u32]> for TriangleDefinition {
    fn as_ref(&self) -> &[u32] {
        &self.0
    }
}

impl AsMut<[u32]> for TriangleDefinition {
    fn as_mut(&mut self) -> &mut [u32] {
        &mut self.0
    }
}

impl Index<usize> for TriangleDefinition {
    type Output = u32;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl IndexMut<usize> for TriangleDefinition {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index]
    }
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct TriangleEdge {
    pub a: u32,
    pub b: u32,
}

impl PartialEq for TriangleEdge {
    fn eq(&self, other: &Self) -> bool {
        self.a == other.a && self.b == other.b
            || self.a == other.b && self.b == other.a
    }
}

impl Eq for TriangleEdge {}

impl Hash for TriangleEdge {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Direction-agnostic hash.
        (self.a as u64 + self.b as u64).hash(state)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ElementKind {
    /// Triangles.
    Triangle,
    /// Lines.
    Line,
    /// Points.
    Point,
}

impl ElementKind {
    pub fn index_per_element(self) -> usize {
        match self {
            ElementKind::Triangle => 3,
            ElementKind::Line => 2,
            ElementKind::Point => 1,
        }
    }
}

#[derive(Debug)]
pub enum ElementsDescriptor<'a> {
    Triangles(&'a [TriangleDefinition]),
    Lines(&'a [[u32; 2]]),
    Points(&'a [u32]),
}

impl ElementsDescriptor<'_> {
    pub fn element_kind(&self) -> ElementKind {
        match self {
            ElementsDescriptor::Triangles(_) => ElementKind::Triangle,
            ElementsDescriptor::Points(_) => ElementKind::Point,
            ElementsDescriptor::Lines(_) => ElementKind::Line,
        }
    }
}

pub struct GeometryBufferDescriptor<'a> {
    pub name: &'a str,
    pub kind: BufferKind,
    pub buffers: &'a [VertexBufferDescriptor<'a>],
    pub element: ElementsDescriptor<'a>,
}

any_ext_for!(GeometryBuffer => GeometryBufferAsAny);
pub trait GeometryBuffer {
    fn set_buffer_data(&self, buffer: usize, data: &[u8]);

    fn element_count(&self) -> usize;

    fn set_triangles(&self, triangles: &[TriangleDefinition]);
}

impl dyn GeometryBuffer {
    pub fn set_buffer_typed_data<T: Pod>(&self, buffer: usize, data: &[T]) {
        self.set_buffer_data(buffer, array_as_u8_slice(data));
    }
}
