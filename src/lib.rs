//! Rust driver for the "LY" Thermalright USB LCD device family — including
//! the **Trofeo Vision 9.16 LCD** (VID:PID `0416:5408`) and the LY1 variant (`0416:5409`).
//!
//! Ported directly from the original Python implementation of the
//! `thermalright-trcc-linux` project (`src/trcc/adapters/.../ly_lcd.py`, class `LyLcd`),
//! including two non-obvious details that are intentionally kept so behavior
//! stays byte-for-byte identical to the version already validated on real
//! hardware (see the note about issue report #248 in the Python source):
//!
//! 1. **`_prepare_frame` always does `+1` chunk** — if `total_size` is exactly
//!    a multiple of 496, there will be one extra empty "terminator" chunk at the end.
//!    This is NOT a plain `ceil()`; don't "fix" it into `div_ceil`.
//! 2. **`_write_frame` advances `pos` by a fixed `USB_WRITE_SIZE` (4096)**,
//!    even though the last write is only 2048 bytes. This is safe because `prepare_frame`
//!    guarantees the total buffer is always a multiple of 2048 bytes (via padding to
//!    a multiple-of-4 chunk count), so the remainder before the last iteration is always exactly 0
//!    or 2048 — but this is not a "general" loop, don't reuse it for other sizes.
//!
//! # Important things to know about this panel
//! - **The frame payload is a JPEG image, not raw RGB565/RGB888.** This
//!   "jpeg=true" panel is limited to roughly 450,000 bytes per frame (the
//!   `max_frame_bytes` constant from the C# version, confirmed by real-hardware
//!   measurements in the Python source: ~360 KB works, ~570 KB fails). The `Framebuffer`
//!   in this module stores RGB888 pixels and has `to_jpeg()` to encode to JPEG
//!   before sending.
//! - The Trofeo Vision 9.16 LCD's resolution, per the comments in the original
//!   Python source, is **1920x462** — see the [`TROFEO_VISION_9_16`] constant.
//! - Image rotation: `Handshake::rotate_180` is now ALWAYS `false` (the previous
//!   automatic heuristic, based on the SUB byte, proved to rotate the wrong way on real
//!   hardware — see the git history). If your panel needs a 180° rotation, set it
//!   manually via `ROTATE_180_OVERRIDE` in `main.rs`, instead of relying on this field.
//! - The bulk OUT/IN endpoints are **not hardcoded** — they're auto-detected from
//!   the USB device descriptor at `open()` time (looking for an interface with a
//!   matching pair of bulk OUT+IN endpoints). This is intentional, because the endpoint
//!   addresses written in the original Python source (`0x01`/`0x81`) turned out **not to always match**
//!   real hardware — different Trofeo Vision 9.16 units/firmware can
//!   use different endpoint addresses (old decompiled protocol
//!   documentation even mentions EP09 OUT). If `open()` fails with
//!   `LcdError::NotFound`, use [`LyLcd::probe_endpoints`] to see
//!   every endpoint that actually exists on your device.

use rusb::{Context, DeviceHandle, UsbContext};
use std::time::Duration;
use thiserror::Error;

pub mod font;
pub mod smooth_font;
pub mod dxgi_capture;
pub mod background;
pub mod config;
pub mod conf_edit;
pub mod schema;
pub mod i18n;
pub mod layout;
pub mod hotkey;
pub mod png_save;

pub const VENDOR_ID: u16 = 0x0416;
/// Trofeo Vision 9.16 LCD.
pub const PID_LY: u16 = 0x5408;
pub const PID_LY1: u16 = 0x5409;

const HANDSHAKE_HEADER: [u8; 16] = [
    0x02, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const HANDSHAKE_READ_SIZE: usize = 512;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(1000);
const WRITE_TIMEOUT: Duration = Duration::from_millis(5000);
const READ_TIMEOUT: Duration = Duration::from_millis(1000);

const CHUNK_SIZE: usize = 512;
const CHUNK_HEADER_SIZE: usize = 16;
const CHUNK_DATA_SIZE: usize = 496;
const USB_WRITE_SIZE: usize = 4096;

/// Maximum JPEG size accepted by the firmware (C# TRCC 2.1.6 constant).
pub const MAX_FRAME_BYTES: usize = 450_000;

/// Default resolution of the Trofeo Vision 9.16 LCD, per the notes in the original Python source.
pub const TROFEO_VISION_9_16: Resolution = Resolution::new(1920, 462);

/// Trofeo Vision 11.3 LCD, as given on Thermalright's product page (1920x400).
/// Not verified on real hardware: if your unit needs another size, set it with
/// `panel_size = WIDTHxHEIGHT`.
pub const TROFEO_VISION_11_3: Resolution = Resolution::new(1920, 400);

/// A known panel model: it decides the size of the picture that is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Panel {
    /// Trofeo Vision 9.16 LCD, 1920x462 (default).
    #[default]
    Trofeo9_16,
    /// Trofeo Vision 11.3 LCD, 1920x400.
    Trofeo11_3,
}

impl Panel {
    pub const NAMES: &'static str = "trofeo-9.16 | trofeo-11.3";

    pub fn parse(s: &str) -> Option<Panel> {
        let t = s.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        let t = t.strip_prefix("trofeo-vision-").or_else(|| t.strip_prefix("trofeo-")).unwrap_or(&t);
        match t.trim_end_matches("-lcd").trim_end_matches('"').trim_end_matches("in") {
            "9.16" | "916" | "9.16-argb" => Some(Panel::Trofeo9_16),
            "11.3" | "113" => Some(Panel::Trofeo11_3),
            _ => None,
        }
    }

    pub const fn resolution(self) -> Resolution {
        match self {
            Panel::Trofeo9_16 => TROFEO_VISION_9_16,
            Panel::Trofeo11_3 => TROFEO_VISION_11_3,
        }
    }
}

/// Panel model recognised from the handshake (byte 20 of the response: 5 = Trofeo Vision
/// 11.3). The 11.3 shares USB 0416:5408 and the handshake with the 9.16, so this byte is the
/// only difference seen by other projects; it is NOT verified on hardware by this project.
pub fn panel_from_handshake(raw: &[u8]) -> Option<Panel> {
    (raw.get(20) == Some(&5)).then_some(Panel::Trofeo11_3)
}

