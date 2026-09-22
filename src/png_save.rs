//! Save the contents of a `Framebuffer` as a **PNG** (lossless) file with
//! deflate compression via the `flate2` crate (pure-Rust miniz_oxide backend
//! — no C dependency). Screenshots are taken via hotkey (rare), so the best
//! compression level is used; a flat background (EQ bar, system info)
//! typically shrinks from ~2.6 MB raw down to a few hundred KB. It's
//! lightweight on its own and only active when the hotkey is pressed (see
//! src/hotkey.rs).

use std::io;
use std::io::Write;
use std::path::PathBuf;

use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::Framebuffer;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Encode `pixels` (RGB888, width*height*3 bytes) as PNG data.
fn png_encode(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let row_len = (width * 3) as usize;

    // Raw PNG scanlines: each row starts with a filter byte 0 (None), then RGB.
    let mut raw = Vec::with_capacity(height as usize * (row_len + 1));
    for y in 0..height {
        raw.push(0);
        let start = (y as usize) * row_len;
        raw.extend_from_slice(&pixels[start..start + row_len]);
    }

    let mut out = Vec::with_capacity(raw.len() + raw.len() / 64 + 64);
    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR: width, height, bit depth 8, color type 2 (RGB), etc.
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(2); // color type: RGB
    ihdr.push(0); // compression: zlib
    ihdr.push(0); // filter: adaptive
    ihdr.push(0); // interlace: none
    push_chunk(&mut out, b"IHDR", &ihdr);

    // Best compression level: screenshots are taken rarely, so speed
    // doesn't matter — the smallest file size does.
    push_chunk(&mut out, b"IDAT", &zlib_stream(&raw));

    push_chunk(&mut out, b"IEND", &[]);
    out
}

/// Write a single PNG chunk: length (BE) + type + data + CRC32(type+data).
fn push_chunk(out: &mut Vec<u8>, ctype: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ctype);
    out.extend_from_slice(data);
    let mut crc = crc_update(0xFFFF_FFFF, ctype);
    crc = crc_update(crc, data);
    out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
}

/// CRC-32 (polynomial 0xEDB88320, same as zlib). Bit-by-bit implementation —
/// plenty fast for infrequent screencaps.
fn crc_update(mut crc: u32, data: &[u8]) -> u32 {
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// Wrap `data` as a zlib stream (header + deflate + adler-32 checksum)
/// with the best compression level.
fn zlib_stream(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data)
        .expect("writing to ZlibEncoder::new(Vec) cannot fail");
    enc.finish()
        .expect("finishing ZlibEncoder::new(Vec) cannot fail")
}

/// Encode `fb` as PNG data.
pub fn encode(fb: &Framebuffer) -> Vec<u8> {
    png_encode(fb.as_bytes(), fb.width(), fb.height())
}

/// Save the framebuffer contents as a PNG in the **Desktop** folder with the
/// name `{prefix}_YYYYMMDD_HHMMSS.png`, then return the full path.
///
/// The Desktop location is obtained from the official Windows API
/// (SHGetKnownFolderPath → FOLDERID_Desktop) so it stays correct even if the
/// Desktop is relocated by OneDrive or redirected; on other OSes it uses
/// `$HOME/Desktop`.
pub fn save(fb: &Framebuffer, prefix: &str) -> io::Result<PathBuf> {
    let dir = desktop_dir()?;
    std::fs::create_dir_all(&dir)?;
    let stamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let path = dir.join(format!("{}_{}.png", prefix, stamp));
    std::fs::write(&path, encode(fb))?;
    Ok(path)
}

/// Path to the user's Desktop folder.
#[cfg(windows)]
fn desktop_dir() -> io::Result<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath};

    unsafe {
        let pw = SHGetKnownFolderPath(&FOLDERID_Desktop, KNOWN_FOLDER_FLAG(0), None)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("SHGetKnownFolderPath(Desktop) failed: {e}")))?;

        // pw is a PWSTR from CoTaskMemAlloc — copy it into a String first,
        // then CoTaskMemFree.
        let wide: &[u16] = pw.as_wide();
        let dir = PathBuf::from(String::from_utf16_lossy(wide));

        CoTaskMemFree(Some(pw.as_ptr() as *const core::ffi::c_void));
        Ok(dir)
    }
}

/// Path to the Desktop folder (non-Windows fallback).
#[cfg(not(windows))]
fn desktop_dir() -> io::Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|e| {
        io::Error::new(io::ErrorKind::NotFound, format!("HOME environment variable is not set: {e}"))
    })?;
    Ok(PathBuf::from(home).join("Desktop"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Framebuffer;
    use crate::Resolution;

    #[test]
    fn desktop_dir_points_to_existing_folder() {
        let dir = desktop_dir().expect("desktop_dir must succeed");
        assert_eq!(dir.file_name().and_then(|s| s.to_str()), Some("Desktop"));
        assert!(dir.is_dir(), "path {dir:?} must be an existing folder");
    }

    #[test]
    fn png_signature_ihdr_dimensions_iend() {
        let fb = Framebuffer::new(Resolution::new(1, 1));
        let w = fb.width();
        let h = fb.height();
        let png = encode(&fb);
        assert_eq!(&png[..8], &PNG_SIGNATURE);
        // IHDR starts at byte 8: length(4) + "IHDR".
        assert_eq!(&png[12..16], b"IHDR");
        let ihdr_w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let ihdr_h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((ihdr_w, ihdr_h), (w, h));
        // Must end with IEND.
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    /// Write a sample PNG to target/ to validate against an external
    /// decoder (manual test via System.Drawing PowerShell).
    #[test]
    fn write_sample_png_for_external_validation() {
        let mut fb = Framebuffer::new(Resolution::new(16, 8));
        let px = fb.as_bytes_mut();
        for (i, byte) in px.iter_mut().enumerate() {
            *byte = (i * 7) as u8; // deterministic pattern
        }
        let out = encode(&fb);
        std::fs::write(std::path::Path::new("target/png_test_sample.png"), out)
            .expect("write sample PNG");
    }

    /// A flat background (visualizer/solid-fill screen) must compress well
    /// below the ~2.6 MB raw size at 1920x462 — this is the whole point of
    /// using deflate.
    #[test]
    fn flat_screen_compresses_well() {
        let mut fb = Framebuffer::new(Resolution::new(1920, 462));
        let px = fb.as_bytes_mut();
        for (i, byte) in px.iter_mut().enumerate() {
            // Quasi-solid fill: dark green RGB, with a few bright speckles
            // of variation to keep it a "legitimate" image.
            *byte = if i % 97 == 0 { 0x30 } else { 0x12 };
        }
        let raw = px.len();
        let png = encode(&fb);
        std::fs::write(std::path::Path::new("target/png_test_flat.png"), &png)
            .expect("write sample PNG");
        assert!(
            png.len() * 10 < raw,
            "PNG too large: {} bytes vs raw {} bytes",
            png.len(),
            raw
        );
    }
}