use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::num::NonZeroU32;
use std::ops::Add;
use std::str::FromStr;
use std::time::{Duration, Instant};

use clap::Parser;
use tracing::{debug, error, info, Level};
use tracing_subscriber::EnvFilter;

use epizentrum::flut_op::FlutOp;
use epizentrum::frame_processing::gpu_processor::GpuProcessor;
use epizentrum::frame_processing::rayon_processor::RayonProcessor;
use epizentrum::frame_processing::FrameProcessor;
use epizentrum::frame_source::media_source::MediaSource;
use epizentrum::frame_source::FrameSource;
use epizentrum::{
    tsunami_ring, CommandBufferSource, CompositeBufferSource, ComputeOnceCache, ControlFlowError,
    SetupError, SingleFrameCache, TeardownError,
};

use crate::cli::{CachingStrategy, CanvasSize, Commands, GpuMode};

mod cli;

const MAX_SIZE_RESPONSE_LENGTH: usize = 32;

fn get_size(stream: &mut TcpStream) -> eyre::Result<(u16, u16)> {
    let buf = "SIZE\n".as_bytes().to_vec();
    stream.write_all(buf.as_slice())?;

    let mut buf = vec![0u8; MAX_SIZE_RESPONSE_LENGTH];
    let mut res = String::new();

    loop {
        let bytes_read = stream.read(buf.as_mut_slice())?;
        match bytes_read {
            0 => return Err(eyre::eyre!("connection closed unexpectedly")),
            bytes => {
                buf[0..bytes]
                    .iter()
                    .map(|&b| char::from(b))
                    .for_each(|c| res.push(c));

                if res.contains('\n') {
                    res = res.replace('\n', "");
                    break;
                }
            }
        }

        if res.len() > MAX_SIZE_RESPONSE_LENGTH {
            return Err(eyre::eyre!("server response exceeded allowed length"));
        }
    }

    let splits = res.split(' ').collect::<Vec<_>>();
    let idx = splits
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| if s == "SIZE" { Some(i) } else { None })
        .next();

    match idx {
        None => Err(eyre::eyre!("invalid response")),
        Some(idx) if splits.len() < idx + 3 => Err(eyre::eyre!("invalid response")),
        Some(idx) => {
            let x = u16::from_str(splits.get(idx + 1).unwrap())?;
            let y = u16::from_str(splits.get(idx + 2).unwrap())?;
            Ok((x, y))
        }
    }
}

fn setup_logging() -> eyre::Result<()> {
    if cfg!(debug_assertions) {
        let filter = EnvFilter::builder()
            .with_default_directive(Level::DEBUG.into())
            .from_env_lossy();

        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .compact()
            //.with_file(true)
            .with_line_number(true)
            //.with_thread_names(true)
            .without_time()
            .finish();
        tracing::subscriber::set_global_default(subscriber)?;
    } else {
        let filter = EnvFilter::builder()
            .with_default_directive(Level::INFO.into())
            .from_env_lossy();

        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .compact()
            .with_target(false)
            .with_thread_names(true)
            .finish();
        tracing::subscriber::set_global_default(subscriber)?;
    }

    Ok(())
}

