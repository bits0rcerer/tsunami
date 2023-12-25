use std::fs::File;
use std::path::Path;
use std::ptr::slice_from_raw_parts_mut;
use std::time::Duration;

use image::{AnimationDecoder, Delay, Frame, Frames, ImageError, ImageFormat, ImageResult};
use rayon::iter::Either;
use thiserror::Error;

use crate::frame_source;
use crate::frame_source::{FrameSource, Timing};

#[derive(Debug, Error)]
pub enum MediaSourceError {
    #[error("io error: {}", 0)]
    Io(#[from] std::io::Error),
    #[error("image error: {}", 0)]
    Image(#[from] ImageError),
    #[error("unknown media format")]
    Format,
    #[error("unsupported media format: {}", 0)]
    UnsupportedFormat(ImageFormat),
    #[error("unable to determine size")]
    UnknownSize,
}

#[derive(Debug)]
pub struct MediaSource {
    size: (u16, u16),
    frames: Box<[(frame_source::Frame, Duration)]>,
    time: Duration,
}

impl MediaSource {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, MediaSourceError> {
        let path = path.as_ref();

        let format = {
            match image::io::Reader::open(path)?
                .with_guessed_format()?
                .format()
            {
                None => return Err(MediaSourceError::Format),
                Some(format) => format,
            }
        };

        let frames = match format {
            ImageFormat::Png => {
                let png_decoder = image::codecs::png::PngDecoder::new(File::open(path)?)?;
                if png_decoder.is_apng() {
                    Either::Left(png_decoder.apng().into_frames())
                } else {
                    Either::Right(Frame::from_parts(
                        image::open(path)?.into_rgba8(),
                        0,
                        0,
                        Delay::from_numer_denom_ms(u32::MAX, 1),
                    ))
                }
            }
            ImageFormat::Gif => {
                Either::Left(image::codecs::gif::GifDecoder::new(File::open(path)?)?.into_frames())
            }
            ImageFormat::WebP => Either::Left(
                image::codecs::webp::WebPDecoder::new(File::open(path)?)?.into_frames(),
            ),
            ImageFormat::Jpeg
            | ImageFormat::Pnm
            | ImageFormat::Tiff
            | ImageFormat::Tga
            | ImageFormat::Dds
            | ImageFormat::Bmp
            | ImageFormat::Ico
            | ImageFormat::Hdr
            | ImageFormat::OpenExr
            | ImageFormat::Farbfeld
            | ImageFormat::Avif
            | ImageFormat::Qoi => Either::Right(Frame::from_parts(
                image::open(path)?.into_rgba8(),
                0,
                0,
                Delay::from_numer_denom_ms(u32::MAX, 1),
            )),
            format => return Err(MediaSourceError::UnsupportedFormat(format)),
        };

        let mut size = None;
        let frames = match frames {
            Either::Left(frames) => Frames::new(Box::new(frames)),
            Either::Right(frame) => Frames::new(Box::new([Ok(frame)].into_iter())),
        }
        .map(|frame| {
            frame.map(|f| {
                size = Some((f.buffer().width() as u16, f.buffer().height() as u16));
                let delay = f.delay().into();

                let buffer = f.into_buffer().into_raw().into_boxed_slice();
                let len = buffer.len();
                let ptr = Box::into_raw(buffer) as *mut [u8; 4];
                let buffer = unsafe { Box::from_raw(slice_from_raw_parts_mut(ptr, len / 4)) };
                (frame_source::Frame::Rgba(buffer), delay)
            })
        })
        .collect::<ImageResult<Vec<_>>>()?
        .into_boxed_slice();

        let time = frames.iter().map(|(_, d)| d).sum();

        match size {
            None => Err(MediaSourceError::UnknownSize),
            Some(size) => Ok(Self { size, frames, time }),
        }
    }
}

impl FrameSource for MediaSource {
    fn size(&self) -> (u16, u16) {
        self.size
    }

    fn cycle_time(&self) -> Duration {
        self.time
    }

    fn frame(&self, delta: Duration) -> Timing<&frame_source::Frame> {
        let delta = Duration::from_nanos((delta.as_nanos() % self.time.as_nanos()) as u64);
        let mut accu = Duration::ZERO;
        for (frame, frame_time) in self.frames.iter() {
            accu += *frame_time;

            if accu >= delta {
                return Timing {
                    frame,
                    frame_time: *frame_time,
                    time_left: accu - delta,
                };
            }
        }

        unreachable!()
    }
}
