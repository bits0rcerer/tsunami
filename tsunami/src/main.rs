use std::error::Error;
use std::fmt::{Debug, Formatter};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::num::{NonZeroU32, NonZeroUsize};
use std::rc::Rc;
use std::str::FromStr;
use std::time::{Duration, Instant};

use tracing::Level;

use epizentrum::draw_strategy::DrawStrategy;
use epizentrum::flut_op::FlutOp;
use epizentrum::frame_processing::gpu_processor::GpuProcessor;
use epizentrum::frame_processing::rayon_processor::RayonProcessor;
use epizentrum::frame_source::media_source::MediaSource;
use epizentrum::frame_source::{FrameSource, Timing};
use epizentrum::{
    tsunami_ring, CommandBufferSource, CompositeBufferSource, ControlFlowError, SetupError,
    SingleFrameCache, TeardownError,
};

const MAX_SIZE_RESPONSE_LENGTH: usize = 32;

fn get_size(stream: &mut TcpStream) -> eyre::Result<(u16, u16)> {
    let buf = "SIZE\n".as_bytes().to_vec();
    stream.write_all(buf.as_slice())?;

    let mut buf = vec![0u8; 32];
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

struct TestSource(Rc<[u8]>);

impl TestSource {
    pub fn new_boxed(size: (u32, u32)) -> Box<dyn CommandBufferSource> {
        Box::new(Self::new(size))
    }

    pub fn new(size: (u32, u32)) -> Self {
        let command_buffer = (0..size.0)
            .flat_map(move |x| {
                (0..size.1).flat_map(move |y| {
                    let r = (x % 256) as u8;
                    let g = (y % 256) as u8;
                    let b = ((x + y) % 256) as u8;
                    format!("PX {x} {y} {r:02x}{g:02x}{b:02x}\n").into_bytes()
                })
            })
            .collect();

        Self(command_buffer)
    }
}

impl Debug for TestSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestSource")
    }
}

impl CommandBufferSource for TestSource {
    fn command_buffer(
        &mut self,
        delta: Duration,
    ) -> Result<Timing<Rc<[u8]>>, Box<dyn Error + Send + Sync>> {
        Ok(Timing {
            frame: self.0.clone(),
            frame_time: Duration::MAX,
            time_left: Duration::MAX - delta,
        })
    }

    fn cycle_time(&self) -> Duration {
        Duration::MAX
    }
}

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .compact()
        .with_line_number(true)
        .init();

    let mut socket =
        TcpStream::connect("127.0.0.1:1337".to_socket_addrs().unwrap().next().unwrap()).unwrap();
    let canvas_size = get_size(&mut socket)?;

    let source = MediaSource::new("assets/media/badapple.gif")?;
    let rayon_processor =
        RayonProcessor::new(source.size(), (100, 100), canvas_size, DrawStrategy::Random);
    let gpu_processor = GpuProcessor::new(
        1,
        source.size(),
        (100, 100),
        canvas_size,
        DrawStrategy::Random,
    )?;
    let cmd_src = SingleFrameCache::new(CompositeBufferSource {
        source,
        processor: gpu_processor,
    });

    let ring = tsunami_ring::Ring::new_raw_ring(NonZeroU32::new(128).unwrap())?;
    let flut_op = FlutOp::new(
        [SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 1337)].as_slice(),
        [Box::new(cmd_src) as Box<dyn CommandBufferSource>].into(),
        Some(NonZeroUsize::new(12).unwrap()),
        Some(Duration::from_secs(10)),
        None,
        vec![socket],
        Instant::now(),
    );
    let mut ring = tsunami_ring::Ring::new(ring, None, flut_op);
    ring.run::<SetupError, ControlFlowError, TeardownError>()?;

    Ok(())
}
