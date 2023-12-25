use std::error::Error;

use crate::frame_source::Frame;

pub mod gpu_processor;
pub mod rayon_processor;

pub trait FrameProcessor {
    type Error: Error + Send + Sync + 'static;
    fn process(&self, frame: &Frame) -> Result<Box<[u8]>, Self::Error>;
}
