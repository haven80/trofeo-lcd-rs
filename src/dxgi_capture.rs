//! Screen capture module (Desktop Duplication API via DXGI & Direct3D 11).
//!
//! Designed specifically to capture the desktop screen or a virtual monitor
//! (e.g. a Virtual Display Driver at 1920x462 resolution) with GPU
//! acceleration, minimal latency, and fast conversion to an RGB888
//! `Framebuffer`.

#[cfg(windows)]
pub mod windows_dxgi {
    use anyhow::{bail, Context, Result};
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1,
        IDXGIOutputDuplication, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT,
        DXGI_OUTDUPL_FRAME_INFO,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

    use crate::Framebuffer;

    /// Summary info for a detected display output (monitor).
    #[derive(Debug, Clone)]
    pub struct DisplayInfo {
        pub index: usize,
        pub adapter_name: String,
        pub device_name: String,
        pub width: u32,
        pub height: u32,
        pub is_attached: bool,
    }

    /// Result of calling `acquire_next_frame`.
    #[derive(Debug, PartialEq, Eq)]
    pub enum CaptureResult {
        /// A new frame was successfully captured and copied into the `Framebuffer`.
        NewFrame,
        /// No change in the display / timeout (screen is idle).
        Timeout,
        /// DXGI access was lost (display mode changed, UAC, sleep/wake); needs reinit.
        NeedsReinit,
    }

    /// Gets the list of all display monitors connected to the system.
    pub fn list_displays() -> Result<Vec<DisplayInfo>> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
        let mut list = Vec::new();
        let mut display_idx = 0;

        let mut adapter_idx = 0;
        while let Ok(adapter) = unsafe { factory.EnumAdapters1(adapter_idx) } {
            adapter_idx += 1;
            let desc = unsafe { adapter.GetDesc1()? };
            let adapter_name = String::from_utf16_lossy(
                &desc.Description[..desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len())]
            );

