use std::fmt::Debug;
use std::ptr::slice_from_raw_parts;

use krnl::buffer::{Buffer, Slice};
use krnl::device::error::DeviceLost;
use krnl::device::Device;
use krnl::macros::module;
use thiserror::Error;
use tracing::error;

use crate::draw_strategy::DrawStrategy;
use crate::flut_op::DebugShield;
use crate::frame_processing::FrameProcessor;
use crate::frame_source::Frame;

const COMMAND_BUFFER_COLOR_LENGTH: usize = 8;

#[module]
mod kernels {
    // The device crate will be linked to krnl-core.
    #[cfg(not(target_arch = "spirv"))]
    use krnl::krnl_core;
    use krnl_core::macros::kernel;

    #[kernel]
    pub fn fill_rbga(
        #[global] color: Slice<u8>,
        #[global] command_buffer: UnsafeSlice<u8>,
        #[global] color_idx: Slice<u32>,
        #[global] digit_lookup: Slice<u8>,
    ) {
        use krnl_core::buffer::UnsafeIndex;

        let idx = kernel.global_id as usize;
        let color_idx = color_idx[idx] as usize;

        let [r, g, b, a] = [
            color[(4 * idx) + 0],
            color[(4 * idx) + 1],
            color[(4 * idx) + 2],
            color[(4 * idx) + 3],
        ];

        unsafe {
            *command_buffer.unsafe_index_mut(color_idx + 0) = digit_lookup[(r >> 4) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 1) = digit_lookup[(r & 0xf) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 2) = digit_lookup[(g >> 4) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 3) = digit_lookup[(g & 0xf) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 4) = digit_lookup[(b >> 4) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 5) = digit_lookup[(b & 0xf) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 6) = digit_lookup[(a >> 4) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 7) = digit_lookup[(a & 0xf) as usize];
        }
    }

    #[kernel]
    pub fn fill_bgra(
        #[global] color: Slice<u8>,
        #[global] command_buffer: UnsafeSlice<u8>,
        #[global] color_idx: Slice<u32>,
        #[global] digit_lookup: Slice<u8>,
    ) {
        use krnl_core::buffer::UnsafeIndex;

        let idx = kernel.global_id as usize;
        let color_idx = color_idx[idx] as usize;

        let [b, g, r, a] = [
            color[(4 * idx) + 0],
            color[(4 * idx) + 1],
            color[(4 * idx) + 2],
            color[(4 * idx) + 3],
        ];

        unsafe {
            *command_buffer.unsafe_index_mut(color_idx + 0) = digit_lookup[(r >> 4) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 1) = digit_lookup[(r & 0xf) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 2) = digit_lookup[(g >> 4) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 3) = digit_lookup[(g & 0xf) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 4) = digit_lookup[(b >> 4) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 5) = digit_lookup[(b & 0xf) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 6) = digit_lookup[(a >> 4) as usize];
            *command_buffer.unsafe_index_mut(color_idx + 7) = digit_lookup[(a & 0xf) as usize];
        }
    }
}

#[derive(Debug, Error)]
pub enum GpuProcessorError {
    #[error("gpu setup failed: {}", 0)]
    Setup(krnl::anyhow::Error),
    #[error("gpu memory upload failed: {}", 0)]
    Upload(krnl::anyhow::Error),
    #[error("gpu memory download failed: {}", 0)]
    Download(krnl::anyhow::Error),
    #[error("gpu memory allocation failed: {}", 0)]
    Alloc(krnl::anyhow::Error),
    #[error("gpu memory copy failed: {}", 0)]
    Copy(krnl::anyhow::Error),
    #[error("gpu kernel dispatch failed: {}", 0)]
    Dispatch(krnl::anyhow::Error),
    #[error("gpu synchronization failed: {}", 0)]
    Sync(#[from] DeviceLost),
}

#[derive(Debug)]
pub struct GpuProcessor {
    device: Device,
    template: Buffer<u8>,
    color_idx: Buffer<u32>,
    digit_lookup: Buffer<u8>,
    rgba_kernel: DebugShield<kernels::fill_rbga::Kernel>,
    bgra_kernel: DebugShield<kernels::fill_bgra::Kernel>,
}

impl GpuProcessor {
    pub fn list_devices() {
        for i in 0.. {
            match Device::builder()
                .index(i)
                .build()
                .map(|d| (d.info().cloned(), d))
            {
                Ok((Some(info), _)) => {
                    println!("Device {i}: {info:#?}");
                }
                Ok((None, _)) => {
                    error!("unable to get information about device {i} ",);
                }
                Err(_) => {
                    return;
                }
            }
        }
    }

