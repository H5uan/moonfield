use crate::Api;

pub trait Surface {
    type A: Api;

    fn configure(&self, device: &<Self::A as Api>::Device);
    fn unconfigure(&self, device: &<Self::A as Api>::Device);

    fn acquire_texture(&self);
    fn discard_texture(&self);
}
