use moonfield_core::allocator::Handle;

use crate::scene::node::{Node, NodePool};


#[derive(Debug)]
pub struct Graph{
    root: Handle<Node>,

    pool: NodePool,

    stack: Vec<Handle<Node>>,

    
}