static DETECTED_PANEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Remember the model found at the handshake (used by the config when `panel`/`panel_size`
/// are not set, also after a hot reload).
pub fn set_detected_panel(p: Option<Panel>) {
    DETECTED_PANEL.store(matches!(p, Some(Panel::Trofeo11_3)) as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn detected_panel() -> Option<Panel> {
    (DETECTED_PANEL.load(std::sync::atomic::Ordering::Relaxed) == 1).then_some(Panel::Trofeo11_3)
}

/// Parse a panel size such as `1920x400` (also `1920*400`, `1920 400`, `1920,400`).
pub fn parse_resolution(s: &str) -> Option<Resolution> {
    let t = s.trim().to_ascii_lowercase();
    let mut it = t.split(|c: char| c == 'x' || c == '*' || c == ',' || c == ' ' || c == '×').filter(|p| !p.is_empty());
    let w: u32 = it.next()?.trim().parse().ok()?;
    let h: u32 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || !(64..=8192).contains(&w) || !(64..=8192).contains(&h) {
        return None;
    }
    Some(Resolution::new(w, h))
}

#[derive(Debug, Error)]
pub enum LcdError {
    #[error("USB error: {0}")]
    Usb(#[from] rusb::Error),
    #[error("LY device (0416:5408 / 0416:5409) not found")]
    NotFound,
    #[error("handshake failed, invalid response: {0:02x?}")]
    BadHandshake(Vec<u8>),
    #[error("empty frame")]
    EmptyFrame,
    #[error("frame of {0} bytes exceeds the firmware limit of {MAX_FRAME_BYTES} bytes")]
    FrameTooLarge(usize),
    #[error("JPEG encode failed: {0}")]
    Jpeg(String),
}

pub type Result<T> = std::result::Result<T, LcdError>;

/// LY family variant, auto-detected from the PID at `open()` time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// 0416:5408 — chunk header byte[8]=1, chunk count rounded up to a multiple of 4.
    Ly,
    /// 0416:5409 — chunk header byte[8]=2, no chunk rounding.
    Ly1,
}

impl Variant {
    fn from_pid(pid: u16) -> Option<Self> {
        match pid {
            PID_LY => Some(Variant::Ly),
            PID_LY1 => Some(Variant::Ly1),
            _ => None,
        }
    }

    fn chunk_cmd(self) -> u8 {
        match self {
            Variant::Ly => 1,
            Variant::Ly1 => 2,
        }
    }

    fn pad_multiple(self) -> usize {
        match self {
            Variant::Ly => 4,
            Variant::Ly1 => 1,
        }
    }
}

/// Parsed handshake response (equivalent to `HandshakeResult` in Python).
#[derive(Debug, Clone)]
pub struct Handshake {
    pub raw_response: Vec<u8>,
    pub pm: u8,
    pub sub: u8,
    /// 180° rotation heuristic — see the module notes. `true` = rotate 180° before encoding.
    pub rotate_180: bool,
    /// Canvas -> panel rotation (see [`Orientation::output_rotation`]).
    /// Combined with `rotate_180` in `send_framebuffer`.
    pub rotation: Rotation,
    /// Brightness 0-100 (100 = unchanged). The LY protocol has no known
    /// brightness command: the image is dimmed in software before sending.
    pub brightness: u8,
}

/// Screen resolution in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Swap width <-> height (used for the portrait canvas).
    pub const fn transposed(self) -> Self {
        Self { width: self.height, height: self.width }
    }
}

/// Clockwise rotation applied to the canvas BEFORE it's encoded to
/// JPEG and sent. The physical panel is always 1920x462 (landscape); 90/270
/// are used when the screen is mounted upright: the logical canvas is then
/// 462x1920 (portrait), and gets rotated to match the panel's native pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    pub fn degrees(self) -> u32 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }

    pub fn from_degrees(d: u32) -> Option<Self> {
        match d % 360 {
            0 => Some(Rotation::R0),
            90 => Some(Rotation::R90),
            180 => Some(Rotation::R180),
            270 => Some(Rotation::R270),
            _ => None,
        }
    }

    /// Add 180° (used for the "flip" option / screen mounted upside down).
    pub fn plus_180(self) -> Self {
        Self::from_degrees(self.degrees() + 180).unwrap()
    }

    /// True if the rotation swaps width/height (90 or 270).
    pub fn swaps_axes(self) -> bool {
        matches!(self, Rotation::R90 | Rotation::R270)
    }
}

/// Image canvas orientation. The physical panel stays 1920x462; `Portrait` = a
/// 462x1920 canvas rotated 90°/270° when sent (screen mounted upright).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Landscape,
    Portrait,
}

impl Orientation {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "landscape" | "horizontal" | "orizzontale" | "h" => Some(Orientation::Landscape),
            "portrait" | "vertical" | "verticale" | "v" => Some(Orientation::Portrait),
            _ => None,
        }
    }

    /// Logical canvas resolution the UI is drawn on, for the 9.16" panel.
    pub fn canvas(self) -> Resolution {
        self.canvas_for(TROFEO_VISION_9_16)
    }

    /// Logical canvas for a panel of the given (landscape) size.
    pub fn canvas_for(self, panel: Resolution) -> Resolution {
        match self {
            Orientation::Landscape => panel,
            Orientation::Portrait => panel.transposed(),
        }
    }

    /// Rotation used when sending to the panel. `flip` = an extra 180°
    /// (if the result comes out upside down, e.g. the cable is on the other side).
    pub fn output_rotation(self, flip: bool) -> Rotation {
        let base = match self {
            Orientation::Landscape => Rotation::R0,
            Orientation::Portrait => Rotation::R90,
        };
        if flip { base.plus_180() } else { base }
    }
}

/// Handle to an already-open LY LCD device.
pub struct LyLcd {
    handle: DeviceHandle<Context>,
    iface: u8,
    variant: Variant,
    /// Bulk OUT & IN endpoint addresses, auto-detected from the USB device
    /// descriptor at `open()` time — NOT hardcoded, because real endpoint
    /// addresses have proven to differ across hardware/drivers (see the module
    /// notes above about 0x01 vs 0x09).
    ep_write: u8,
    ep_read: u8,
}

/// Endpoint info detected on one interface — used by `open()` and
/// usable for diagnostics (see `LyLcd::probe_endpoints`).
#[derive(Debug, Clone, Copy)]
pub struct EndpointInfo {
    pub interface: u8,
    pub address: u8,
    pub direction_in: bool,
}

