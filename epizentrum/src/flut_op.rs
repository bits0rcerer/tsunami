use std::cmp::min;
use std::fmt::{Debug, Formatter};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::num::NonZeroUsize;
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

use os_socketaddr::OsSocketAddr;
use rummelplatz::io_uring::opcode;
use rummelplatz::io_uring::squeue::{Entry, Flags};
use rummelplatz::io_uring::types::{Fd, Timespec};
use rummelplatz::{ControlFlow, RingOperation, SubmissionQueueSubmitter};
use socket2::{Domain, Protocol, Socket, Type};
use tracing::{debug, error, info, warn};

use crate::breadth_flatten::BreadthFlatten;
use crate::{CommandBufferSource, ControlFlowError, ControlFlowWarn, SetupError, TeardownError};

#[derive(Debug)]
pub struct FlutOp {
    reuse_connections: Vec<TcpStream>,

    targets: Box<[SocketAddr]>,
    connection_limit: Option<NonZeroUsize>,
    reconnect_backoff_limit: Option<Duration>,
    reconnect_limit: Option<usize>,
    connections: usize,

    time_anchor: Instant,
    // TODO: determine the impact of dynamic dispatch performance penalty
    command_buffer_sources: Box<[Box<dyn CommandBufferSource>]>,
}

impl Drop for FlutOp {
    fn drop(&mut self) {
        if self.connections > 0 {
            warn!("leaking {} connections", self.connections)
        }
    }
}

impl FlutOp {
    pub fn new(
        targets: &[SocketAddr],
        command_buffer_sources: Box<[Box<dyn CommandBufferSource>]>,
        connection_limit: Option<NonZeroUsize>,
        reconnect_backoff_limit: Option<Duration>,
        reconnect_limit: Option<usize>,
        reuse_connections: Vec<TcpStream>,
        time_anchor: Instant,
    ) -> Self {
        Self {
            reuse_connections,
            targets: targets.into(),
            connection_limit,
            reconnect_backoff_limit,
            reconnect_limit,
            connections: 0,
            time_anchor,
            command_buffer_sources,
        }
    }
}

pub struct DebugShield<T>(T);

impl<T> Debug for DebugShield<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("...")
    }
}

impl<T> From<T> for DebugShield<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

#[derive(Debug)]
pub enum FlutOpData {
    ConnectionEstablished {
        connection_id: usize,
        socket: Socket,
        addr: OsSocketAddr,
        source_index: usize,
        last_buffer: Option<(DebugShield<Rc<[u8]>>, usize)>,
    },
    Reconnecting {
        connection_id: usize,
        socket: Socket,
        addr: OsSocketAddr,
        source_index: usize,
        backoff: Duration,
        backoff_timespec: Timespec,
        reconnects: usize,
    },
    Backoff(Entry, Box<FlutOpData>),
}

impl RingOperation for FlutOp {
    type RingData = FlutOpData;
    type SetupError = SetupError;
    type TeardownError = TeardownError;
    type ControlFlowWarn = ControlFlowWarn;
    type ControlFlowError = ControlFlowError;

    fn setup<W: Fn(&mut Entry, Self::RingData)>(
        &mut self,
        mut submitter: SubmissionQueueSubmitter<Self::RingData, W>,
    ) -> Result<(), Self::SetupError> {
        let open_connections = self.reuse_connections.len();
        let connection_iters = self.targets.iter().map(|target| {
            (0..).map_while(move |i| match TcpStream::connect(target) {
                Ok(socket) => {
                    debug!("+ connection {} -> {target}", open_connections + i);
                    Some(socket)
                }
                Err(e) => {
                    debug!("unable to connect to {target}: {e:?}");
                    None
                }
            })
        });
        let connections = self
            .reuse_connections
            .drain(..)
            .map(|c| {
                debug!(
                    "reusing connection {} -> {}",
                    c.local_addr().unwrap(),
                    c.peer_addr().unwrap()
                );
                c
            })
            .chain(BreadthFlatten::new(connection_iters));
        let connections: Vec<_> = match self.connection_limit {
            None => connections.collect(),
            Some(limit) => connections.take(limit.get()).collect(),
        };
        info!("opened {} connections", connections.len());

        for (i, c) in connections.into_iter().enumerate() {
            let buffer = self.command_buffer_sources[i % self.command_buffer_sources.len()]
                .command_buffer(self.time_anchor.elapsed());
            let socket_write = opcode::Write::new(
                Fd(c.as_raw_fd()),
                buffer.frame.as_ptr(),
                buffer.frame.len() as u32,
            )
            .build();
            submitter.push(
                socket_write,
                FlutOpData::ConnectionEstablished {
                    connection_id: i,
                    addr: c.peer_addr().unwrap().into(),
                    socket: c.into(),
                    source_index: (i + 1) % self.command_buffer_sources.len(),
                    last_buffer: Some((buffer.frame.into(), 0)),
                },
            )?;
            self.connections += 1;
        }

        Ok(())
    }

