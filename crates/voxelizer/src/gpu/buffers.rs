//! Buffer-map readback helpers.
//!
//! Async so the same code runs on both targets: the map result arrives over a
//! oneshot future. On native the device is driven to completion with
//! `device.poll(PollType::wait_indefinitely())` before the await (which then
//! resolves immediately — callers `pollster::block_on` the public API). On
//! `wasm32` blocking is impossible and unnecessary: awaiting yields to the
//! browser event loop, which drives the device and fires the map callback.
//! Errors propagate as [`VoxelizeGpuError`] rather than panicking.

use crate::error::VoxelizeGpuError;

/// Maps `buffer` for read, awaits readiness, and returns its contents as `u32`s.
pub(crate) async fn map_buffer_u32(
    buffer: &wgpu::Buffer,
    device: &wgpu::Device,
) -> Result<Vec<u32>, VoxelizeGpuError> {
    let slice = buffer.slice(..);
    let (sender, receiver) = futures_channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    #[cfg(not(target_arch = "wasm32"))]
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|_| VoxelizeGpuError::Poll)?;
    #[cfg(target_arch = "wasm32")]
    let _ = device; // the browser event loop drives the device
    receiver.await.map_err(|_| VoxelizeGpuError::Poll)??;

    let data = slice.get_mapped_range();
    let result = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    buffer.unmap();
    Ok(result)
}

/// Maps `buffer` for read, awaits readiness, and returns its contents as `f32`s.
pub(crate) async fn map_buffer_f32(
    buffer: &wgpu::Buffer,
    device: &wgpu::Device,
) -> Result<Vec<f32>, VoxelizeGpuError> {
    let slice = buffer.slice(..);
    let (sender, receiver) = futures_channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    #[cfg(not(target_arch = "wasm32"))]
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|_| VoxelizeGpuError::Poll)?;
    #[cfg(target_arch = "wasm32")]
    let _ = device; // the browser event loop drives the device
    receiver.await.map_err(|_| VoxelizeGpuError::Poll)??;

    let data = slice.get_mapped_range();
    let result = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    buffer.unmap();
    Ok(result)
}
