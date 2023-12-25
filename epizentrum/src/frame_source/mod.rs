use std::fmt::Debug;
use std::time::Duration;

pub mod media_source;

#[derive(Debug)]
pub struct Timing<F: Debug> {
    pub frame: F,
    pub frame_time: Duration,
    pub time_left: Duration,
}

#[derive(Debug)]
pub enum Frame {
    Rgba(Box<[[u8; 4]]>),
    Bgra(Box<[[u8; 4]]>),
}

pub trait FrameSource {
    fn size(&self) -> (u16, u16);

    fn cycle_time(&self) -> Duration;

    /// Returns a frame, its full frame time and and how long the frame should be displayed relativ to delta
    fn frame(&self, delta: Duration) -> Timing<&Frame>;
}