    fn on_completion<W: Fn(&mut Entry, Self::RingData)>(
        &mut self,
        completion_entry: rummelplatz::io_uring::cqueue::Entry,
        ring_data: Self::RingData,
        mut submitter: SubmissionQueueSubmitter<Self::RingData, W>,
    ) -> (
        ControlFlow<Self::ControlFlowWarn, Self::ControlFlowError>,
        Option<Self::RingData>,
    ) {
        match ring_data {
            FlutOpData::ConnectionEstablished {
                connection_id,
                socket,
                addr,
                source_index,
                last_buffer,
            } => match (completion_entry.result(), last_buffer) {
                (e, _) if e <= 0 => {
                    if e == 0 {
                        warn!(
                            "connection {connection_id} {} -> {} closed",
                            socket.local_addr().unwrap().as_socket().unwrap(),
                            addr.into_addr().unwrap(),
                        );
                    } else {
                        let e = std::io::Error::from_raw_os_error(-e);
                        warn!("connection {connection_id} failed: {e}");
                    }

                    if let Some(limit) = self.reconnect_limit {
                        if limit == 0 {
                            error!("connection {connection_id} died");
                            self.connections -= 1;

                            if self.connections == 0 {
                                error!("all connections died, exiting..",);
                                return (ControlFlow::Exit, None);
                            }
                        }
                    }
                    let backoff = Duration::from_secs(1);
                    let backoff_timespec = Timespec::from(backoff);

                    info!(
                        "connection -> {} reconnecting in {} seconds",
                        addr.into_addr().unwrap(),
                        backoff.as_secs()
                    );

                    let backoff_timeout = opcode::Timeout::new(&backoff_timespec)
                        .build()
                        .flags(Flags::IO_LINK);

                    let socket = match addr.into_addr().unwrap().ip() {
                        IpAddr::V4(_) => {
                            Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
                        }
                        IpAddr::V6(_) => {
                            Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))
                        }
                    };

                    let socket = match socket {
                        Ok(socket) => socket,
                        Err(e) => {
                            error!("unable to create a new socket to reconnect connection {connection_id}: {e:?}");
                            return (ControlFlow::Error(ControlFlowError::Io(e)), None);
                        }
                    };

                    let connect =
                        opcode::Connect::new(Fd(socket.as_raw_fd()), addr.as_ptr(), addr.len())
                            .build();
                    match submitter.push(
                        backoff_timeout,
                        FlutOpData::Backoff(
                            connect,
                            Box::new(FlutOpData::Reconnecting {
                                connection_id,
                                backoff,
                                backoff_timespec,
                                addr,
                                socket,
                                source_index,
                                reconnects: 1,
                            }),
                        ),
                    ) {
                        Ok(()) => (ControlFlow::Continue, None),
                        Err(e) => (ControlFlow::Error(ControlFlowError::SqeSubmission(e)), None),
                    }
                }
                (n, Some((last_buffer, written)))
                    if written + n as usize == last_buffer.0.len() =>
                {
                    let buffer = self.command_buffer_sources[source_index]
                        .command_buffer(self.time_anchor.elapsed());
                    let socket_write = opcode::Write::new(
                        Fd(socket.as_raw_fd()),
                        buffer.frame.as_ptr(),
                        buffer.frame.len() as u32,
                    )
                    .build();
                    match submitter.push(
                        socket_write,
                        FlutOpData::ConnectionEstablished {
                            connection_id,
                            socket,
                            addr,
                            source_index: (source_index + 1) % self.command_buffer_sources.len(),
                            last_buffer: Some((buffer.frame.into(), 0)),
                        },
                    ) {
                        Ok(()) => (ControlFlow::Continue, None),
                        Err(e) => (ControlFlow::Error(ControlFlowError::SqeSubmission(e)), None),
                    }
                }
                (n, Some((last_buffer, written))) if n > 0 => {
                    let socket_write = opcode::Write::new(
                        Fd(socket.as_raw_fd()),
                        unsafe { last_buffer.0.as_ptr().add(written + n as usize) },
                        (last_buffer.0.len() - written) as u32 - n as u32,
                    )
                    .build();
                    match submitter.push(
                        socket_write,
                        FlutOpData::ConnectionEstablished {
                            connection_id,
                            socket,
                            addr,
                            source_index,
                            last_buffer: Some((last_buffer, written + n as usize)),
                        },
                    ) {
                        Ok(()) => (ControlFlow::Continue, None),
                        Err(e) => (ControlFlow::Error(ControlFlowError::SqeSubmission(e)), None),
                    }
                }
                _ => unreachable!(),
            },
            FlutOpData::Reconnecting {
                connection_id,
                socket,
                addr,
                source_index,
                backoff,
                reconnects,
                ..
            } => match completion_entry.result() {
                e if e < 0 => {
                    if e < 0 {
                        let e = std::io::Error::from_raw_os_error(-e);
                        debug!("connection {connection_id} reconnect failed: {e}");
                    }

                    if let Some(limit) = self.reconnect_limit {
                        if limit >= reconnects {
                            error!("connection {connection_id} died");
                            self.connections -= 1;

                            if self.connections == 0 {
                                error!("all connections died, exiting..",);
                                return (ControlFlow::Exit, None);
                            }
                        }
                    }

                    let backoff = match self.reconnect_backoff_limit {
                        None => backoff * 2,
                        Some(limit) => min(backoff * 2, limit),
                    };
                    let backoff_timespec = Timespec::from(backoff);

                    info!(
                        "connection {connection_id} reconnecting in {} seconds",
                        backoff.as_secs()
                    );

                    let backoff_timeout = opcode::Timeout::new(&backoff_timespec).build();

                    let connect =
                        opcode::Connect::new(Fd(socket.as_raw_fd()), addr.as_ptr(), addr.len())
                            .build();

                    match submitter.push(
                        backoff_timeout,
                        FlutOpData::Backoff(
                            connect,
                            Box::new(FlutOpData::Reconnecting {
                                connection_id,
                                backoff,
                                backoff_timespec,
                                addr,
                                socket,
                                source_index,
                                reconnects: reconnects + 1,
                            }),
                        ),
                    ) {
                        Ok(()) => (ControlFlow::Continue, None),
                        Err(e) => (ControlFlow::Error(ControlFlowError::SqeSubmission(e)), None),
                    }
                }
                0 => {
                    info!("connection {connection_id} reconnected");

                    let buffer = self.command_buffer_sources[source_index]
                        .command_buffer(self.time_anchor.elapsed());
                    let socket_write = opcode::Write::new(
                        Fd(socket.as_raw_fd()),
                        buffer.frame.as_ptr(),
                        buffer.frame.len() as u32,
                    )
                    .build();
                    match submitter.push(
                        socket_write,
                        FlutOpData::ConnectionEstablished {
                            connection_id,
                            addr,
                            socket,
                            source_index: (source_index + 1) % self.command_buffer_sources.len(),
                            last_buffer: Some((buffer.frame.into(), 0)),
                        },
                    ) {
                        Ok(()) => (ControlFlow::Continue, None),
                        Err(e) => (ControlFlow::Error(ControlFlowError::SqeSubmission(e)), None),
                    }
                }
                _ => unreachable!(),
            },
            FlutOpData::Backoff(entry, data) => match submitter.push(entry, *data) {
                Ok(()) => (ControlFlow::Continue, None),
                Err(e) => (ControlFlow::Error(ControlFlowError::SqeSubmission(e)), None),
            },
        }
    }

    fn on_teardown_completion<W: Fn(&mut Entry, Self::RingData)>(
        &mut self,
        _completion_entry: rummelplatz::io_uring::cqueue::Entry,
        ring_data: Self::RingData,
        _submitter: SubmissionQueueSubmitter<Self::RingData, W>,
    ) -> Result<(), Self::TeardownError> {
        match ring_data {
            FlutOpData::ConnectionEstablished { .. } => self.connections -= 1,
            FlutOpData::Reconnecting { .. } => self.connections -= 1,
            FlutOpData::Backoff(_, _) => self.connections -= 1,
        }

        Ok(())
    }
}
