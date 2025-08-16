use crate::Api;

pub trait CommandEncoder {
    type A: Api;

    fn begin_encoding();

    fn discard_encoding();

    fn end_encoding();

    fn reset_all();

    fn draw();

    fn draw_indexed();
}
