use std::fmt::{Display, Formatter};
use std::str::FromStr;

use rand::prelude::SliceRandom;
use rand::thread_rng;
use thiserror::Error;

#[derive(Debug, Copy, Clone)]
pub enum DrawStrategy {
    Random,
    Rows { reversed: bool },
    Columns { reversed: bool },
}

impl DrawStrategy {
    pub fn draw_order(&self, size: (u16, u16)) -> Vec<(u16, u16)> {
        match self {
            DrawStrategy::Random => {
                let mut draw_order = generate_draw_order(size, &|x, y| (x, y));
                draw_order.shuffle(&mut thread_rng());
                draw_order
            }
            DrawStrategy::Rows { reversed } => {
                let mut draw_order = generate_draw_order((size.1, size.0), &|x, y| (y, x));
                if *reversed {
                    draw_order.reverse();
                }
                draw_order
            }
            DrawStrategy::Columns { reversed } => {
                let mut draw_order = generate_draw_order(size, &|x, y| (x, y));
                if *reversed {
                    draw_order.reverse();
                }
                draw_order
            }
        }
    }
}

impl Display for DrawStrategy {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            DrawStrategy::Random => "random",
            DrawStrategy::Rows { reversed: false } => "down",
            DrawStrategy::Rows { reversed: true } => "up",
            DrawStrategy::Columns { reversed: false } => "right",
            DrawStrategy::Columns { reversed: true } => "left",
        };

        f.write_str(s)
    }
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid draw strategy: {}", 0)]
    Invalid(String),
}

impl FromStr for DrawStrategy {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "random" => DrawStrategy::Random,
            "down" => DrawStrategy::Rows { reversed: false },
            "up" => DrawStrategy::Rows { reversed: true },
            "right" => DrawStrategy::Columns { reversed: false },
            "left" => DrawStrategy::Columns { reversed: true },
            s => return Err(ParseError::Invalid(s.into())),
        })
    }
}

pub fn generate_draw_order<M: Fn(u16, u16) -> (u16, u16)>(
    size: (u16, u16),
    mapping: &M,
) -> Vec<(u16, u16)> {
    (0..size.0)
        .flat_map(|x| (0..size.1).map(move |y| mapping(x, y)))
        .collect()
}
