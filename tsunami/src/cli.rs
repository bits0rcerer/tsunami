use std::fmt::{Display, Formatter};
use std::net::IpAddr;
use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};
use std::path::PathBuf;
use std::str::FromStr;

use clap::builder::{PossibleValue, Str};
use clap::{Parser, Subcommand, ValueEnum};

use epizentrum::draw_strategy::DrawStrategy;

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Addresses of the pixelflut server
    #[arg(short, long, num_args = 1.., value_delimiter = ',', env = "TSUNAMI_TARGETS")]
    pub target_hosts: Vec<String>,

    /// Local interfaces to bind on
    #[arg(short, long, num_args = 1.., value_delimiter = ',', env = "TSUNAMI_INTERFACES")]
    pub interfaces: Vec<IpAddr>,

    /// Connection limit
    #[arg(short = 'c', long, env = "TSUNAMI_CONNECTIONS")]
    pub max_connections: Option<NonZeroUsize>,

    /// Reconnect limit
    #[arg(short = 'r', long, env = "TSUNAMI_RECONNECTS")]
    pub reconnects: Option<NonZeroUsize>,

    /// Reconnect backoff limit in seconds
    #[arg(long, env = "TSUNAMI_RECONNECT_BACKOFF_LIMIT")]
    pub reconnect_backoff_limit: Option<NonZeroU64>,

    /// Specify explicit canvas size instead of asking the server
    /// (Example: 1280x720)
    #[arg(long = "canvas", env = "TSUNAMI_CANVAS_SIZE")]
    pub canvas_size: Option<CanvasSize>,

    /// Time offset for animations in seconds
    #[arg(long, default_value_t, env = "TSUNAMI_TIME_OFFSET")]
    pub time_offset: i64,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Copy, Clone)]
pub struct CanvasSize(pub NonZeroU16, pub NonZeroU16);

impl FromStr for CanvasSize {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split('x').collect::<Vec<_>>().as_slice() {
            [x, y] => Ok(Self(NonZeroU16::from_str(x)?, NonZeroU16::from_str(y)?)),
            _ => Err(eyre::eyre!("invalid canvas size: \"{s}\"")),
        }
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// List all GPUs
    Gpus,
    /// Flut media files (jpeg, png, apng, gif, ...)
    Media(Media),
}

#[derive(clap::Args, Debug, Clone)]
pub struct GpuPreference {
    #[arg(long, default_value_t)]
    pub gpu_mode: GpuMode,

    #[arg(long, default_value_t)]
    pub gpu_index: usize,
}

#[derive(Debug, Copy, Clone)]
pub enum GpuMode {
    None,
    Preferred,
    Required,
}

impl Display for GpuMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GpuMode::None => f.write_str("None"),
            GpuMode::Preferred => f.write_str("Preferred"),
            GpuMode::Required => f.write_str("Required"),
        }
    }
}

impl From<&GpuMode> for Str {
    fn from(value: &GpuMode) -> Self {
        match value {
            GpuMode::None => Str::from("None"),
            GpuMode::Preferred => Str::from("Preferred"),
            GpuMode::Required => Str::from("Required"),
        }
    }
}

impl ValueEnum for GpuMode {
    fn value_variants<'a>() -> &'a [Self] {
        &[GpuMode::None, GpuMode::Preferred, GpuMode::Required]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(PossibleValue::new(self))
    }
}

impl Default for GpuMode {
    fn default() -> Self {
        Self::Preferred
    }
}

#[derive(Debug, Copy, Clone)]
pub enum CachingStrategy {
    /// Do not cache command buffers
    None,
    /// Cache a command buffer for a frame and keep it forever. (Do you have enough RAM?)
    /// Build command buffer on demand.
    KeepAllLazy,
    /// Cache the command buffer only for the latest frame
    KeepLast,
}

impl From<&CachingStrategy> for Str {
    fn from(value: &CachingStrategy) -> Self {
        match value {
            CachingStrategy::None => Str::from("None"),
            CachingStrategy::KeepLast => Str::from("KeepLast"),
            CachingStrategy::KeepAllLazy => Str::from("KeepAllLazy"),
        }
    }
}

impl ValueEnum for CachingStrategy {
    fn value_variants<'a>() -> &'a [Self] {
        &[Self::None, Self::KeepLast, Self::KeepAllLazy]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(PossibleValue::new(self))
    }
}

impl Default for CachingStrategy {
    fn default() -> Self {
        Self::KeepLast
    }
}

impl Display for CachingStrategy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CachingStrategy::None => f.write_str("None"),
            CachingStrategy::KeepAllLazy => f.write_str("KeepAllLazy"),
            CachingStrategy::KeepLast => f.write_str("KeepLast"),
        }
    }
}

#[derive(clap::Args, Debug, Clone)]
pub struct Media {
    #[command(flatten)]
    pub gpu_preference: GpuPreference,

    #[arg(long = "caching", default_value_t)]
    pub caching_strategy: CachingStrategy,

    #[arg(num_args = 1.., value_parser = clap::value_parser ! (MediaDescription), help = r"Media objects to flut
    
MEDIA_OBJECTS: <MEDIA_OBJECT>[ <MEDIA_OBJECT>…]
MEDIA_OBJECT : <path to file>[:<OFFSET>[:<DRAW_STRATEGY>]]
OFFSET:        <x>:<y>  (default: 0:0)
DRAW_STRATEGY: random   (random pixel order, default)
               up       (draw pixels from bottom to top)
               down     (draw pixels from top to bottom)
               left     (draw pixels from right to left)
               right    (draw pixels from left to right)")]
    pub media_objects: Vec<MediaDescription>,
}

#[derive(Debug, Clone)]
pub struct MediaDescription {
    pub x: u16,
    pub y: u16,
    pub path: PathBuf,
    pub draw_strategy: DrawStrategy,
}

impl FromStr for MediaDescription {
    type Err = eyre::Error;

    fn from_str(s: &str) -> eyre::Result<Self> {
        let splits = s.split(':').collect::<Vec<_>>();

        match splits.as_slice() {
            [path] => Ok(Self {
                x: 0,
                y: 0,
                path: PathBuf::from(path),
                draw_strategy: DrawStrategy::Random,
            }),
            [path, x, y] => Ok(Self {
                x: u16::from_str(x)?,
                y: u16::from_str(y)?,
                path: PathBuf::from(path),
                draw_strategy: DrawStrategy::Random,
            }),
            [path, x, y, strategy] => Ok(Self {
                x: u16::from_str(x)?,
                y: u16::from_str(y)?,
                path: PathBuf::from(path),
                draw_strategy: DrawStrategy::from_str(strategy)?,
            }),
            _ => Err(eyre::eyre!("unable to parse media object: {s}")),
        }
    }
}
