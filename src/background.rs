//! Background behind the interface: image (JPEG/PNG/BMP), animated GIF or video.
//!
//! - Images and GIFs are decoded once and scaled in "cover" mode (fills the
//!   canvas, cropping to center) to the canvas resolution. GIFs are kept in
//!   RAM (limit ~240 MB: if there are too many frames, some are skipped,
//!   with their delays added together so the total duration stays the same).
//! - Videos require **ffmpeg** (not bundled): it looks for `ffmpeg.exe` next
//!   to the program, then `ffmpeg` on the PATH, or the path given via
//!   `ffmpeg = ...`. ffmpeg decodes in a loop and sends raw RGB frames to a
//!   thread, which always keeps only the latest frame.

use crate::layout::{BgLayout, Fit};
use crate::{Framebuffer, Resolution};
use image::{imageops::FilterType, AnimationDecoder, DynamicImage};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const GIF_RAM_BUDGET: usize = 240 * 1024 * 1024;

pub enum Background {
    Static(Framebuffer),
    Gif {
        frames: Vec<(Framebuffer, u64)>, // (frame, duration ms)
        total_ms: u64,
        start: Instant,
    },
    Video {
        latest: Arc<Mutex<Vec<u8>>>,
        /// Set on drop: the reader thread terminates ffmpeg (needed for config reloading).
        stop: Arc<std::sync::atomic::AtomicBool>,
    },
}

