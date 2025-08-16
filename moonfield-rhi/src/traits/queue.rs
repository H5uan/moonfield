use crate::Api;

pub trait Queue {
    type A: Api;

    fn submit(&self) {}
    fn present(&self) {}
}