impl LyLcd {
    /// Open a connection to the first LY device found (0416:5408 or 0416:5409).
    ///
    /// Bulk OUT/IN endpoints are auto-detected from the USB descriptor, not
    /// hardcoded — some Trofeo Vision 9.16 units turned out to use endpoint
    /// addresses different from what's written in the original Python source.
    pub fn open() -> Result<Self> {
        let context = Context::new()?;
        for device in context.devices()?.iter() {
            let desc = device.device_descriptor()?;
            if desc.vendor_id() != VENDOR_ID {
                continue;
            }
            let Some(variant) = Variant::from_pid(desc.product_id()) else {
                continue;
            };

            let handle = device.open()?;
            let config = device.active_config_descriptor()?;

            // Look for an interface that has a MATCHING PAIR of bulk OUT + IN endpoints.
            let mut found: Option<(u8, u8, u8)> = None; // (iface_num, ep_out, ep_in)
            for interface in config.interfaces() {
                for iface_desc in interface.descriptors() {
                    let mut ep_out = None;
                    let mut ep_in = None;
                    for ep in iface_desc.endpoint_descriptors() {
                        if ep.transfer_type() != rusb::TransferType::Bulk {
                            continue;
                        }
                        match ep.direction() {
                            rusb::Direction::Out => ep_out = Some(ep.address()),
                            rusb::Direction::In => ep_in = Some(ep.address()),
                        }
                    }
                    if let (Some(out), Some(inp)) = (ep_out, ep_in) {
                        found = Some((interface.number(), out, inp));
                    }
                }
            }

            let (iface_num, ep_write, ep_read) = found.ok_or(LcdError::NotFound)?;

            if handle.kernel_driver_active(iface_num).unwrap_or(false) {
                handle.detach_kernel_driver(iface_num)?;
            }
            handle.claim_interface(iface_num)?;

            return Ok(Self {
                handle,
                iface: iface_num,
                variant,
                ep_write,
                ep_read,
            });
        }
        Err(LcdError::NotFound)
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// Detected bulk OUT/IN endpoint addresses (for debugging).
    pub fn endpoints(&self) -> (u8, u8) {
        (self.ep_write, self.ep_read)
    }

    /// List of ALL endpoints on every interface of the first LY device
    /// found — useful for debugging when `open()` fails with `NotFound`
    /// or the handshake fails because of a wrong endpoint.
    pub fn probe_endpoints() -> Result<Vec<EndpointInfo>> {
        let context = Context::new()?;
        for device in context.devices()?.iter() {
            let desc = device.device_descriptor()?;
            if desc.vendor_id() != VENDOR_ID || Variant::from_pid(desc.product_id()).is_none() {
                continue;
            }
            let config = device.active_config_descriptor()?;
            let mut out = Vec::new();
            for interface in config.interfaces() {
                for iface_desc in interface.descriptors() {
                    for ep in iface_desc.endpoint_descriptors() {
                        out.push(EndpointInfo {
                            interface: interface.number(),
                            address: ep.address(),
                            direction_in: ep.direction() == rusb::Direction::In,
                        });
                    }
                }
            }
            return Ok(out);
        }
        Err(LcdError::NotFound)
    }

    /// Send the handshake payload (16 + 2032 bytes) and read+validate the 512-byte
    /// response, then extract PM/SUB — equivalent to `LyLcd._do_handshake` in Python.
    pub fn handshake(&self) -> Result<Handshake> {
        let mut payload = vec![0u8; 16 + 2032];
        payload[..16].copy_from_slice(&HANDSHAKE_HEADER);

        self.handle
            .write_bulk(self.ep_write, &payload, HANDSHAKE_TIMEOUT)?;

        let mut resp = vec![0u8; HANDSHAKE_READ_SIZE];
        let n = self.handle.read_bulk(self.ep_read, &mut resp, HANDSHAKE_TIMEOUT)?;
        resp.truncate(n);

        if resp.len() < 37 || resp[0] != 3 || resp[1] != 0xFF || resp[8] != 1 {
            return Err(LcdError::BadHandshake(resp));
        }

        let (pm, sub) = match self.variant {
            Variant::Ly => {
                let mut raw = resp[20];
                if raw <= 3 {
                    raw = 1;
                }
                let pm = 64 + raw;
                let raw_sub = resp.get(22).copied().unwrap_or(0);
                (pm, raw_sub + 1)
            }
            Variant::Ly1 => {
                let raw_sub = resp.get(22).copied().unwrap_or(0);
                let pm = 49 + resp[20];
                (pm, raw_sub)
            }
        };

        // The old heuristic (SUB 3/5 -> 180°, SUB 4 -> 0°) PROVED WRONG on
        // real hardware: units with PM=65 SUB=3 actually showed up upside-down
        // when rotate_180=true. Rather than guess again, the default now
        // does NOT rotate anything — if your panel actually needs a 180°
        // rotation, use the manual override in `main.rs` (`ROTATE_180_OVERRIDE`)
        // instead of relying on this field.
        let rotate_180 = false;

        Ok(Handshake {
            raw_response: resp,
            pm,
            sub,
            rotate_180,
            rotation: Rotation::R0,
            brightness: 100,
        })
    }

    /// Pack the raw payload (JPEG bytes) into a buffer of 512-byte chunks,
    /// equivalent to `LyLcd._prepare_frame` — including its "+1 chunk" quirk.
    fn prepare_frame(&self, payload: &[u8]) -> Vec<u8> {
        build_chunks(self.variant, payload)
    }

    /// Write the already-chunked frame buffer in 4096-byte writes (2048
    /// bytes for the last remainder on LY), then read the 512-byte ACK. Equivalent to
    /// `LyLcd._write_frame`, including the fixed `pos += 4096`.
    fn write_frame(&self, frame: &[u8]) -> Result<()> {
        let total_bytes = frame.len();
        let mut pos = 0usize;
        while pos < total_bytes {
            let remaining = total_bytes - pos;
            let write_size = if remaining >= USB_WRITE_SIZE {
                USB_WRITE_SIZE
            } else if self.variant == Variant::Ly {
                remaining.min(2048)
            } else {
                remaining
            };
            self.handle
                .write_bulk(self.ep_write, &frame[pos..pos + write_size], WRITE_TIMEOUT)?;
            pos += USB_WRITE_SIZE;
        }

        let mut ack = [0u8; HANDSHAKE_READ_SIZE];
        self.handle.read_bulk(self.ep_read, &mut ack, READ_TIMEOUT)?;
        Ok(())
    }

    /// Send a single frame. For this panel, `payload` must be already-encoded
    /// JPEG bytes (see [`Framebuffer::to_jpeg`]) — not raw RGB.
    pub fn send_frame(&self, payload: &[u8]) -> Result<()> {
        if payload.is_empty() {
            return Err(LcdError::EmptyFrame);
        }
        if payload.len() > MAX_FRAME_BYTES {
            return Err(LcdError::FrameTooLarge(payload.len()));
        }
        let frame = self.prepare_frame(payload);
        self.write_frame(&frame)
    }

    /// Full pipeline: encode the `Framebuffer` to JPEG (respecting `handshake.rotate_180`)
    /// then send it.
    pub fn send_framebuffer(&self, handshake: &Handshake, fb: &Framebuffer, quality: u8) -> Result<()> {
        let rot = if handshake.rotate_180 {
            handshake.rotation.plus_180()
        } else {
            handshake.rotation
        };
        let rotated;
        let src = if rot == Rotation::R0 {
            fb
        } else {
            rotated = fb.rotated(rot);
            &rotated
        };
        let dimmed;
        let src = if handshake.brightness < 100 {
            dimmed = src.with_brightness(handshake.brightness);
            &dimmed
        } else {
            src
        };
        // Photo/video backgrounds can exceed the firmware's 450 KB limit:
        // in that case, retry with lower quality instead of failing.
        let mut q = quality.clamp(1, 100);
        loop {
            match src.to_jpeg(q) {
                Ok(jpeg) => return self.send_frame(&jpeg),
                Err(LcdError::FrameTooLarge(_)) if q > 20 => q = q.saturating_sub(12).max(20),
                Err(e) => return Err(e),
            }
        }
    }

    pub fn release(self) -> Result<()> {
        self.handle.release_interface(self.iface)?;
        Ok(())
    }
}

/// Pure chunk-building logic — kept separate from `LyLcd` so it can be tested
/// without real USB hardware. Equivalent to `LyLcd._prepare_frame` in Python,
/// including the "+1 chunk" quirk when `total_size` is exactly a multiple of 496.
fn build_chunks(variant: Variant, payload: &[u8]) -> Vec<u8> {
    let total_size = payload.len();
    let num_chunks = total_size / CHUNK_DATA_SIZE + 1;
    let last_chunk_data = total_size % CHUNK_DATA_SIZE;

    let mut chunks = vec![0u8; num_chunks * CHUNK_SIZE];
    for i in 0..num_chunks {
        let offset = i * CHUNK_SIZE;
        let is_last = i == num_chunks - 1;
        let data_len = if is_last { last_chunk_data } else { CHUNK_DATA_SIZE };

        chunks[offset] = 0x01;
        chunks[offset + 1] = 0xFF;
        chunks[offset + 2..offset + 6].copy_from_slice(&(total_size as u32).to_le_bytes());
        chunks[offset + 6..offset + 8].copy_from_slice(&(data_len as u16).to_le_bytes());
        chunks[offset + 8] = variant.chunk_cmd();
        chunks[offset + 9..offset + 11].copy_from_slice(&(num_chunks as u16).to_le_bytes());
        chunks[offset + 11..offset + 13].copy_from_slice(&(i as u16).to_le_bytes());

        let src_offset = i * CHUNK_DATA_SIZE;
        let dst = offset + CHUNK_HEADER_SIZE;
        chunks[dst..dst + data_len].copy_from_slice(&payload[src_offset..src_offset + data_len]);
    }

    // Pure zero-byte padding (not a headered chunk) up to a multiple-of-4
    // chunk count for LY (no effect for LY1), so the total buffer length is
    // always a multiple of 2048 bytes for `write_frame`.
    let pad_multiple = variant.pad_multiple();
    let mut padded_chunks = num_chunks;
    let remainder = padded_chunks % pad_multiple;
    if remainder != 0 {
        padded_chunks += pad_multiple - remainder;
    }
    chunks.resize(padded_chunks * CHUNK_SIZE, 0);
    chunks
}

/// `dst` = `dst` * (1 - a) + `src` * a, with `a` in 1-255.
#[inline]
fn blend_over(dst: &mut [u8], src: &[u8], a: u8) {
    let a = a as u32;
    for k in 0..3 {
        dst[k] = ((dst[k] as u32 * (255 - a) + src[k] as u32 * a + 127) / 255) as u8;
    }
}

/// RGB888 pixel buffer that can be filled manually (text, graphs, CPU/GPU
/// monitoring, etc.), then encoded to JPEG for sending to the panel.
pub struct Framebuffer {
    width: u32,
    height: u32,
    pixels: Vec<u8>, // RGB888, 3 byte per piksel
    /// The colour the buffer was last cleared with: where the buffer is still this colour,
    /// smooth text is drawn with a coverage (see `edge`) instead of being blended with it.
    key: Option<[u8; 3]>,
    /// Coverage (0 = none / opaque) of the anti-aliased edge pixels of smooth text drawn over
    /// the `key` colour; `blit_ui` / `blit_keyed` use it to blend those pixels with whatever is
    /// underneath instead of leaving a dark halo. Empty until smooth text is drawn.
    edge: Vec<u8>,
}

/// The font of the program's own text (status lines, clock, panels...).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TextFont {
    /// The blocky 5x7 pixel font.
    #[default]
    Pixel,
    /// Anti-aliased Poppins (see `smooth_font`).
    Smooth,
}