impl Drop for Background {
    fn drop(&mut self) {
        if let Background::Video { stop, .. } = self {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn dim_pixels(px: &mut [u8], dim: u8) {
    if dim == 0 {
        return;
    }
    let keep = 100u32 - dim.min(100) as u32;
    for v in px.iter_mut() {
        *v = ((*v as u32 * keep) / 100) as u8;
    }
}

/// Color of the letterbox bands when the image doesn't cover the whole canvas.
const LETTERBOX: [u8; 3] = [0x08, 0x08, 0x10];

fn fit(img: DynamicImage, canvas: Resolution, dim: u8, lay: &BgLayout) -> Framebuffer {
    let (cw, ch) = (canvas.width as i64, canvas.height as i64);
    let (iw, ih) = (img.width().max(1), img.height().max(1));
    let (sw, sh): (u32, u32) = match lay.fit {
        Fit::Cover => {
            let s = (cw as f64 / iw as f64).max(ch as f64 / ih as f64);
            ((iw as f64 * s).ceil() as u32, (ih as f64 * s).ceil() as u32)
        }
        Fit::Contain => {
            let s = (cw as f64 / iw as f64).min(ch as f64 / ih as f64);
            (((iw as f64 * s).round() as u32).max(1), ((ih as f64 * s).round() as u32).max(1))
        }
        Fit::Stretch => (canvas.width, canvas.height),
        Fit::Original => (iw, ih),
    };
    let scaled = if (sw, sh) == (iw, ih) {
        img.to_rgb8()
    } else {
        img.resize_exact(sw, sh, FilterType::Triangle).to_rgb8()
    };

    // Position of the scaled image on the canvas: anchor + offset.
    let mut x0 = ((cw - sw as i64) as f32 * lay.anchor.fx()) as i64 + lay.offset.0 as i64;
    let mut y0 = ((ch - sh as i64) as f32 * lay.anchor.fy()) as i64 + lay.offset.1 as i64;
    if sw as i64 >= cw {
        x0 = x0.clamp(cw - sw as i64, 0); // no bands if the image is wider
    }
    if sh as i64 >= ch {
        y0 = y0.clamp(ch - sh as i64, 0);
    }

    let mut px = vec![0u8; (cw * ch * 3) as usize];
    for p in px.chunks_exact_mut(3) {
        p.copy_from_slice(&LETTERBOX);
    }
    let src = scaled.as_raw();
    for dy in 0..sh as i64 {
        let y = y0 + dy;
        if y < 0 || y >= ch {
            continue;
        }
        let xs = 0.max(-x0);
        let xe = (sw as i64).min(cw - x0);
        if xe <= xs {
            continue;
        }
        let s_off = ((dy * sw as i64 + xs) * 3) as usize;
        let d_off = ((y * cw + x0 + xs) * 3) as usize;
        let n = ((xe - xs) * 3) as usize;
        px[d_off..d_off + n].copy_from_slice(&src[s_off..s_off + n]);
    }
    dim_pixels(&mut px, dim);
    Framebuffer::from_rgb(canvas.width, canvas.height, px).expect("consistent dimensions")
}

/// ffmpeg filter equivalent to `fit` (same anchor/offset).
fn video_filter(lay: &BgLayout, w: u32, h: u32, fps: f32) -> String {
    let (fx, fy) = (lay.anchor.fx(), lay.anchor.fy());
    let (ox, oy) = lay.offset;
    let bg = "0x080810";
    let scale_pad = match lay.fit {
        Fit::Stretch => format!("scale={w}:{h}"),
        Fit::Cover => format!(
            "scale={w}:{h}:force_original_aspect_ratio=increase,\
             crop={w}:{h}:'clip((iw-ow)*{fx}-({ox}),0,iw-ow)':'clip((ih-oh)*{fy}-({oy}),0,ih-oh)'"
        ),
        Fit::Contain => format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:'(ow-iw)*{fx}+({ox})':'(oh-ih)*{fy}+({oy})':color={bg}"
        ),
        Fit::Original => format!(
            "pad='max(iw,{w})':'max(ih,{h})':'(ow-iw)*{fx}':'(oh-ih)*{fy}':color={bg},\
             crop={w}:{h}:'clip((iw-ow)*{fx}-({ox}),0,iw-ow)':'clip((ih-oh)*{fy}-({oy}),0,ih-oh)'"
        ),
    };
    format!("fps={},{scale_pad}", fps.max(1.0))
}

impl Background {
    pub fn is_animated(&self) -> bool {
        !matches!(self, Background::Static(_))
    }

    /// `dim` 0-100 = how much to darken the background (to keep text
    /// readable); `fps` = frame rate requested from ffmpeg for videos.
    pub fn load(
        path: &Path,
        canvas: Resolution,
        dim: u8,
        fps: f32,
        ffmpeg: Option<&str>,
        layout: &BgLayout,
    ) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!("background not found: {}", path.display()));
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "bmp" => {
                let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(Background::Static(fit(img, canvas, dim, layout)))
            }
            "gif" => Self::load_gif(path, canvas, dim, layout),
            _ => Self::load_video(path, canvas, dim, fps, ffmpeg, layout),
        }
    }

    fn load_gif(path: &Path, canvas: Resolution, dim: u8, layout: &BgLayout) -> Result<Self, String> {
        let open = || -> Result<_, String> {
            let f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            image::codecs::gif::GifDecoder::new(BufReader::new(f))
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        // Pass 1: count the frames to decide the stride (limited memory).
        let total = open()?.into_frames().count();
        if total == 0 {
            return Err("GIF has no frames".into());
        }
        let frame_bytes = (canvas.width * canvas.height * 3) as usize;
        let max_frames = (GIF_RAM_BUDGET / frame_bytes).max(1);
        let stride = total.div_ceil(max_frames);

        let mut frames: Vec<(Framebuffer, u64)> = Vec::new();
        let mut acc_ms = 0u64;
        for (i, fr) in open()?.into_frames().enumerate() {
            let fr = fr.map_err(|e| format!("GIF: {e}"))?;
            let (n, d) = fr.delay().numer_denom_ms();
            let mut ms = (n as u64) / (d.max(1) as u64);
            if ms < 20 {
                ms = 100; // like browsers do: 0/10ms delays -> 100ms
            }
            acc_ms += ms;
            if i % stride == 0 {
                let img = DynamicImage::ImageRgba8(fr.into_buffer());
                frames.push((fit(img, canvas, dim, layout), acc_ms));
                acc_ms = 0;
            }
        }
        if acc_ms > 0 {
            if let Some(last) = frames.last_mut() {
                last.1 += acc_ms;
            }
        }
        let total_ms = frames.iter().map(|f| f.1).sum::<u64>().max(1);
        Ok(Background::Gif { frames, total_ms, start: Instant::now() })
    }

    fn load_video(
        path: &Path,
        canvas: Resolution,
        dim: u8,
        fps: f32,
        ffmpeg: Option<&str>,
        layout: &BgLayout,
    ) -> Result<Self, String> {
        let exe = resolve_ffmpeg(ffmpeg);
        let (w, h) = (canvas.width, canvas.height);
        let vf = video_filter(layout, w, h, fps);
        let mut cmd = Command::new(&exe);
        cmd.args(["-hide_banner", "-loglevel", "error", "-re", "-stream_loop", "-1", "-i"])
            .arg(path)
            .args(["-an", "-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "couldn't start ffmpeg ({}): {e}. Install ffmpeg and put it on the PATH, \
                 or copy ffmpeg.exe next to the program, or set ffmpeg = <path>",
                exe.display()
            )
        })?;
        let mut out = child.stdout.take().ok_or("ffmpeg: stdout not available")?;
        let n = (w * h * 3) as usize;
        let latest = Arc::new(Mutex::new(vec![0u8; n]));
        let shared = latest.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_t = stop.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; n];
            while out.read_exact(&mut buf).is_ok() {
                if stop_t.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                dim_pixels(&mut buf, dim);
                if let Ok(mut g) = shared.lock() {
                    g.copy_from_slice(&buf);
                }
            }
            eprintln!("ffmpeg terminated (video ended or error): the background stays on the last frame");
            let _ = child.kill();
        });
        Ok(Background::Video { latest, stop })
    }

    /// Copy the current background into `fb` (must have the canvas dimensions).
    pub fn render_into(&mut self, fb: &mut Framebuffer) {
        match self {
            Background::Static(f) => fb.copy_from(f),
            Background::Gif { frames, total_ms, start } => {
                let t = start.elapsed().as_millis() as u64 % *total_ms;
                let mut acc = 0;
                for (f, ms) in frames.iter() {
                    acc += ms;
                    if t < acc {
                        fb.copy_from(f);
                        return;
                    }
                }
                if let Some((f, _)) = frames.last() {
                    fb.copy_from(f);
                }
            }
            Background::Video { latest, .. } => {
                if let Ok(g) = latest.lock() {
                    if g.len() == fb.as_bytes().len() {
                        fb.as_bytes_mut().copy_from_slice(&g);
                    }
                }
            }
        }
    }
}