            let mut output_idx = 0;
            while let Ok(output) = unsafe { adapter.EnumOutputs(output_idx) } {
                output_idx += 1;
                let out_desc = unsafe { output.GetDesc()? };

                let device_name = String::from_utf16_lossy(
                    &out_desc.DeviceName[..out_desc.DeviceName.iter().position(|&c| c == 0).unwrap_or(out_desc.DeviceName.len())]
                );

                let rect = out_desc.DesktopCoordinates;
                let width = (rect.right - rect.left).unsigned_abs();
                let height = (rect.bottom - rect.top).unsigned_abs();

                list.push(DisplayInfo {
                    index: display_idx,
                    adapter_name: adapter_name.clone(),
                    device_name,
                    width,
                    height,
                    is_attached: out_desc.AttachedToDesktop.as_bool(),
                });
                display_idx += 1;
            }
        }

        Ok(list)
    }

    /// A DXGI Output Duplication screen-capture session.
    pub struct DxgiSession {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        duplication: IDXGIOutputDuplication,
        staging_texture: ID3D11Texture2D,
        src_width: u32,
        src_height: u32,
        target_display_index: usize,
    }

    impl DxgiSession {
        /// Create a new session. If `display_index` is `None`, it will
        /// automatically look for a monitor at 1920x462 resolution. If none
        /// is found, it picks monitor #1 (secondary) or #0 (primary).
        pub fn new(display_index: Option<usize>) -> Result<Self> {
            let displays = list_displays()?;
            if displays.is_empty() {
                bail!("No monitors detected on the system!");
            }

            let chosen_idx = match display_index {
                Some(idx) => {
                    if idx >= displays.len() {
                        bail!("Monitor index {} is invalid (only {} display(s) found)", idx, displays.len());
                    }
                    idx
                }
                None => {
                    // Look for a display at 1920x462 resolution (Trofeo Vision native)
                    if let Some(pos) = displays.iter().position(|d| (d.width == 1920 && d.height == 462) || (d.width == 462 && d.height == 1920)) {
                        pos
                    } else if displays.len() > 1 {
                        1 // fallback to the second monitor
                    } else {
                        0 // fallback to the primary monitor
                    }
                }
            };

            Self::init_display(chosen_idx)
        }

        fn init_display(target_idx: usize) -> Result<Self> {
            let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
            let mut current_idx = 0;
            let mut adapter_idx = 0;

            let mut matched: Option<(IDXGIAdapter1, IDXGIOutput1, u32, u32)> = None;

            'outer: while let Ok(adapter) = unsafe { factory.EnumAdapters1(adapter_idx) } {
                adapter_idx += 1;
                let mut output_idx = 0;
                while let Ok(output) = unsafe { adapter.EnumOutputs(output_idx) } {
                    output_idx += 1;
                    if current_idx == target_idx {
                        let out_desc = unsafe { output.GetDesc()? };

                        let rect = out_desc.DesktopCoordinates;
                        let width = (rect.right - rect.left).unsigned_abs();
                        let height = (rect.bottom - rect.top).unsigned_abs();
                        let output1: IDXGIOutput1 = output.cast()?;
                        matched = Some((adapter, output1, width, height));
                        break 'outer;
                    }
                    current_idx += 1;
                }
            }

            let (adapter, output1, src_width, src_height) = match matched {
                Some(m) => m,
                None => bail!("Monitor with index {} not found", target_idx),
            };

            // Initialize the D3D11 Device
            let feature_levels = [D3D_FEATURE_LEVEL_11_0];
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;

            let adapter_base: IDXGIAdapter = adapter.cast()?;
            unsafe {
                D3D11CreateDevice(
                    Some(&adapter_base),
                    D3D_DRIVER_TYPE_UNKNOWN,
                    None,
                    D3D11_CREATE_DEVICE_FLAG(0),
                    Some(&feature_levels),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
                .context("Failed to create D3D11 Device for DXGI Duplication")?;
            }

            let device = device.unwrap();
            let context = context.unwrap();

            // Duplicate the output
            let duplication = unsafe {
                output1.DuplicateOutput(&device)
                    .context("Failed to duplicate the DXGI output (DuplicateOutput). Make sure the monitor is active and the driver supports DXGI Duplication.")?
            };

            // Staging texture for the GPU -> CPU transfer
            let desc = D3D11_TEXTURE2D_DESC {
                Width: src_width,
                Height: src_height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };

            let mut staging_texture: Option<ID3D11Texture2D> = None;
            unsafe {
                device.CreateTexture2D(&desc, None, Some(&mut staging_texture))
                    .context("Failed to create D3D11 staging texture")?;
            }
            let staging_texture = staging_texture.unwrap();

            Ok(Self {
                _device: device,
                context,
                duplication,
                staging_texture,
                src_width,
                src_height,
                target_display_index: target_idx,
            })
        }

        pub fn src_resolution(&self) -> (u32, u32) {
            (self.src_width, self.src_height)
        }

        pub fn display_index(&self) -> usize {
            self.target_display_index
        }

        /// Try to capture the next frame with a `timeout_ms` deadline.
        /// If there's a new frame, fill the data into `fb` (converted/scaled to `fb`'s size).
        pub fn acquire_next_frame(
            &mut self,
            timeout_ms: u32,
            fb: &mut Framebuffer,
        ) -> Result<CaptureResult> {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;

            let hr = unsafe {
                self.duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
            };

            if let Err(e) = hr {
                let code = e.code().0 as u32;
                if code == DXGI_ERROR_WAIT_TIMEOUT.0 as u32 {
                    return Ok(CaptureResult::Timeout);
                }
                if code == DXGI_ERROR_ACCESS_LOST.0 as u32 {
                    return Ok(CaptureResult::NeedsReinit);
                }
                return Err(e.into());
            }

            // If there are no accumulated frame updates, treat it as an idle screen
            if frame_info.AccumulatedFrames == 0 {
                let _ = unsafe { self.duplication.ReleaseFrame() };
                return Ok(CaptureResult::Timeout);
            }

            let desktop_resource = match resource {
                Some(r) => r,
                None => {
                    let _ = unsafe { self.duplication.ReleaseFrame() };
                    return Ok(CaptureResult::Timeout);
                }
            };

            let texture: ID3D11Texture2D = desktop_resource.cast()?;

            // Copy the GPU texture to the CPU staging texture
            unsafe {
                self.context.CopyResource(&self.staging_texture, &texture);
                let _ = self.duplication.ReleaseFrame();
            }

            // Map the staging texture into CPU memory
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                self.context.Map(&self.staging_texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                    .context("Failed to map staging texture")?;
            }

            let row_pitch = mapped.RowPitch as usize;
            let p_data = mapped.pData as *const u8;

            let dst_w = fb.width();
            let dst_h = fb.height();
            let dst_pixels = fb.as_bytes_mut();

            if self.src_width == dst_w && self.src_height == dst_h {
                // Fast 1:1 path (exact 1920x462 resolution)
                for y in 0..dst_h as usize {
                    let src_row = unsafe { p_data.add(y * row_pitch) };
                    let dst_offset = y * (dst_w as usize * 3);
                    for x in 0..dst_w as usize {
                        let b = unsafe { *src_row.add(x * 4) };
                        let g = unsafe { *src_row.add(x * 4 + 1) };
                        let r = unsafe { *src_row.add(x * 4 + 2) };
                        dst_pixels[dst_offset + x * 3] = r;
                        dst_pixels[dst_offset + x * 3 + 1] = g;
                        dst_pixels[dst_offset + x * 3 + 2] = b;
                    }
                }
            } else {
                // Scaled path (nearest-neighbor) if the monitor resolution differs
                let x_ratio = (self.src_width as f32) / (dst_w as f32);
                let y_ratio = (self.src_height as f32) / (dst_h as f32);

                for dy in 0..dst_h as usize {
                    let sy = ((dy as f32 * y_ratio) as usize).min((self.src_height - 1) as usize);
                    let src_row = unsafe { p_data.add(sy * row_pitch) };
                    let dst_offset = dy * (dst_w as usize * 3);

                    for dx in 0..dst_w as usize {
                        let sx = ((dx as f32 * x_ratio) as usize).min((self.src_width - 1) as usize);
                        let b = unsafe { *src_row.add(sx * 4) };
                        let g = unsafe { *src_row.add(sx * 4 + 1) };
                        let r = unsafe { *src_row.add(sx * 4 + 2) };
                        dst_pixels[dst_offset + dx * 3] = r;
                        dst_pixels[dst_offset + dx * 3 + 1] = g;
                        dst_pixels[dst_offset + dx * 3 + 2] = b;
                    }
                }
            }

            unsafe {
                self.context.Unmap(&self.staging_texture, 0);
            }

            Ok(CaptureResult::NewFrame)
        }
    }
}

