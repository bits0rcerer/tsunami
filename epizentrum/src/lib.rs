#![feature(const_for)]
extern crate core;

use std::cmp::Ordering;
use std::error::Error;
use std::fmt::Debug;
use std::ops::Add;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub use rummelplatz;
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
    fn command_buffer(
        &mut self,
        delta: Duration,
    ) -> Result<Timing<Rc<[u8]>>, Box<dyn Error + Send + Sync>>;
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
    fn command_buffer(
        &mut self,
        delta: Duration,
    ) -> Result<Timing<Rc<[u8]>>, Box<dyn Error + Send + Sync>> {
        let Timing {
            frame,
            frame_time,
            time_left,
        } = self.source.frame(delta);

        self.processor
            .process(frame)
            .map(|frame| Timing {
                frame: frame.into(),
                frame_time,
                time_left,
            })
            .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
    }

    fn cycle_time(&self) -> Duration {
        self.source.cycle_time()
    }
}

#[derive(Debug)]
pub struct ComputeOnceCache<Src: CommandBufferSource> {
    cache: Vec<((Duration, Duration), Rc<[u8]>)>,
    src: Src,
}

impl<Src: CommandBufferSource> ComputeOnceCache<Src> {
    pub fn new(src: Src) -> Self {
        Self {
            src,
            cache: Default::default(),
        }
    }
}

impl<Src: CommandBufferSource> CommandBufferSource for ComputeOnceCache<Src> {
    fn command_buffer(
        &mut self,
        delta: Duration,
    ) -> Result<Timing<Rc<[u8]>>, Box<dyn Error + Send + Sync>> {
        let delta = Duration::from_nanos((delta.as_nanos() % self.cycle_time().as_nanos()) as u64);

        match self.cache.binary_search_by(|&((start, end), _)| {
            if delta < start {
                Ordering::Less
            } else if delta >= end {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        }) {
            Ok(i) => {
                let ((start, end), frame) = &self.cache[i];

                Ok(Timing {
                    frame: frame.clone(),
                    frame_time: (*end - *start),
                    time_left: (*end - delta),
                })
            }
            Err(i) => self.src.command_buffer(delta).map(|command_buffer| {
                let Timing {
                    frame,
                    frame_time,
                    time_left,
                } = command_buffer;

                let end = delta + time_left;
                let start = end - frame_time;
                self.cache.insert(i, ((start, end), frame.clone()));

                Timing {
                    frame,
                    frame_time,
                    time_left,
                }
            }),
        }
    }

    fn cycle_time(&self) -> Duration {
        self.src.cycle_time()
    }
}

#[derive(Debug)]
pub struct SingleFrameCache<Src: CommandBufferSource> {
    cache: Option<(Instant, Duration, Rc<[u8]>)>,
    src: Src,
}

impl<Src: CommandBufferSource> SingleFrameCache<Src> {
    pub fn new(src: Src) -> Self {
        Self {
            src,
            cache: Default::default(),
        }
    }
}

impl<Src: CommandBufferSource> CommandBufferSource for SingleFrameCache<Src> {
    fn command_buffer(
        &mut self,
        delta: Duration,
    ) -> Result<Timing<Rc<[u8]>>, Box<dyn Error + Send + Sync>> {
        let delta = Duration::from_nanos((delta.as_nanos() % self.cycle_time().as_nanos()) as u64);
        let now = Instant::now();

        match &self.cache {
            Some((valid_until, frame_time, frame)) if now <= *valid_until => Ok(Timing {
                frame: frame.clone(),
                frame_time: *frame_time,
                time_left: *valid_until - now,
            }),
            _ => {
                let timing = self.src.command_buffer(delta)?;
                let Timing {
                    frame,
                    frame_time,
                    time_left,
                } = &timing;

                self.cache = Some((Instant::now().add(*time_left), *frame_time, frame.clone()));

                Ok(timing)
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

    #[error("error: {:?}", 0)]
    Any(#[from] Box<dyn Error + Send + Sync>),
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

    #[error("error: {:?}", 0)]
    Any(#[from] Box<dyn Error + Send + Sync>),
}

rummelplatz::ring! {
    tsunami_ring,
    flut_op: super::flut_op::FlutOp
}
