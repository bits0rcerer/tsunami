use crate::frame_source::Frame;

pub mod rayon_processor;

pub trait FrameProcessor {
    fn process(&self, frame: &Frame) -> Box<[u8]>;
}
