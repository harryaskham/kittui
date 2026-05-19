//! wgpu adapter / device / queue ownership.

/// Errors during GPU initialization. Treated as a soft-failure signal by
/// the facade: callers fall back to the CPU renderer on `Init`.
#[derive(Debug, thiserror::Error)]
pub enum GpuInitError {
    /// No usable adapter was found.
    #[error("no usable wgpu adapter")]
    NoAdapter,
    /// Device request failed.
    #[error("wgpu device request failed: {0}")]
    DeviceRequest(String),
}

/// Long-lived wgpu handles. Renderers borrow these for every pass.
pub struct GpuDevice {
    /// wgpu device.
    pub device: wgpu::Device,
    /// wgpu queue.
    pub queue: wgpu::Queue,
    /// Adapter info (for diagnostics + parity cache).
    pub adapter_info: wgpu::AdapterInfo,
}

impl GpuDevice {
    /// Construct a device. Tries `HighPerformance` then `LowPower` then
    /// falls back to a software adapter if one is available.
    pub fn new() -> Result<Self, GpuInitError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter: wgpu::Adapter = pollster::block_on(async {
            for power in [
                wgpu::PowerPreference::HighPerformance,
                wgpu::PowerPreference::LowPower,
                wgpu::PowerPreference::None,
            ] {
                if let Some(adapter) = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: power,
                        compatible_surface: None,
                        force_fallback_adapter: false,
                    })
                    .await
                {
                    return Some(adapter);
                }
            }
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::None,
                    compatible_surface: None,
                    force_fallback_adapter: true,
                })
                .await
        })
        .ok_or(GpuInitError::NoAdapter)?;

        let adapter_info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("kittui-gpu"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e: wgpu::RequestDeviceError| GpuInitError::DeviceRequest(e.to_string()))?;

        Ok(Self {
            device,
            queue,
            adapter_info,
        })
    }
}