#[cfg(windows)]
pub use windows_dxgi::*;

#[cfg(not(windows))]
pub mod dummy {
    use anyhow::{bail, Result};
    use crate::Framebuffer;

    #[derive(Debug, Clone)]
    pub struct DisplayInfo {
        pub index: usize,
        pub adapter_name: String,
        pub device_name: String,
        pub width: u32,
        pub height: u32,
        pub is_attached: bool,
    }

    #[derive(Debug, PartialEq, Eq)]
    pub enum CaptureResult {
        NewFrame,
        Timeout,
        NeedsReinit,
    }

    pub fn list_displays() -> Result<Vec<DisplayInfo>> {
        bail!("DXGI Desktop Duplication is only available on Windows.")
    }

    pub struct DxgiSession;

    impl DxgiSession {
        pub fn new(_display_index: Option<usize>) -> Result<Self> {
            bail!("DXGI Desktop Duplication is only available on Windows.")
        }

        pub fn src_resolution(&self) -> (u32, u32) {
            (1920, 462)
        }

        pub fn display_index(&self) -> usize {
            0
        }

        pub fn acquire_next_frame(&mut self, _timeout_ms: u32, _fb: &mut Framebuffer) -> Result<CaptureResult> {
            bail!("DXGI Desktop Duplication is only available on Windows.")
        }
    }
}

#[cfg(not(windows))]
pub use dummy::*;
