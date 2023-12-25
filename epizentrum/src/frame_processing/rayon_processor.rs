use std::error::Error;

use rayon::prelude::*;

use crate::draw_strategy::DrawStrategy;
use crate::frame_processing::FrameProcessor;
use crate::frame_source::Frame;

#[derive(Debug)]
pub struct RayonProcessor {
    size: (u16, u16),
    draw_order: Box<[(u16, u16)]>,
    offset: (u16, u16),
    canvas_size: (u16, u16),
}

impl RayonProcessor {
    pub fn new(
        size: (u16, u16),
        offset: (u16, u16),
        canvas_size: (u16, u16),
        draw_strategy: DrawStrategy,
    ) -> Self {
        Self {
            draw_order: draw_strategy.draw_order(size).into(),
            size,
            offset,
            canvas_size,
        }
    }
}

impl FrameProcessor for RayonProcessor {
    fn process(&self, frame: &Frame) -> Result<Box<[u8]>, Box<dyn Error + Send + Sync + 'static>> {
        Ok(match frame {
            Frame::Rgba(buffer) => self
                .draw_order
                .into_par_iter()
                .filter_map(|(x, y)| {
                    if *x >= self.canvas_size.0 || *y >= self.canvas_size.1 {
                        return None;
                    }

                    let xx = x + self.offset.0;
                    let yy = y + self.offset.1;

                    Some(
                        match buffer[(*y as usize * self.size.0 as usize) + *x as usize] {
                            [r, g, b, 255] if r == g && g == b => format!("PX {xx} {yy} {r:02x}\n"),
                            [r, g, b, 255] => format!("PX {xx} {yy} {r:02x}{g:02x}{b:02x}\n"),
                            [r, g, b, a] => format!("PX {xx} {yy} {r:02x}{g:02x}{b:02x}{a:02x}\n"),
                        }
                        .into_bytes(),
                    )
                })
                .flatten()
                .collect(),
            Frame::Bgra(buffer) => self
                .draw_order
                .into_par_iter()
                .filter_map(|(x, y)| {
                    if *x >= self.canvas_size.0 || *y >= self.canvas_size.1 {
                        return None;
                    }

                    Some(
                        match buffer[(*y as usize * self.size.0 as usize) + *x as usize] {
                            [b, g, r, 255] if r == g && g == b => format!("PX {x} {y} {r:02x}"),
                            [b, g, r, 255] => format!("PX {x} {y} {r:02x}{g:02x}{b:02x}"),
                            [b, g, r, a] => format!("PX {x} {y} {r:02x}{g:02x}{b:02x}{a:02x}"),
                        }
                        .into_bytes(),
                    )
                })
                .flatten()
                .collect(),
        })
    }
}
