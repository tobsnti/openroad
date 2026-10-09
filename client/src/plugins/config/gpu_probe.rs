//! Reads the GPU the renderer will use, before the App exists, so the `auto`
//! quality preset can be resolved while the config is built.
//!
//! Bevy creates its wgpu instance and adapter inside `RenderPlugin`, long
//! after `config.yaml` is loaded, and the preset decides settings that are
//! read at plugin build (the requested wgpu features, among others). So the
//! probe asks wgpu for the adapter on a throwaway instance first. It uses the
//! same backends (`WGPU_BACKEND`) and the same high-performance preference
//! Bevy uses, then reads the adapter's type and capability class and drops
//! everything. It costs one adapter enumeration at startup and runs only for
//! `preset: auto`.

use std::env;

use super::preset::{GpuKind, GpuSummary};

/// The adapter Bevy is about to pick, summarized. `None` when wgpu finds
/// none, e.g. no driver at all.
pub fn probe() -> Option<GpuSummary> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all());
    let instance = wgpu::Instance::new(descriptor);
    let adapter =
        futures_lite::future::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference:
                wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance),
            ..Default::default()
        }))
        .ok()?;
    let info = adapter.get_info();
    let limits = adapter.limits();
    let downlevel = adapter.get_downlevel_capabilities();
    // `OPENROAD_GPU_BASELINE=gl33` emulates a DX10-class card on a current
    // one (main.rs::render_plugin); `auto` should then pick what that card
    // would get.
    let emulate_gl33 = env::var("OPENROAD_GPU_BASELINE").is_ok_and(|v| v == "gl33");
    Some(GpuSummary {
        name: info.name.clone(),
        backend: format!("{:?}", info.backend),
        kind: match info.device_type {
            wgpu::DeviceType::DiscreteGpu => GpuKind::Discrete,
            wgpu::DeviceType::IntegratedGpu => GpuKind::Integrated,
            wgpu::DeviceType::Cpu | wgpu::DeviceType::VirtualGpu => GpuKind::Software,
            wgpu::DeviceType::Other => GpuKind::Unknown,
        },
        compute: !emulate_gl33
            && downlevel
                .flags
                .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
            && limits.max_storage_buffers_per_shader_stage > 0,
        gl: info.backend == wgpu::Backend::Gl,
    })
}
