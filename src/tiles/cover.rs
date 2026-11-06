//! Cover pictures for the game screens: a game's box art is read from the file Steam has cached,
//! scaled once to the height it is drawn at, and kept (a few of them) for the next frames.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// A picture as straight (not premultiplied) RGBA, row after row.
pub(super) struct Tex {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Tex {
    /// The colour at (x, y) with bilinear filtering; `x` and `y` are in pixels of the picture.
    #[inline]
    pub fn sample(&self, x: f32, y: f32) -> [f32; 4] {
        let x = (x - 0.5).clamp(0.0, (self.w - 1) as f32);
        let y = (y - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |xx: usize, yy: usize| -> [f32; 4] {
            let i = (yy * self.w + xx) * 4;
            [self.px[i] as f32, self.px[i + 1] as f32, self.px[i + 2] as f32, self.px[i + 3] as f32]
        };
        let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bot = c[k] + (d[k] - c[k]) * fx;
            out[k] = top + (bot - top) * fy;
        }
        out
    }

    /// The average colour (for tinting a background after the cover).
    pub fn average(&self) -> (u8, u8, u8) {
        let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
        for p in self.px.chunks_exact(4).step_by(7) {
            r += p[0] as u64;
            g += p[1] as u64;
            b += p[2] as u64;
            n += 1;
        }
        if n == 0 {
            return (16, 16, 24);
        }
        ((r / n) as u8, (g / n) as u8, (b / n) as u8)
    }
}

/// Reads a picture and scales it so it is `height` pixels tall.
pub(super) fn load_tex(path: &Path, height: u32) -> Option<Tex> {
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let height = height.max(8);
    let width = ((w as f32 * height as f32 / h as f32).round() as u32).max(1);
    let img = image::imageops::resize(&img, width, height, image::imageops::FilterType::Triangle);
    Some(Tex { w: width as usize, h: height as usize, px: img.into_raw() })
}

/// The last pictures read. A picture that cannot be read is remembered too (it is not retried
/// every frame).
pub(super) struct CoverCache {
    items: VecDeque<(PathBuf, u32, Option<Tex>)>,
    cap: usize,
}

impl CoverCache {
    pub fn new(cap: usize) -> Self {
        CoverCache { items: VecDeque::new(), cap: cap.max(2) }
    }

    #[allow(dead_code)]
    pub fn has(&self, path: &Path, height: u32) -> bool {
        self.items.iter().any(|(p, h, _)| p == path && *h == height)
    }

    /// The picture if it is here already.
    pub fn peek(&self, path: &Path, height: u32) -> Option<&Tex> {
        self.items.iter().find(|(p, h, _)| p == path && *h == height).and_then(|(_, _, t)| t.as_ref())
    }

    /// The picture, read now if it is not here yet and `load` allows a read (so the caller can
    /// limit the reads to one per frame: it clears `load` when one was made).
    pub fn get(&mut self, path: &Path, height: u32, load: &mut bool) -> Option<&Tex> {
        if let Some(i) = self.items.iter().position(|(p, h, _)| p == path && *h == height) {
            return self.items[i].2.as_ref();
        }
        if !*load {
            return None;
        }
        *load = false;
        let tex = load_tex(path, height);
        self.items.push_back((path.to_path_buf(), height, tex));
        while self.items.len() > self.cap {
            self.items.pop_front();
        }
        self.items.back().and_then(|(_, _, t)| t.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_scaled_to_the_height_asked_and_cached() {
        let dir = std::env::temp_dir().join(format!("trofeo_cover_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("c.png");
        let img = image::RgbaImage::from_fn(60, 90, |x, _| image::Rgba([(x * 4) as u8, 40, 200, 255]));
        img.save(&f).unwrap();
        let t = load_tex(&f, 30).unwrap();
        assert_eq!((t.w, t.h), (20, 30));
        let s = t.sample(0.0, 15.0);
        assert!(s[2] > 150.0 && s[3] > 250.0);
        let mut cache = CoverCache::new(2);
        let mut load = false;
        assert!(cache.get(&f, 30, &mut load).is_none()); // reading not allowed this time
        load = true;
        assert!(cache.get(&f, 30, &mut load).is_some());
        assert!(!load, "a read was made");
        load = false;
        assert!(cache.get(&f, 30, &mut load).is_some(), "now it is cached");
        // a file that is not a picture is remembered as unreadable
        let bad = dir.join("bad.jpg");
        std::fs::write(&bad, b"nope").unwrap();
        load = true;
        assert!(cache.get(&bad, 30, &mut load).is_none());
        load = true;
        assert!(cache.get(&bad, 30, &mut load).is_none());
        assert!(load, "no second read of the same bad file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