thread_local! {
    static TEXT_STYLE: std::cell::Cell<(TextFont, Option<smooth_font::Weight>)> = const { std::cell::Cell::new((TextFont::Pixel, None)) };
}

/// Chooses the font of `Framebuffer::draw_text` & co. for the calling thread (the one that draws
/// the screen); `weight` forces a weight of the smooth font (`None` = by size).
pub fn set_text_style(font: TextFont, weight: Option<smooth_font::Weight>) {
    TEXT_STYLE.with(|c| c.set((font, weight)));
}

/// The font in use (see `set_text_style`).
pub fn text_style() -> (TextFont, Option<smooth_font::Weight>) {
    TEXT_STYLE.with(|c| c.get())
}

/// The em size and weight the smooth font uses for text of pixel-font `scale`: capital letters
/// as tall as the pixel font's (7 * scale), a stronger weight for small text.
fn smooth_metrics(scale: u32) -> (f32, smooth_font::Weight) {
    let cap = (font::GLYPH_HEIGHT * scale.max(1)) as f32;
    let weight = text_style().1.unwrap_or(if cap <= 21.0 {
        smooth_font::Weight::Medium
    } else if cap <= 70.0 {
        smooth_font::Weight::Regular
    } else {
        smooth_font::Weight::Light
    });
    (smooth_font::px_for_cap(cap), weight)
}

