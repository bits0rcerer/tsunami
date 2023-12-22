use std::fmt::{Debug, Formatter};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::num::{NonZeroU32, NonZeroUsize};
use std::rc::Rc;
use std::str::FromStr;
use std::time::{Duration, Instant};

use tracing::Level;

use epizentrum::flut_op::FlutOp;
use epizentrum::{CommandBufferSource, ControlFlowError, SetupError, TeardownError};

const MAX_SIZE_RESPONSE_LENGTH: usize = 32;

fn get_size(stream: &mut TcpStream) -> eyre::Result<(u32, u32)> {
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
            let x = u32::from_str(splits.get(idx + 1).unwrap())?;
            let y = u32::from_str(splits.get(idx + 2).unwrap())?;
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
    fn command_buffer(&self, _delta: Duration) -> Rc<[u8]> {
        self.0.clone()
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
    let size = get_size(&mut socket)?;

    let ring = epizentrum::tsunami_ring::Ring::new_raw_ring(NonZeroU32::new(128).unwrap())?;
    let flut_op = FlutOp::new(
        [SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 1337)].as_slice(),
        [TestSource::new_boxed(size)].into(),
        Some(NonZeroUsize::new(1).unwrap()),
        Some(Duration::from_secs(10)),
        None,
        vec![socket],
        Instant::now(),
    );
    let mut ring = epizentrum::tsunami_ring::Ring::new(ring, None, flut_op);
    ring.run::<SetupError, ControlFlowError, TeardownError>()?;

    Ok(())
}
