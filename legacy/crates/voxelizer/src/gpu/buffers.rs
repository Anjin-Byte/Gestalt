pub async fn map_buffer_u32(buffer: &wgpu::Buffer, device: &wgpu::Device) -> Vec<u32> {
    let slice = buffer.slice(..);
    let (sender, receiver) = futures::channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    receiver.await.expect("map buffer").expect("map buffer");
    let data = slice.get_mapped_range();
    let result = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    buffer.unmap();
    result
}

pub async fn map_buffer_f32(buffer: &wgpu::Buffer, device: &wgpu::Device) -> Vec<f32> {
    let slice = buffer.slice(..);
    let (sender, receiver) = futures::channel::oneshot::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    receiver.await.expect("map buffer").expect("map buffer");
    let data = slice.get_mapped_range();
    let result = bytemuck::cast_slice(&data).to_vec();
    drop(data);
    buffer.unmap();
    result
}