    pub fn new(
        device_index: usize,
        size: (u16, u16),
        offset: (u16, u16),
        canvas_size: (u16, u16),
        draw_strategy: DrawStrategy,
    ) -> Result<Self, GpuProcessorError> {
        let device = Device::builder()
            .index(device_index)
            .build()
            .map_err(GpuProcessorError::Setup)?;

        // out of canvas pixel will be written to index 0, later we will ignore the first bytes
        let mut command_buffer_template: Vec<u8> = vec![0u8; COMMAND_BUFFER_COLOR_LENGTH];
        let mut color_idx: Vec<u32> = vec![0; size.0 as usize * size.1 as usize];

        let draw_order = draw_strategy.draw_order(size);
        for &(x, y) in &draw_order {
            if x + offset.0 >= canvas_size.0 || y + offset.1 >= canvas_size.1 {
                continue;
            }

            command_buffer_template
                .append(&mut format!("PX {} {} ", x + offset.0, y + offset.1).into_bytes());
            color_idx[(y as usize * size.0 as usize) + x as usize] =
                command_buffer_template.len() as u32;
            command_buffer_template.append(&mut vec![b'X'; COMMAND_BUFFER_COLOR_LENGTH]);
            command_buffer_template.push(b'\n');
        }

        let template = Buffer::from_vec(command_buffer_template)
            .into_device(device.clone())
            .map_err(GpuProcessorError::Upload)?;
        let color_idx = Buffer::from_vec(color_idx)
            .into_device(device.clone())
            .map_err(GpuProcessorError::Upload)?;
        let digit_lookup = Buffer::from_vec(digit_lookup())
            .into_device(device.clone())
            .map_err(GpuProcessorError::Upload)?;

        let rgba_kernel = kernels::fill_rbga::builder()
            .and_then(|b| b.build(device.clone()))
            .map_err(GpuProcessorError::Setup)?
            .with_global_threads(draw_order.len() as u32)
            .into();

        let bgra_kernel = kernels::fill_bgra::builder()
            .and_then(|b| b.build(device.clone()))
            .map_err(GpuProcessorError::Setup)?
            .with_global_threads(draw_order.len() as u32)
            .into();

        Ok(Self {
            device,
            template,
            color_idx,
            digit_lookup,
            rgba_kernel,
            bgra_kernel,
        })
    }
}

impl FrameProcessor for GpuProcessor {
    type Error = GpuProcessorError;

    fn process(&self, frame: &Frame) -> Result<Box<[u8]>, Self::Error> {
        let buffer = match frame {
            Frame::Rgba(buffer) => buffer.as_ref(),
            Frame::Bgra(buffer) => buffer.as_ref(),
        };

        let buffer = unsafe {
            &*slice_from_raw_parts(
                buffer.as_ptr() as *const u8,
                buffer.len() * std::mem::size_of_val(&buffer[0]),
            )
        };

        let buffer = Slice::from_host_slice(buffer)
            .into_device(self.device.clone())
            .map_err(GpuProcessorError::Upload)?;

        let mut command_buffer = unsafe {
            let mut buffer = Buffer::uninit(self.device.clone(), self.template.len())
                .map_err(GpuProcessorError::Alloc)?;
            buffer
                .copy_from_slice(&self.template.as_slice())
                .map_err(GpuProcessorError::Copy)?;
            buffer
        };

        match frame {
            Frame::Rgba(_) => self.rgba_kernel.get().dispatch(
                buffer.as_slice(),
                command_buffer.as_slice_mut(),
                self.color_idx.as_slice(),
                self.digit_lookup.as_slice(),
            ),
            Frame::Bgra(_) => self.bgra_kernel.get().dispatch(
                buffer.as_slice(),
                command_buffer.as_slice_mut(),
                self.color_idx.as_slice(),
                self.digit_lookup.as_slice(),
            ),
        }
        .map_err(GpuProcessorError::Dispatch)?;

        command_buffer
            .slice(COMMAND_BUFFER_COLOR_LENGTH..)
            .unwrap()
            .into_vec()
            .map(|v| v.into_boxed_slice())
            .map_err(GpuProcessorError::Download)
    }
}

fn digit_lookup() -> Vec<u8> {
    (0..=0xf)
        .map(|n| format!("{n:x}").bytes().next().unwrap())
        .collect()
}