fn resolve_ffmpeg(explicit: Option<&str>) -> PathBuf {
    if let Some(p) = explicit.filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["ffmpeg.exe", "ffmpeg"] {
                let c = dir.join(name);
                if c.is_file() {
                    return c;
                }
            }
        }
    }
    PathBuf::from("ffmpeg")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_image_is_cover_fitted_and_dimmed() {
        let dir = std::env::temp_dir().join("trofeo_bg_test");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.png");
        image::RgbImage::from_pixel(50, 50, image::Rgb([200, 100, 50])).save(&p).unwrap();
        let canvas = Resolution::new(400, 100);
        let mut bg = Background::load(&p, canvas, 50, 15.0, None, &BgLayout::default()).unwrap();
        let mut fb = Framebuffer::new(canvas);
        bg.render_into(&mut fb);
        assert_eq!(&fb.as_bytes()[0..3], &[100, 50, 25]);
        assert!(!bg.is_animated());
    }

    /// End-to-end test of the video path (only if ffmpeg is installed).
    #[test]
    fn video_and_gif_via_ffmpeg_and_native() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            eprintln!("ffmpeg not present: test skipped");
            return;
        }
        let dir = std::env::temp_dir().join("trofeo_bg_test");
        std::fs::create_dir_all(&dir).unwrap();
        let mp4 = dir.join("t.mp4");
        let gif = dir.join("t.gif");
        for (out, extra) in [(&mp4, vec!["-pix_fmt", "yuv420p"]), (&gif, vec![])] {
            let st = Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=320x180:rate=10:duration=2"])
                .args(&extra)
                .arg(out)
                .status()
                .unwrap();
            assert!(st.success());
        }
        let canvas = Resolution::new(480, 120);
        let mut g = Background::load(&gif, canvas, 0, 15.0, None, &BgLayout::default()).unwrap();
        assert!(g.is_animated());
        let mut fb = Framebuffer::new(canvas);
        g.render_into(&mut fb);
        assert!(fb.as_bytes().iter().any(|&b| b != 0));

        for fit in [Fit::Cover, Fit::Stretch, Fit::Contain, Fit::Original] {
            let lay = BgLayout { fit, anchor: crate::layout::Anchor::parse("bottom-right").unwrap(), offset: (5, -5) };
            let mut v = Background::load(&mp4, canvas, 0, 15.0, None, &lay).unwrap();
            let mut got = false;
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                v.render_into(&mut fb);
                if fb.as_bytes().iter().any(|&b| b != 0) {
                    got = true;
                    break;
                }
            }
            assert!(got, "no video frame with fit {fit:?}");
        }
    }

    #[test]
    fn fit_modes_and_positions() {
        use crate::layout::Anchor;
        let canvas = Resolution::new(200, 100);
        // 100x100 image: red on the left, blue on the right
        let mut img = image::RgbImage::new(100, 100);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 50 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) };
        }
        let dynimg = || DynamicImage::ImageRgb8(img.clone());
        let px = |f: &Framebuffer, x: u32, y: u32| {
            let i = ((y * 200 + x) * 3) as usize;
            [f.as_bytes()[i], f.as_bytes()[i + 1], f.as_bytes()[i + 2]]
        };
        // Contain, centered: bands on the sides (x=0 letterbox), red at x=60, blue at x=140.
        let c = fit(dynimg(), canvas, 0, &BgLayout { fit: Fit::Contain, anchor: Anchor::CENTER, offset: (0, 0) });
        assert_eq!(px(&c, 0, 50), LETTERBOX);
        assert_eq!(px(&c, 60, 50), [255, 0, 0]);
        assert_eq!(px(&c, 140, 50), [0, 0, 255]);
        // Contain, left-anchored: red at x=10.
        let l = fit(dynimg(), canvas, 0, &BgLayout { fit: Fit::Contain, anchor: Anchor::parse("left").unwrap(), offset: (0, 0) });
        assert_eq!(px(&l, 10, 50), [255, 0, 0]);
        // Stretch: fully covered, red on the left and blue on the right.
        let s = fit(dynimg(), canvas, 0, &BgLayout { fit: Fit::Stretch, ..Default::default() });
        assert_eq!(px(&s, 10, 10), [255, 0, 0]);
        assert_eq!(px(&s, 190, 90), [0, 0, 255]);
        // Cover anchored at the top: no bands, fully covered.
        let t = fit(dynimg(), canvas, 0, &BgLayout { fit: Fit::Cover, anchor: Anchor::parse("top").unwrap(), offset: (0, 0) });
        assert_ne!(px(&t, 0, 0), LETTERBOX);
        assert_ne!(px(&t, 199, 99), LETTERBOX);
        // X offset on contain shifts it 20px to the right.
        let o = fit(dynimg(), canvas, 0, &BgLayout { fit: Fit::Contain, anchor: Anchor::CENTER, offset: (20, 0) });
        assert_eq!(px(&o, 60, 50), LETTERBOX);
        assert_eq!(px(&o, 80, 50), [255, 0, 0]);
    }

    #[test]
    fn missing_file_is_error() {
        assert!(Background::load(Path::new("/nonexistent.png"), Resolution::new(10, 10), 0, 15.0, None, &BgLayout::default()).is_err());
    }
}