fn main() -> eyre::Result<()> {
    setup_logging()?;

    let args = cli::Args::parse();

    match &args.command {
        Commands::Gpus => GpuProcessor::list_devices(),
        Commands::Media(media) => {
            let targets = args
                .target_hosts
                .iter()
                .map(|host| {
                    if let Ok(iter) = host.to_socket_addrs() {
                        Ok(iter)
                    } else if let Ok(iter) = format!("{host}:1337").to_socket_addrs() {
                        Ok(iter)
                    } else if let Ok(iter) = format!("[{host}]:1337").to_socket_addrs() {
                        Ok(iter)
                    } else if let Ok(iter) = format!("{host}:1234").to_socket_addrs() {
                        Ok(iter)
                    } else if let Ok(iter) = format!("[{host}]:1234").to_socket_addrs() {
                        Ok(iter)
                    } else {
                        Err(eyre::eyre!("invalid host: {host}"))
                    }
                })
                .collect::<eyre::Result<Vec<_>>>()
                .map(|v| v.into_iter().flatten().collect::<Vec<_>>())?;

            let mut init_connection = None;
            let canvas_size = match &args.canvas_size {
                None => {
                    let (size, socket) = match targets
                        .iter()
                        .filter_map(|addr| {
                            match TcpStream::connect(addr)
                                .map_err(eyre::Error::from)
                                .and_then(|mut socket| Ok((get_size(&mut socket)?, socket)))
                            {
                                Ok((size, socket)) => {
                                    info!("Canvas size: {}x{}", size.0, size.1);
                                    Some((size, socket))
                                }
                                Err(e) => {
                                    debug!("unable to request canvas size via \"{addr}\": {e:?}");
                                    None
                                }
                            }
                        })
                        .next()
                    {
                        None => {
                            error!("unable to get canvas size");
                            return Err(eyre::eyre!("unable to get canvas size"));
                        }
                        Some(v) => v,
                    };

                    init_connection = Some(socket);
                    size
                }
                Some(CanvasSize(x, y)) => (x.get(), y.get()),
            };

            let sources = media
                .media_objects
                .iter()
                .map(|desc| {
                    let source = MediaSource::new(&desc.path)?;
                    let processor: Box<dyn FrameProcessor> = match media.gpu_preference.gpu_mode {
                        GpuMode::None => Box::new(RayonProcessor::new(
                            source.size(),
                            (desc.x, desc.y),
                            canvas_size,
                            desc.draw_strategy,
                        )),
                        GpuMode::Preferred | GpuMode::Required => {
                            let devices = GpuProcessor::devices();

                            let proc = devices
                                .iter()
                                .find_map(|(index, info)| {
                                    match GpuProcessor::new(
                                        *index,
                                        source.size(),
                                        (desc.x, desc.y),
                                        canvas_size,
                                        desc.draw_strategy,
                                    ) {
                                        Ok(proc) => {
                                            info!("using GPU {index}");
                                            if let Some(info) = info {
                                                debug!("GPU Info: {info:#?}");
                                            }
                                            Some(proc)
                                        }
                                        Err(e) => {
                                            debug!("unable to use GPU {index}: {e}");
                                            None
                                        }
                                    }
                                })
                                .map(|proc| Box::new(proc) as Box<dyn FrameProcessor>);

                            if matches!(media.gpu_preference.gpu_mode, GpuMode::Required)
                                && proc.is_none()
                            {
                                error!("no GPU available");
                                return Err(eyre::eyre!("no GPU available"));
                            }

                            proc.unwrap_or(Box::new(RayonProcessor::new(
                                source.size(),
                                (desc.x, desc.y),
                                canvas_size,
                                desc.draw_strategy,
                            )))
                        }
                    };

                    let pipeline = CompositeBufferSource { source, processor };

                    Ok(match media.caching_strategy {
                        CachingStrategy::None => Box::new(pipeline) as Box<dyn CommandBufferSource>,
                        CachingStrategy::KeepAllLazy => Box::new(ComputeOnceCache::new(pipeline)),
                        CachingStrategy::KeepLast => Box::new(SingleFrameCache::new(pipeline)),
                    })
                })
                .collect::<eyre::Result<Vec<_>>>()?;

            let ring = tsunami_ring::Ring::new_raw_ring(NonZeroU32::new(128).unwrap())?;
            let flut_op = FlutOp::new(
                targets.as_slice(),
                Some(args.interfaces.as_slice()),
                sources.into_boxed_slice(),
                args.max_connections,
                args.reconnect_backoff_limit
                    .map(|s| Duration::from_secs(s.get())),
                args.reconnects.map(|r| r.get()),
                match init_connection {
                    Some(c) => vec![c],
                    None => vec![],
                },
                Instant::now().add(match args.time_offset {
                    n if n > 0 => Duration::from_secs(n as u64),
                    n => Duration::from_secs(-n as u64),
                }),
            );
            let mut ring = tsunami_ring::Ring::new(ring, None, flut_op);
            ring.run::<SetupError, ControlFlowError, TeardownError>()?;
        }
    }

    Ok(())
}
