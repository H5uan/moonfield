use std::fmt::Debug;

use moonfield_core::{
    allocator::{Pool, Slot},
    any_ext_for,
    math::geometry::AABB,
    type_traits::ComponentProvider,
};

pub trait BaseNodeTrait: Debug + Send {
    /// Create a raw copy of a node. This should not be called
    /// under normal circumstances
    fn clone_box(&self) -> Node;
}

any_ext_for!(BaseNodeTrait=>BaseNodeAsAny);

impl<T> BaseNodeTrait for T
where
    T: Clone + NodeTrait + 'static,
{
    fn clone_box(&self) -> Node {
        Node(Box::new(self.clone()))
    }
}

pub trait NodeTrait: BaseNodeTrait + ComponentProvider {
    fn local_bounding_box(&self) -> AABB;
    fn world_bounding_box(&self) -> AABB;
}

#[derive(Debug)]
pub struct Node(pub(crate) Box<dyn NodeTrait>);

impl<T: NodeTrait + 'static> From<T> for Node {
    fn from(value: T) -> Self {
        Self(Box::new(value))
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        self.0.clone_box()
    }
}

#[derive(Debug, Default)]
pub struct NodeSlot(Option<Node>);

impl Slot for NodeSlot {
    type Element = Node;

    fn new_empty() -> Self {
        Self(None)
    }

    fn new(element: Self::Element) -> Self {
        Self(Some(element))
    }

    fn is_some(&self) -> bool {
        self.0.is_some()
    }

    fn as_ref(&self) -> Option<&Self::Element> {
        self.0.as_ref()
    }

    fn as_mut(&mut self) -> Option<&mut Self::Element> {
        self.0.as_mut()
    }

    fn replace(&mut self, element: Self::Element) -> Option<Self::Element> {
        self.0.replace(element)
    }

    fn take(&mut self) -> Option<Self::Element> {
        self.0.take()
    }
}

pub type NodePool = Pool<Node, NodeSlot>;
