//! wgpu adapter / device / queue ownership.

/// Power preference used when choosing a headless/offscreen adapter.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GpuPowerPreference {
    /// Prefer a discrete / high-throughput adapter, falling back if unavailable.
    HighPerformance,
    /// Prefer an integrated / low-power adapter, falling back if unavailable.
    LowPower,
    /// Let wgpu choose any compatible adapter.
    None,
}

impl From<GpuPowerPreference> for wgpu::PowerPreference {
    fn from(value: GpuPowerPreference) -> Self {
        match value {
            GpuPowerPreference::HighPerformance => wgpu::PowerPreference::HighPerformance,
            GpuPowerPreference::LowPower => wgpu::PowerPreference::LowPower,
            GpuPowerPreference::None => wgpu::PowerPreference::None,
        }
    }
}

/// Options for constructing a [`GpuDevice`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct GpuDeviceOptions {
    /// Preferred adapter class for the first probe attempt.
    pub power_preference: GpuPowerPreference,
    /// Force wgpu's fallback adapter path. Useful for deterministic headless CI.
    pub force_fallback_adapter: bool,
}

impl Default for GpuDeviceOptions {
    fn default() -> Self {
        Self {
            power_preference: GpuPowerPreference::HighPerformance,
            force_fallback_adapter: false,
        }
    }
}

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
    /// Construct a device with default options. Tries high-performance,
    /// low-power, unconstrained, then fallback adapters.
    pub fn new() -> Result<Self, GpuInitError> {
        Self::new_with_options(GpuDeviceOptions::default())
    }

    /// Construct a device with explicit adapter options. The renderer never
    /// creates a surface, so this path is suitable for headless/offscreen
    /// macOS and Linux processes as well as ordinary terminal hosts.
    pub fn new_with_options(options: GpuDeviceOptions) -> Result<Self, GpuInitError> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter: wgpu::Adapter = pollster::block_on(async {
            if options.force_fallback_adapter {
                return instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: options.power_preference.into(),
                        compatible_surface: None,
                        force_fallback_adapter: true,
                    })
                    .await;
            }

            let ordered = match options.power_preference {
                GpuPowerPreference::HighPerformance => [
                    wgpu::PowerPreference::HighPerformance,
                    wgpu::PowerPreference::LowPower,
                    wgpu::PowerPreference::None,
                ],
                GpuPowerPreference::LowPower => [
                    wgpu::PowerPreference::LowPower,
                    wgpu::PowerPreference::HighPerformance,
                    wgpu::PowerPreference::None,
                ],
                GpuPowerPreference::None => [
                    wgpu::PowerPreference::None,
                    wgpu::PowerPreference::HighPerformance,
                    wgpu::PowerPreference::LowPower,
                ],
            };
            for power in ordered {
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
