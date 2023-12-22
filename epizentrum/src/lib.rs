extern crate core;

use std::fmt::Debug;
use std::rc::Rc;
use std::time::Duration;

use rummelplatz::io_uring::squeue::PushError;
use thiserror::Error;

mod breadth_flatten;
pub mod flut_op;

pub trait CommandBufferSource: Debug {
    fn command_buffer(&self, delta: Duration) -> Rc<[u8]>;
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
