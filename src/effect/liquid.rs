//! The two liquid styles added after the aquarium: `sea` (a sunset seen from the water, with
//! layers of waves, foam and the sun's glitter) and `ink` (clouds of coloured ink spreading
//! in water). Both are stateless functions of the time (plus the small "live" inputs of
//! `Effect`: how loud the music is, the level of the water), like the aquarium.

use super::*;

// ---------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------

#[inline]
fn hash2(ix: i32, iy: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x27d4_eb2d) ^ (iy as u32).wrapping_mul(0x1656_67b1) ^ seed.wrapping_mul(0x9E37_79B1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Smooth value noise, 0..1.
#[inline]
fn vnoise(x: f32, y: f32, seed: u32) -> f32 {
    let (xf, yf) = (x.floor(), y.floor());
    let (ix, iy) = (xf as i32, yf as i32);
    let (fx, fy) = (x - xf, y - yf);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash2(ix, iy, seed);
    let b = hash2(ix + 1, iy, seed);
    let c = hash2(ix, iy + 1, seed);
    let d = hash2(ix + 1, iy + 1, seed);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

/// Fractal noise of `oct` octaves, 0..1 (centred on 0.5).
fn fbm(mut x: f32, mut y: f32, seed: u32, oct: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0f32, 0.5f32, 0.0f32);
    for i in 0..oct {
        sum += amp * vnoise(x, y, seed.wrapping_add(i * 31));
        norm += amp;
        amp *= 0.5;
        x = x * 2.03 + 17.1;
        y = y * 2.03 - 9.7;
    }
    sum / norm
}

#[inline]
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn mixf(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

#[inline]
fn tof(c: Rgb) -> [f32; 3] {
    [c.0 as f32, c.1 as f32, c.2 as f32]
}

// ---------------------------------------------------------------------------
// Sea: layers of waves under a sunset sky
// ---------------------------------------------------------------------------

/// Layers of waves, from the horizon (0) to the foreground.
const SEA_LAYERS: usize = 5;

impl Effect {
    /// A sea seen from the side: a sky (sunset, or a clear day in the light variant) with the
    /// sun on the horizon, and five layers of waves, each larger and darker than the one behind,
    /// with a lighter edge, foam on the nearest ones and the sun's glitter on the water.
    /// `level` is how much of the panel is sea (0-100); `react` (0-1) is the music: it makes
    /// the near waves taller and the sun brighter.
    pub(super) fn paint_sea(&mut self, fb: &mut Framebuffer, level: f32, scale: u32, t: f64, react: f32) {
        let (w, h) = (fb.width() as usize, fb.height() as usize);
        let g = scale.clamp(25, 400) as f32 / 100.0;
        let tf = (t % 100_000.0) as f32;
        let light = self.light;
        let k = self.bright;
        let hf = h as f32;
        let y_h = (hf * (1.0 - level.clamp(0.0, 100.0) / 100.0)).clamp(0.0, hf);

        // ----- the sky: one colour per row
        let (top, mid, hor): (Rgb, Rgb, Rgb) = if light {
            ((140, 195, 240), (190, 222, 246), (238, 247, 252))
        } else {
            ((8, 10, 36), (84, 36, 104), (255, 138, 66))
        };
        let tone = |c: Rgb| if light { c } else { dim(c, 0.30 + 0.70 * k) };
        let (top, mid, hor) = (tone(top), tone(mid), tone(hor));
        let sky_h = y_h.max(1.0);
        let sky: Vec<[f32; 3]> = (0..h)
            .map(|y| {
                let u = ((y as f32 + 0.5) / sky_h).clamp(0.0, 1.0);
                let c = if u < 0.55 {
                    lerp_rgb(top, mid, u / 0.55)
                } else {
                    lerp_rgb(mid, hor, ((u - 0.55) / 0.45).powf(1.5))
                };
                tof(c)
            })
            .collect();
        let hor_f = tof(lerp_rgb(hor, mid, 0.25));

        // ----- the sun
        let dim_px = (w as f32).min(hf * 2.0);
        let r_sun = (0.06 * dim_px * g.sqrt()).max(6.0);
        let (sx, sy) = (w as f32 * 0.66, y_h - 0.22 * r_sun);
        let sun_col: [f32; 3] = if light { [255.0, 250.0, 226.0] } else { tof(dim((255, 232, 160), 0.80 + 0.20 * k)) };
        let glow_k = (if light { 0.35 } else { 0.55 }) * (1.0 + 0.5 * react);

        // ----- the waves: crest line of every layer, column by column
        let span = (hf - y_h).max(1.0);
        let mut crest = vec![vec![0.0f32; w]; SEA_LAYERS];
        for (l, row) in crest.iter_mut().enumerate() {
            let f = l as f32 / (SEA_LAYERS - 1) as f32;
            let base = y_h + span * 0.80 * f.powf(1.35);
            let amp = (0.012 + 0.07 * f) * hf * g.sqrt() * (1.0 + 1.3 * react * f);
            let l1 = (70.0 + 160.0 * f) * g;
            let (l2, l3) = (l1 * 0.47, l1 * 0.23);
            let sp = 0.25 + 0.6 * f;
            let dir = if l % 2 == 0 { 1.0 } else { -1.0 };
            let (p1, p2, p3) = (dir * sp * tf * 1.3 + l as f32 * 1.7, dir * sp * tf * 1.9 + l as f32 * 0.6, dir * sp * tf * 2.7 + l as f32 * 3.1);
            for (x, v) in row.iter_mut().enumerate() {
                let x = x as f32;
                *v = base + amp * (0.6 * (x / l1 + p1).sin() + 0.3 * (x / l2 - p2).sin() + 0.1 * (x / l3 + p3).sin());
            }
        }
        // a nearer layer never rises above the one behind it, so every pixel belongs to one layer
        for l in 1..SEA_LAYERS {
            let (a, b) = crest.split_at_mut(l);
            for (v, back) in b[0].iter_mut().zip(a[l - 1].iter()) {
                *v = v.max(back + 1.5);
            }
        }

        // ----- per-row and per-column helpers for the glitter (so a pixel costs no trigonometry)
        let shim_row: Vec<f32> = (0..h).map(|y| (y as f32 * 0.5 - tf * 2.6).sin().max(0.0).powi(2)).collect();
        let shim_col: Vec<f32> = (0..w).map(|x| 0.5 + 0.5 * (x as f32 * 0.05 + tf).sin()).collect();
        let foam_col: Vec<f32> =
            (0..w).map(|x| (0.5 + 0.5 * (x as f32 * 0.31 + tf * 2.2).sin()) * (0.5 + 0.5 * (x as f32 * 0.077 - tf * 1.1).sin())).collect();

        // ----- paint
        let wl = &self.water_lut;
        let foam: [f32; 3] = [245.0, 250.0, 255.0];
        let edge = tof((self.edge[0], self.edge[1], self.edge[2]));
        let crest_light = {
            let c = wl[0];
            if light { edge } else { mixf(tof((c[0], c[1], c[2])), foam, 0.5) }
        };
        let px = fb.as_bytes_mut();
        // the sky (down to where the farthest waves can reach)
        let sky_rows = ((y_h + 0.012 * hf * g.sqrt() + 2.0).ceil() as usize).min(h);
        for y in 0..sky_rows {
            let c = sky[y];
            let dy = y as f32 + 0.5 - sy;
            let row = &mut px[y * w * 3..(y + 1) * w * 3];
            for (x, p) in row.chunks_exact_mut(3).enumerate() {
                let dx = x as f32 + 0.5 - sx;
                let d2 = dx * dx + dy * dy;
                let mut col = c;
                if d2 < 49.0 * r_sun * r_sun {
                    let d = d2.sqrt();
                    let glow = (-(d / (2.2 * r_sun)).powi(2)).exp() * glow_k;
                    col = mixf(col, sun_col, glow.min(1.0));
                    let disc = ((r_sun - d) / 1.5 + 0.5).clamp(0.0, 1.0);
                    if disc > 0.0 {
                        col = mixf(col, sun_col, disc);
                    }
                }
                p[0] = col[0].clamp(0.0, 255.0) as u8;
                p[1] = col[1].clamp(0.0, 255.0) as u8;
                p[2] = col[2].clamp(0.0, 255.0) as u8;
            }
        }
        // the water, layer by layer inside its own band of rows
        for x in 0..w {
            let dx = x as f32 + 0.5 - sx;
            for l in 0..SEA_LAYERS {
                let f = l as f32 / (SEA_LAYERS - 1) as f32;
                let y0 = crest[l][x];
                let y1 = if l + 1 < SEA_LAYERS { crest[l + 1][x] } else { hf };
                let (ya, yb) = ((y0.max(0.0)).floor() as usize, (y1.min(hf)).ceil() as usize);
                let haze = 0.85 * (1.0 - f).powf(1.6);
                let band = 2.0 + 7.0 * f;
                let foam_w = 2.0 + 6.0 * f;
                for y in ya..yb.min(h) {
                    let yf = y as f32 + 0.5;
                    let d = yf - y0;
                    if d < -1.0 {
                        continue;
                    }
                    let u = ((yf - y_h) / span).clamp(0.0, 1.0);
                    let b = wl[((u * 0.85 + 0.05) * (WATER_N - 1) as f32) as usize];
                    let mut c = [b[0] as f32, b[1] as f32, b[2] as f32];
                    if haze > 0.0 {
                        c = mixf(c, hor_f, haze * (1.0 - u));
                    }
                    if d < band {
                        let a = (1.0 - d.max(0.0) / band).powi(2);
                        c = mixf(c, crest_light, a * 0.55);
                    }
                    if l + 2 >= SEA_LAYERS && d < foam_w {
                        let a = (1.0 - d.max(0.0) / foam_w) * (0.35 + 0.65 * foam_col[x]);
                        c = if light { mixf(c, edge, a * 0.7) } else { mixf(c, foam, a * 0.9) };
                    }
                    // the sun's glitter: a column under the sun that widens towards the viewer
                    let sig = 10.0 + 0.22 * (yf - y_h);
                    if dx.abs() < 3.2 * sig {
                        let q = dx / sig;
                        let gl = 1.0 / ((1.0 + q * q) * (1.0 + q * q));
                        let amount = gl * (0.25 + 0.75 * shim_row[y] * shim_col[x]) * (0.9 - 0.5 * u) * (1.0 + 0.5 * react);
                        c = mixf(c, sun_col, (amount * 0.85).min(1.0));
                    }
                    let o = (y * w + x) * 3;
                    px[o] = c[0].clamp(0.0, 255.0) as u8;
                    px[o + 1] = c[1].clamp(0.0, 255.0) as u8;
                    px[o + 2] = c[2].clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ink: coloured clouds spreading in water
// ---------------------------------------------------------------------------

/// Pixels between two samples of the ink field.
const INK_CELL: usize = 6;

impl Effect {
    /// Clouds of ink in water: a warped fractal noise (the noise is bent by two other noises,
    /// which is what gives smoke and ink their curling arms). The ink is thick where the noise is
    /// high and fades into the water at its edge; its colour drifts along the palette. `t` is the
    /// time already multiplied by the speed; `react` (0-1) is the music: more ink, more curl.
    pub(super) fn paint_ink(&mut self, fb: &mut Framebuffer, scale: u32, t: f64, react: f32, phase: f32) {
        let (w, h) = (fb.width() as usize, fb.height() as usize);
        let unit = 300.0 * scale.clamp(25, 400) as f32 / 100.0;
        let tf = (t % 10_000.0) as f32 * 0.5 + phase;
        let (cols, rows) = (w / INK_CELL + 2, h / INK_CELL + 2);
        let n = cols * rows;
        let (mut dens, mut hue, mut rim) = (vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n]);
        let warp = 2.1 + 0.9 * react;
        let lo = 0.45 - 0.07 * react;
        for r in 0..rows {
            let y = (r * INK_CELL) as f32 / unit;
            for c in 0..cols {
                let x = (c * INK_CELL) as f32 / unit;
                let w1 = fbm(x * 1.3 + tf * 0.11, y * 1.3 - tf * 0.07, 11, 3);
                let w2 = fbm(x * 1.3 + 5.2 - tf * 0.05, y * 1.3 + 1.3 + tf * 0.09, 23, 3);
                let px_ = x + warp * (w1 - 0.5);
                let py_ = y + warp * (w2 - 0.5);
                let f = fbm(px_ * 1.6 + tf * 0.06, py_ * 1.6 - tf * 0.03, 37, 4);
                let i = r * cols + c;
                dens[i] = smoothstep(lo, lo + 0.17, f);
                hue[i] = w1 * 0.9 + f * 0.7 + tf * 0.012;
                rim[i] = (-((f - (lo + 0.07)) / 0.05).powi(2)).exp();
            }
        }
        // colours
        let light = self.light;
        let base = self.rain_lut[0];
        let ground: [f32; 3] =
            if light { tof((self.paper[0], self.paper[1], self.paper[2])) } else { [base[0] as f32 / 14.0, base[1] as f32 / 14.0, base[2] as f32 / 14.0] };
        let rim_k = if light { 0.35 } else { 0.55 };
        let gain = 1.0 + 0.45 * react;
        let lut = &self.rain_lut;
        let ground_u8 = [ground[0] as u8, ground[1] as u8, ground[2] as u8];
        let px = fb.as_bytes_mut();
        // horizontal sample position of every pixel
        let xi: Vec<(usize, f32)> = (0..w).map(|x| (x / INK_CELL, (x % INK_CELL) as f32 / INK_CELL as f32)).collect();
        let (mut rd, mut rh, mut rr) = (vec![0.0f32; cols], vec![0.0f32; cols], vec![0.0f32; cols]);
        for y in 0..h {
            let (r, wy) = (y / INK_CELL, (y % INK_CELL) as f32 / INK_CELL as f32);
            let (a, b) = (r * cols, (r + 1) * cols);
            for c in 0..cols {
                rd[c] = dens[a + c] + (dens[b + c] - dens[a + c]) * wy;
                rh[c] = hue[a + c] + (hue[b + c] - hue[a + c]) * wy;
                rr[c] = rim[a + c] + (rim[b + c] - rim[a + c]) * wy;
            }
            let row = &mut px[y * w * 3..(y + 1) * w * 3];
            for (x, p) in row.chunks_exact_mut(3).enumerate() {
                let (c, wx) = xi[x];
                let d = rd[c] + (rd[c + 1] - rd[c]) * wx;
                let u = rh[c] + (rh[c + 1] - rh[c]) * wx;
                let e = rr[c] + (rr[c + 1] - rr[c]) * wx;
                if d < 0.003 && e < 0.01 {
                    // plain water: no ink here
                    p[0] = ground_u8[0];
                    p[1] = ground_u8[1];
                    p[2] = ground_u8[2];
                    continue;
                }
                // `u` is always positive (the noise and the time only add), so no floor is needed
                let idx = (((u + 64.0) * RAIN_N as f32) as i32 & (RAIN_N as i32 - 1)) as usize;
                let ink = lut[idx];
                let ink = [ink[0] as f32, ink[1] as f32, ink[2] as f32];
                // thicker ink is a little brighter (dark variant) or darker (light variant)
                let body = if light { 1.0 - 0.25 * d } else { (0.80 + 0.45 * d) * gain };
                let col = [ink[0] * body, ink[1] * body, ink[2] * body];
                let mut out = mixf(ground, col, d);
                // a thin bright (dark variant) or dark (light variant) edge of the same colour
                let edge_c = if light { [ink[0] * 0.45, ink[1] * 0.45, ink[2] * 0.45] } else { [ink[0] * 0.6 + 140.0, ink[1] * 0.6 + 140.0, ink[2] * 0.6 + 140.0] };
                out = mixf(out, edge_c, (e * rim_k).min(1.0));
                p[0] = out[0].clamp(0.0, 255.0) as u8;
                p[1] = out[1].clamp(0.0, 255.0) as u8;
                p[2] = out[2].clamp(0.0, 255.0) as u8;
            }
        }
    }
}
