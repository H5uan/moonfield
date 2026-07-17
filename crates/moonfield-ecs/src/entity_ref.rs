use crate::entities::EntityMeta;

#[derive(Copy, Clone)]
pub struct EntityRef<'a>{
    meta: &'a EntityMeta,


}