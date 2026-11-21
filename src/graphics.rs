use eframe::egui_wgpu::WgpuSetup;

pub fn setup() -> WgpuSetup {
    #[cfg(windows)]
    {
        choose(std::env::var_os("WGPU_BACKEND").is_some(), directx)
    }
    #[cfg(not(windows))]
    WgpuSetup::without_display_handle()
}

#[cfg(windows)]
fn choose(override_set: bool, fast: impl FnOnce() -> Option<WgpuSetup>) -> WgpuSetup {
    if !override_set && let Some(setup) = fast() {
        return setup;
    }
    WgpuSetup::without_display_handle()
}

#[cfg(windows)]
fn directx() -> Option<WgpuSetup> {
    use eframe::{
        egui_wgpu::{WgpuSetupCreateNew, WgpuSetupExisting},
        wgpu,
    };
    let mut options = WgpuSetupCreateNew::without_display_handle();
    options.instance_descriptor.backends = wgpu::Backends::DX12;
    // Initialize only the native Windows backend on the fast path. If either
    // adapter selection or device creation fails, retain eframe's full fallback.
    pollster::block_on(async {
        let instance = wgpu::Instance::new(options.instance_descriptor);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: options.power_preference,
                ..Default::default()
            })
            .await
            .ok()?;
        let (device, queue) = adapter
            .request_device(&(options.device_descriptor)(&adapter))
            .await
            .ok()?;
        Some(
            WgpuSetupExisting {
                instance,
                adapter,
                device,
                queue,
            }
            .into(),
        )
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn honors_backend_override_without_initializing_directx() {
        let setup = choose(true, || panic!("must honor the backend override"));
        assert!(matches!(setup, WgpuSetup::CreateNew(_)));
    }

    #[test]
    fn missing_directx_device_keeps_the_default_fallback() {
        let setup = choose(false, || None);
        assert!(matches!(setup, WgpuSetup::CreateNew(_)));
    }
}
