#![feature(const_for)]
extern crate core;

use std::cmp::Ordering;
use std::fmt::Debug;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

use rummelplatz::io_uring::squeue::PushError;
use thiserror::Error;

use crate::frame_processing::FrameProcessor;
use crate::frame_source::{FrameSource, Timing};

mod breadth_flatten;
pub mod flut_op;

pub mod draw_strategy;
pub mod frame_processing;
pub mod frame_source;

pub trait CommandBufferSource: Debug {
    fn command_buffer(&self, delta: Duration) -> Timing<Rc<[u8]>>;
    fn cycle_time(&self) -> Duration;
}

#[derive(Debug)]
pub struct CompositeBufferSource<Src: FrameSource + Debug, Proc: FrameProcessor + Debug> {
    pub source: Src,
    pub processor: Proc,
}

impl<Src: FrameSource + Debug, Proc: FrameProcessor + Debug> CommandBufferSource
    for CompositeBufferSource<Src, Proc>
{
    fn command_buffer(&self, delta: Duration) -> Timing<Rc<[u8]>> {
        let Timing {
            frame,
            frame_time,
            time_left,
        } = self.source.frame(delta);

        Timing {
            frame: self.processor.process(frame).into(),
            frame_time,
            time_left,
        }
    }

    fn cycle_time(&self) -> Duration {
        self.source.cycle_time()
    }
}

#[derive(Debug)]
pub struct OnDemandCache<Src: CommandBufferSource> {
    cache: Mutex<Vec<((Duration, Duration), Rc<[u8]>)>>,
    src: Src,
}

impl<Src: CommandBufferSource> OnDemandCache<Src> {
    pub fn new(src: Src) -> Self {
        Self {
            src,
            cache: Default::default(),
        }
    }
}

impl<Src: CommandBufferSource> CommandBufferSource for OnDemandCache<Src> {
    fn command_buffer(&self, delta: Duration) -> Timing<Rc<[u8]>> {
        let delta = Duration::from_nanos((delta.as_nanos() % self.cycle_time().as_nanos()) as u64);

        let mut cache = self
            .cache
            .lock()
            .expect("unable to acquire a lock on cache");

        match cache.binary_search_by(|&((start, end), _)| {
            if delta < start {
                Ordering::Less
            } else if delta >= end {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }) {
            Ok(i) => {
                let ((start, end), frame) = &cache[i];

                Timing {
                    frame: frame.clone(),
                    frame_time: (*end - *start),
                    time_left: (*end - delta),
                }
            }
            Err(i) => {
                let Timing {
                    frame,
                    frame_time,
                    time_left,
                } = self.src.command_buffer(delta);

                let end = delta + time_left;
                let start = end - frame_time;
                cache.insert(i, ((start, end), frame.clone()));

                Timing {
                    frame,
                    frame_time,
                    time_left,
                }
            }
        }
    }

    fn cycle_time(&self) -> Duration {
        self.src.cycle_time()
    }
}

#[derive(Debug, Error)]
pub enum SetupError {
    #[error("unable to submit sqe to submission queue: {:?}", 0)]
    SqeSubmission(#[from] PushError),
}

#[derive(Debug, Error)]
pub enum TeardownError {}

#[derive(Debug, Error)]
pub enum ControlFlowWarn {}

#[derive(Debug, Error)]
pub enum ControlFlowError {
    #[error("unable to submit sqe to submission queue: {:?}", 0)]
    SqeSubmission(#[from] PushError),

    #[error("io error: {:?}", 0)]
    Io(#[from] std::io::Error),
}

rummelplatz::ring! {
    tsunami_ring,
    flut_op: super::flut_op::FlutOp
}