impl Framebuffer {
    pub fn new(resolution: Resolution) -> Self {
        Self {
            width: resolution.width,
            height: resolution.height,
            pixels: vec![0u8; (resolution.width * resolution.height * 3) as usize],
            key: None,
            edge: Vec::new(),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.pixels
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    /// Fill the entire buffer with one color. Called once per frame (across
    /// the whole 1920x462 screen = ~2.66MB), so this is a fairly hot
    /// spot — optimized with a "doubling" technique: fill the first 3 bytes
    /// manually, then each step doubles the already-filled portion via
    /// `copy_within` (a large-block memcpy, not a per-pixel loop with
    /// individual bounds-checks). The number of copy operations becomes O(log n), not O(n).
    pub fn clear(&mut self, r: u8, g: u8, b: u8) {
        if self.pixels.is_empty() {
            return;
        }
        self.key = Some([r, g, b]);
        self.edge.fill(0);
        self.pixels[0..3].copy_from_slice(&[r, g, b]);
        let mut filled = 3usize;
        let total = self.pixels.len();
        while filled < total {
            let copy_len = filled.min(total - filled);
            self.pixels.copy_within(0..copy_len, filled);
            filled += copy_len;
        }
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = ((y * self.width + x) * 3) as usize;
        self.pixels[idx..idx + 3].copy_from_slice(&[r, g, b]);
    }

    /// Draw a filled rectangle — the basis for monitoring graph bars & text
    /// (every glyph bit in `draw_text` also goes through this). Called hundreds of
    /// thousands of times per frame (48 bars + dozens of status characters), so it's optimized:
    /// per row, fill the first pixel then double the rest via `copy_within`
    /// (doubling, same as `clear`) — instead of per-pixel `set_pixel` with
    /// individual bounds-checks, and with no extra heap allocation at all.
    pub fn fill_rect(&mut self, x0: u32, y0: u32, w: u32, h: u32, r: u8, g: u8, b: u8) {
        let x0 = x0.min(self.width);
        let y0 = y0.min(self.height);
        let x1 = (x0 + w).min(self.width);
        let y1 = (y0 + h).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }

        let stride = self.width as usize; // pixels per row
        let row_w = (x1 - x0) as usize; // pixels in this range
        let row_bytes = row_w * 3;

        for y in y0..y1 {
            let row_start = (y as usize * stride + x0 as usize) * 3;
            let row = &mut self.pixels[row_start..row_start + row_bytes];
            row[0..3].copy_from_slice(&[r, g, b]);
            let mut filled = 3usize;
            while filled < row_bytes {
                let copy_len = filled.min(row_bytes - filled);
                row.copy_within(0..copy_len, filled);
                filled += copy_len;
            }
        }
    }

    /// Draw text using the internal 5x7 bitmap font (see `font.rs`). Only
    /// supports uppercase letters, digits, and common symbols (`: % . - /`) —
    /// other characters are drawn as a space. `scale` = pixel size per
    /// glyph "pixel" (1 = native 5x7, 2 = 10x14, etc.).
    ///
    /// Returns the total text width in pixels (useful for
    /// centering/laying out other text).
    pub fn draw_text(&mut self, x: u32, y: u32, text: &str, r: u8, g: u8, b: u8, scale: u32) -> u32 {
        if text_style().0 == TextFont::Smooth {
            return self.draw_text_smooth(x, y, text, r, g, b, scale);
        }
        let scale = scale.max(1);
        let advance = (font::GLYPH_WIDTH + 1) * scale;
        let mut cursor_x = x;

        for ch in text.chars() {
            let rows = font::glyph(ch);
            for (row_idx, row_bits) in rows.iter().enumerate() {
                for col in 0..font::GLYPH_WIDTH {
                    let bit = font::GLYPH_WIDTH - 1 - col; // bit4 = leftmost column
                    if (row_bits >> bit) & 1 == 1 {
                        let px = cursor_x + col * scale;
                        let py = y + row_idx as u32 * scale;
                        self.fill_rect(px, py, scale, scale, r, g, b);
                    }
                }
            }
            cursor_x += advance;
        }

        cursor_x.saturating_sub(x)
    }

    /// Same as `draw_text`, but `x` may be negative (`i64`) and the drawn
    /// result is clipped so that only pixels falling in the range
    /// `[clip_x0, clip_x1)` are actually drawn. Used for the
    /// scroll/marquee effect: characters currently "exiting" the left/right of the window
    /// are automatically not drawn, without needing u32 arithmetic that could
    /// underflow. A character is treated as "whole" (all-or-nothing per glyph
    /// pixel column) — there's no sub-pixel clipping in the middle of a character.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_clipped(
        &mut self,
        x: i64,
        y: u32,
        text: &str,
        r: u8,
        g: u8,
        b: u8,
        scale: u32,
        clip_x0: u32,
        clip_x1: u32,
    ) {
        if text_style().0 == TextFont::Smooth {
            return self.draw_text_smooth_clipped(x, y, text, (r, g, b), scale, clip_x0, clip_x1);
        }
        let scale = scale.max(1);
        let advance = (font::GLYPH_WIDTH + 1) * scale;
        let mut cursor_x = x;
        let clip_x0 = clip_x0 as i64;
        let clip_x1 = clip_x1 as i64;

        for ch in text.chars() {
            let rows = font::glyph(ch);
            for (row_idx, row_bits) in rows.iter().enumerate() {
                for col in 0..font::GLYPH_WIDTH {
                    let bit = font::GLYPH_WIDTH - 1 - col;
                    if (row_bits >> bit) & 1 == 1 {
                        let px = cursor_x + (col * scale) as i64;
                        if px >= clip_x0 && px + scale as i64 <= clip_x1 {
                            let py = y + row_idx as u32 * scale;
                            self.fill_rect(px as u32, py, scale, scale, r, g, b);
                        }
                    }
                }
            }
            cursor_x += advance as i64;
        }
    }

