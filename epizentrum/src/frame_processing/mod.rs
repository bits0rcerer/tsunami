use std::error::Error;
use std::fmt::Debug;

use crate::frame_source::Frame;

pub mod gpu_processor;
pub mod rayon_processor;

pub trait FrameProcessor: Debug {
    fn process(&self, frame: &Frame) -> Result<Box<[u8]>, Box<dyn Error + Send + Sync>>;
}

impl<F: FrameProcessor + ?Sized> FrameProcessor for Box<F> {
    #[inline]
    fn process(&self, frame: &Frame) -> Result<Box<[u8]>, Box<dyn Error + Send + Sync>> {
        (**self).process(frame)
    }
}