    /// `draw_text` in the smooth font: capital letters are as tall as the pixel font's at the
    /// same `scale`, so a layout made for one fits the other. Mixed case, accents and the degree
    /// sign are drawn as they are.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_smooth(&mut self, x: u32, y: u32, text: &str, r: u8, g: u8, b: u8, scale: u32) -> u32 {
        let (px, weight) = smooth_metrics(scale);
        let baseline = (y + font::GLYPH_HEIGHT * scale.max(1)) as f32;
        let w = smooth_font::draw_with(text, x as f32, baseline, px, weight, |gx, gy, a| self.plot_cov(gx, gy, [r, g, b], a));
        w.ceil() as u32
    }

    /// The scrolling variant of `draw_text_smooth`: only pixels in `[clip_x0, clip_x1)` are drawn
    /// (a letter can be cut in the middle).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_smooth_clipped(&mut self, x: i64, y: u32, text: &str, rgb: (u8, u8, u8), scale: u32, clip_x0: u32, clip_x1: u32) {
        let (px, weight) = smooth_metrics(scale);
        let baseline = (y + font::GLYPH_HEIGHT * scale.max(1)) as f32;
        let (x0, x1) = (clip_x0 as i32, clip_x1 as i32);
        let c = [rgb.0, rgb.1, rgb.2];
        // the glyph cache works on f32 positions: keep them small enough to stay exact
        let start = x.clamp(-1_000_000, 1_000_000) as f32;
        smooth_font::draw_with(text, start, baseline, px, weight, |gx, gy, a| {
            if gx >= x0 && gx < x1 {
                self.plot_cov(gx, gy, c, a);
            }
        });
    }

    /// One pixel of anti-aliased text: `cov` (0-255) of colour `c`. Over the colour the buffer was
    /// cleared with (the "transparent" colour of the UI layer) the pixel gets the pure text colour
    /// and a coverage kept in `edge`, so that it blends with the real background when the layer
    /// is composed; anywhere else it is blended with what is there.
    fn plot_cov(&mut self, x: i32, y: i32, c: [u8; 3], cov: u8) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 || cov == 0 {
            return;
        }
        let i = y as usize * self.width as usize + x as usize;
        let p = i * 3;
        let dst = [self.pixels[p], self.pixels[p + 1], self.pixels[p + 2]];
        if cov == 255 {
            if self.key != Some(c) {
                self.pixels[p..p + 3].copy_from_slice(&c);
                if !self.edge.is_empty() {
                    self.edge[i] = 0;
                }
            }
            return;
        }
        let partial = !self.edge.is_empty() && self.edge[i] != 0;
        if partial || self.key == Some(dst) {
            if self.edge.is_empty() {
                self.edge = vec![0u8; self.width as usize * self.height as usize];
            }
            let a0 = if partial { self.edge[i] as u32 } else { 0 };
            let a = a0 + cov as u32 * (255 - a0) / 255;
            self.pixels[p..p + 3].copy_from_slice(&c);
            self.edge[i] = if a >= 250 { 0 } else { a.max(1) as u8 };
        } else {
            let a = cov as u32;
            for k in 0..3 {
                self.pixels[p + k] = ((dst[k] as u32 * (255 - a) + c[k] as u32 * a + 127) / 255) as u8;
            }
        }
    }

    /// Total width (pixels) if `text` is drawn with `draw_text` at this `scale`.
    pub fn text_width(text: &str, scale: u32) -> u32 {
        if text_style().0 == TextFont::Smooth {
            let (px, w) = smooth_metrics(scale);
            return smooth_font::text_width(text, px, w).ceil() as u32;
        }
        let scale = scale.max(1);
        let advance = (font::GLYPH_WIDTH + 1) * scale;
        text.chars().count() as u32 * advance
    }

    /// Height (pixels) of one line of text at this `scale`.
    pub fn text_height(scale: u32) -> u32 {
        font::GLYPH_HEIGHT * scale.max(1)
    }

    /// A copy rotated 180° (used when `Handshake::rotate_180 == true`).
    pub fn rotated_180(&self) -> Framebuffer {
        let mut out = Framebuffer::new(Resolution::new(self.width, self.height));
        for y in 0..self.height {
            for x in 0..self.width {
                let src = (((self.height - 1 - y) * self.width + (self.width - 1 - x)) * 3) as usize;
                let dst = ((y * self.width + x) * 3) as usize;
                out.pixels[dst..dst + 3].copy_from_slice(&self.pixels[src..src + 3]);
            }
        }
        out
    }

    /// A copy with brightness `pct` (0-100) applied to every channel.
    pub fn with_brightness(&self, pct: u8) -> Framebuffer {
        let pct = pct.min(100) as u32;
        let mut lut = [0u8; 256];
        for (i, v) in lut.iter_mut().enumerate() {
            *v = ((i as u32 * pct + 50) / 100) as u8;
        }
        Framebuffer {
            width: self.width,
            height: self.height,
            pixels: self.pixels.iter().map(|&p| lut[p as usize]).collect(),
            key: None,
            edge: Vec::new(),
        }
    }

    /// Copy all pixels from `src` (same size; otherwise a no-op).
    pub fn copy_from(&mut self, src: &Framebuffer) {
        if src.width == self.width && src.height == self.height {
            self.pixels.copy_from_slice(&src.pixels);
        }
    }

    /// Paste `src` with its top-left corner at (x, y); clips to the bounds.
    pub fn blit(&mut self, src: &Framebuffer, x: u32, y: u32) {
        self.blit_impl(src, x, y, None);
    }

    /// Like `blit` but skips pixels of color `key` (transparency).
    pub fn blit_keyed(&mut self, src: &Framebuffer, x: u32, y: u32, key: (u8, u8, u8)) {
        self.blit_impl(src, x, y, Some(key));
    }

    fn blit_impl(&mut self, src: &Framebuffer, x: u32, y: u32, key: Option<(u8, u8, u8)>) {
        if x >= self.width || y >= self.height {
            return;
        }
        let w = src.width.min(self.width - x) as usize;
        let h = src.height.min(self.height - y) as usize;
        for row in 0..h {
            let s0 = row * src.width as usize * 3;
            let d0 = ((y as usize + row) * self.width as usize + x as usize) * 3;
            let srow = &src.pixels[s0..s0 + w * 3];
            let drow = &mut self.pixels[d0..d0 + w * 3];
            let erow = (!src.edge.is_empty()).then(|| &src.edge[row * src.width as usize..row * src.width as usize + w]);
            match key {
                None => drow.copy_from_slice(srow),
                Some((kr, kg, kb)) => {
                    for (n, (sp, dp)) in srow.chunks_exact(3).zip(drow.chunks_exact_mut(3)).enumerate() {
                        if let Some(a) = erow.map(|e| e[n]).filter(|&a| a != 0) {
                            blend_over(dp, sp, a);
                        } else if !(sp[0] == kr && sp[1] == kg && sp[2] == kb) {
                            dp.copy_from_slice(sp);
                        }
                    }
                }
            }
        }
    }

    /// Like `blit_keyed`, but pixels of color `panel.0` are "semi-transparent
    /// panels": they get blended with the underlying background using
    /// color `panel.1` and opacity `panel.2` (0-100).
    pub fn blit_ui(
        &mut self,
        src: &Framebuffer,
        x: u32,
        y: u32,
        key: (u8, u8, u8),
        panel: Option<((u8, u8, u8), (u8, u8, u8), u8)>,
    ) {
        if x >= self.width || y >= self.height {
            return;
        }
        let w = src.width.min(self.width - x) as usize;
        let h = src.height.min(self.height - y) as usize;
        for row in 0..h {
            let s0 = row * src.width as usize * 3;
            let d0 = ((y as usize + row) * self.width as usize + x as usize) * 3;
            let srow = &src.pixels[s0..s0 + w * 3];
            let drow = &mut self.pixels[d0..d0 + w * 3];
            let erow = (!src.edge.is_empty()).then(|| &src.edge[row * src.width as usize..row * src.width as usize + w]);
            for (n, (sp, dp)) in srow.chunks_exact(3).zip(drow.chunks_exact_mut(3)).enumerate() {
                if let Some(a) = erow.map(|e| e[n]).filter(|&a| a != 0) {
                    blend_over(dp, sp, a);
                    continue;
                }
                let px = (sp[0], sp[1], sp[2]);
                if px == key {
                    continue;
                }
                match panel {
                    Some((pk, pc, a)) if px == pk => {
                        let a = a.min(100) as u32;
                        dp[0] = ((dp[0] as u32 * (100 - a) + pc.0 as u32 * a) / 100) as u8;
                        dp[1] = ((dp[1] as u32 * (100 - a) + pc.1 as u32 * a) / 100) as u8;
                        dp[2] = ((dp[2] as u32 * (100 - a) + pc.2 as u32 * a) / 100) as u8;
                    }
                    _ => dp.copy_from_slice(sp),
                }
            }
        }
    }

    /// Build a framebuffer from already-prepared RGB888 pixels.
    pub fn from_rgb(width: u32, height: u32, pixels: Vec<u8>) -> Option<Self> {
        if pixels.len() == (width * height * 3) as usize {
            Some(Self { width, height, pixels, key: None, edge: Vec::new() })
        } else {
            None
        }
    }

    /// A copy rotated clockwise by `rot`. For 90/270, width and height are swapped.
    pub fn rotated(&self, rot: Rotation) -> Framebuffer {
        let (w, h) = (self.width as usize, self.height as usize);
        match rot {
            Rotation::R0 => Framebuffer {
                width: self.width,
                height: self.height,
                pixels: self.pixels.clone(),
                key: None,
                edge: Vec::new(),
            },
            Rotation::R180 => self.rotated_180(),
            Rotation::R90 | Rotation::R270 => {
                // Result: width = h, height = w.
                let mut out = Framebuffer::new(Resolution::new(self.height, self.width));
                for y in 0..h {
                    for x in 0..w {
                        let (dx, dy) = if rot == Rotation::R90 {
                            (h - 1 - y, x) // (x,y) -> (h-1-y, x)
                        } else {
                            (y, w - 1 - x) // (x,y) -> (y, w-1-x)
                        };
                        let src = (y * w + x) * 3;
                        let dst = (dy * h + dx) * 3;
                        out.pixels[dst..dst + 3].copy_from_slice(&self.pixels[src..src + 3]);
                    }
                }
                out
            }
        }
    }

    /// Encode to JPEG. `quality` 1-100. Returns an error if the result
    /// exceeds `MAX_FRAME_BYTES` (lower `quality` if that happens).
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>> {
        use jpeg_encoder::{ColorType, Encoder};

        let mut out = Vec::new();
        let encoder = Encoder::new(&mut out, quality);
        encoder
            .encode(&self.pixels, self.width as u16, self.height as u16, ColorType::Rgb)
            .map_err(|e| LcdError::Jpeg(e.to_string()))?;

        if out.len() > MAX_FRAME_BYTES {
            return Err(LcdError::FrameTooLarge(out.len()));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handshake_recognises_the_11_3() {
        let mut r = vec![0u8; 512];
        assert_eq!(panel_from_handshake(&r), None);
        for v in [1u8, 2, 3] { r[20] = v; assert_eq!(panel_from_handshake(&r), None); }
        r[20] = 5;
        assert_eq!(panel_from_handshake(&r), Some(Panel::Trofeo11_3));
        assert_eq!(panel_from_handshake(&r[..10]), None);
    }

    #[test]
    fn panel_models_and_sizes() {
        assert_eq!(Panel::default().resolution(), Resolution::new(1920, 462));
        for n in ["trofeo-11.3", "Trofeo Vision 11.3 LCD", "11.3", "trofeo_11.3", "TROFEO-VISION-11.3"] {
            assert_eq!(Panel::parse(n), Some(Panel::Trofeo11_3), "{n}");
        }
        for n in ["trofeo-9.16", "9.16", "Trofeo Vision 9.16 LCD"] {
            assert_eq!(Panel::parse(n), Some(Panel::Trofeo9_16), "{n}");
        }
        assert_eq!(Panel::parse("trofeo-7.0"), None);
        assert_eq!(Panel::Trofeo11_3.resolution(), Resolution::new(1920, 400));
        for (t, w, h) in [("1920x400", 1920, 400), ("1920 X 440", 1920, 440), ("1920*400", 1920, 400), ("1920, 462", 1920, 462), ("1920×400", 1920, 400)] {
            assert_eq!(parse_resolution(t), Some(Resolution::new(w, h)), "{t}");
        }
        for t in ["", "1920", "abcxdef", "1920x400x3", "10x10", "99999x400", "1920x-5"] {
            assert_eq!(parse_resolution(t), None, "{t}");
        }
    }

    #[test]
    fn canvas_follows_the_panel_in_both_orientations() {
        let p = TROFEO_VISION_11_3;
        assert_eq!(Orientation::Landscape.canvas_for(p), Resolution::new(1920, 400));
        assert_eq!(Orientation::Portrait.canvas_for(p), Resolution::new(400, 1920));
        // What reaches the panel is always the panel's own size, whatever the orientation.
        for o in [Orientation::Landscape, Orientation::Portrait] {
            for flip in [false, true] {
                let fb = Framebuffer::new(o.canvas_for(p));
                let out = fb.rotated(o.output_rotation(flip));
                assert_eq!((out.width(), out.height()), (1920, 400), "{o:?} flip={flip}");
            }
        }
        // The 9.16 default is unchanged.
        assert_eq!(Orientation::Landscape.canvas(), Resolution::new(1920, 462));
        assert_eq!(Orientation::Portrait.canvas(), Resolution::new(462, 1920));
    }

    #[test]
    fn chunk_count_has_trailing_empty_chunk_on_exact_multiple() {
        // total_size exactly 2 * 496 -> must be 3 chunks (2 full + 1 empty).
        let payload = vec![0xAAu8; CHUNK_DATA_SIZE * 2];
        let frame = build_chunks(Variant::Ly, &payload);
        // 3 real chunks, rounded up to a multiple of 4 -> 4 chunks -> 2048 bytes.
        assert_eq!(frame.len(), 4 * CHUNK_SIZE);
        // The header of the last of the 3 real chunks (index 2) has data_len 0.
        let last_real_chunk_offset = 2 * CHUNK_SIZE;
        assert_eq!(frame[last_real_chunk_offset + 6], 0);
        assert_eq!(frame[last_real_chunk_offset + 7], 0);
    }

    #[test]
    fn chunk_count_normal_case() {
        let payload = vec![0xBBu8; 1000];
        let frame = build_chunks(Variant::Ly, &payload);
        // 1000/496 + 1 = 3 real chunks -> rounded up to a multiple of 4 -> 4 chunks.
        assert_eq!(frame.len(), 4 * CHUNK_SIZE);
    }

    #[test]
    fn ly1_has_no_padding() {
        let payload = vec![0xCCu8; CHUNK_DATA_SIZE * 2]; // -> 3 real chunks
        let frame = build_chunks(Variant::Ly1, &payload);
        assert_eq!(frame.len(), 3 * CHUNK_SIZE); // no rounding to a multiple of 4
    }

    #[test]
    fn blit_ui_blends_panel_pixels() {
        let mut dst = Framebuffer::new(Resolution::new(2, 1));
        dst.clear(200, 100, 0);
        let mut src = Framebuffer::new(Resolution::new(2, 1));
        src.set_pixel(0, 0, 2, 0, 1); // "panel" color
        src.set_pixel(1, 0, 1, 0, 2); // transparency key
        dst.blit_ui(&src, 0, 0, (1, 0, 2), Some(((2, 0, 1), (0, 0, 100), 50)));
        assert_eq!(&dst.as_bytes()[0..3], &[100, 50, 50]);
        assert_eq!(&dst.as_bytes()[3..6], &[200, 100, 0]);
    }

    #[test]
    fn brightness_scales_channels() {
        let mut fb = Framebuffer::new(Resolution::new(1, 1));
        fb.set_pixel(0, 0, 200, 100, 0);
        assert_eq!(fb.with_brightness(50).as_bytes(), &[100, 50, 0]);
        assert_eq!(fb.with_brightness(0).as_bytes(), &[0, 0, 0]);
        assert_eq!(fb.with_brightness(100).as_bytes(), &[200, 100, 0]);
    }

    #[test]
    fn blit_keyed_skips_key_color() {
        let mut dst = Framebuffer::new(Resolution::new(4, 4));
        dst.clear(9, 9, 9);
        let mut src = Framebuffer::new(Resolution::new(2, 2));
        src.clear(1, 0, 2);
        src.set_pixel(1, 1, 200, 0, 0);
        dst.blit_keyed(&src, 1, 1, (1, 0, 2));
        assert_eq!(dst.as_bytes()[((1 * 4 + 1) * 3) as usize], 9); // key: skipped
        assert_eq!(dst.as_bytes()[((2 * 4 + 2) * 3) as usize], 200);
    }

    #[test]
    fn rotate_90_and_270_are_inverse_and_swap_axes() {
        let mut fb = Framebuffer::new(Resolution::new(3, 2));
        fb.set_pixel(0, 0, 10, 0, 0); // top-left
        fb.set_pixel(2, 0, 20, 0, 0); // top-right
        let r90 = fb.rotated(Rotation::R90);
        assert_eq!((r90.width(), r90.height()), (2, 3));
        // 90° CW: top-left -> top-right, top-right -> bottom-right.
        assert_eq!(r90.as_bytes()[((0 * 2 + 1) * 3) as usize], 10);
        assert_eq!(r90.as_bytes()[((2 * 2 + 1) * 3) as usize], 20);
        let back = r90.rotated(Rotation::R270);
        assert_eq!(back.as_bytes(), fb.as_bytes());
    }

    #[test]
    fn portrait_canvas_rotates_to_native_panel_size() {
        let canvas = Framebuffer::new(Orientation::Portrait.canvas());
        let out = canvas.rotated(Orientation::Portrait.output_rotation(false));
        assert_eq!((out.width(), out.height()), (1920, 462));
    }

    #[test]
    fn framebuffer_rotate_180_swaps_corners() {
        let mut fb = Framebuffer::new(Resolution::new(2, 2));
        fb.set_pixel(0, 0, 1, 0, 0);
        fb.set_pixel(1, 1, 2, 0, 0);
        let rotated = fb.rotated_180();
        assert_eq!(rotated.as_bytes()[0], 2); // (0,0) rotated <- (1,1) original
        let idx_11 = ((1 * 2 + 1) * 3) as usize;
        assert_eq!(rotated.as_bytes()[idx_11], 1); // (1,1) rotated <- (0,0) original
    }

    #[test]
    fn smooth_text_has_the_height_of_the_pixel_font_and_a_width() {
        set_text_style(TextFont::Smooth, None);
        for scale in [1u32, 2, 3, 6, 20] {
            let w = Framebuffer::text_width("20:51:49", scale);
            // about 6 pixels per character per unit of scale, like the pixel font
            let pixel = 8 * 6 * scale;
            assert!(w * 10 > pixel * 6 && w * 10 < pixel * 14, "scale {scale}: {w} vs {pixel}");
            assert_eq!(Framebuffer::text_height(scale), 7 * scale);
        }
        let mut fb = Framebuffer::new(Resolution::new(300, 60));
        fb.clear(1, 0, 2);
        let w = fb.draw_text(10, 10, "Thursday 15", 255, 255, 255, 3);
        assert!(w > 100 && w < 250, "{w}");
        assert!(fb.as_bytes().chunks_exact(3).any(|p| p == [255, 255, 255]));
        set_text_style(TextFont::Pixel, None);
        assert_eq!(Framebuffer::text_width("ABC", 2), 3 * 6 * 2);
    }

    #[test]
    fn smooth_text_edges_blend_with_the_background_not_with_the_key_colour() {
        set_text_style(TextFont::Smooth, None);
        let key = (1u8, 0u8, 2u8);
        let mut ui = Framebuffer::new(Resolution::new(200, 50));
        ui.clear(key.0, key.1, key.2);
        ui.draw_text(5, 5, "Wag", 255, 255, 255, 4);
        set_text_style(TextFont::Pixel, None);
        // composed over a mid-grey background: no pixel may be darker than the background
        // (a halo would be), and the text colour is reached
        let mut bg = Framebuffer::new(Resolution::new(200, 50));
        bg.clear(120, 120, 120);
        bg.blit_ui(&ui, 0, 0, key, None);
        let b = bg.as_bytes();
        assert!(b.chunks_exact(3).all(|p| p[0] >= 120 && p[0] == p[1] && p[1] == p[2]), "dark halo");
        assert!(b.chunks_exact(3).any(|p| p[0] == 255), "full text colour");
        assert!(b.chunks_exact(3).any(|p| p[0] > 130 && p[0] < 250), "anti-aliased edges");
        // blit_keyed does the same
        let mut bg2 = Framebuffer::new(Resolution::new(200, 50));
        bg2.clear(120, 120, 120);
        bg2.blit_keyed(&ui, 0, 0, key);
        assert_eq!(bg.as_bytes(), bg2.as_bytes());
        // a clear() forgets the edges
        ui.clear(key.0, key.1, key.2);
        let mut bg3 = Framebuffer::new(Resolution::new(200, 50));
        bg3.clear(120, 120, 120);
        bg3.blit_ui(&ui, 0, 0, key, None);
        assert!(bg3.as_bytes().chunks_exact(3).all(|p| p == [120, 120, 120]));
    }

    #[test]
    fn smooth_text_over_a_panel_blends_with_the_panel_and_scrolls_clipped() {
        set_text_style(TextFont::Smooth, None);
        let mut fb = Framebuffer::new(Resolution::new(120, 30));
        fb.clear(40, 40, 90);
        fb.draw_text(2, 2, "OK", 255, 255, 255, 2);
        // nothing is darker than the panel, nothing outside the glyph area changed
        assert!(fb.as_bytes().chunks_exact(3).all(|p| p[0] >= 40));
        let mut s = Framebuffer::new(Resolution::new(120, 30));
        s.clear(0, 0, 0);
        s.draw_text_clipped(-20, 2, "Scrolling title", 255, 255, 255, 2, 10, 60);
        let b = s.as_bytes();
        for y in 0..30usize {
            for x in 0..120usize {
                if x < 10 || x >= 60 {
                    assert_eq!(&b[(y * 120 + x) * 3..(y * 120 + x) * 3 + 3], &[0, 0, 0], "outside the window at {x},{y}");
                }
            }
        }
        assert!(b.iter().any(|&v| v > 100), "something is drawn inside the window");
        set_text_style(TextFont::Pixel, None);
    }
}
