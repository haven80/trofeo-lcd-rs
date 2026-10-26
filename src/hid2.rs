//! The "HID Type 2" Thermalright LCD family (USB `0416:5302`): the **Trofeo Vision 6.86**
//! (1280x480) and, with the same protocol, other Vision screens.
//!
//! The protocol comes from the open-source `thermalright-trcc-linux` project
//! (GPL-3.0, `adapters/device/hid_lcd.py` and `doc/PROTOCOL_USBLCDNEW.md`), which in turn
//! documents what the vendor's Windows program (TRCC / USBLCDNEW) sends. **It has not been
//! verified on real hardware by this project** — that is why the program prints what the
//! screen answers and has a test pattern (`--lcd-test`) to find the right orientation.
//!
//! Wire summary:
//! - Handshake: 512 bytes `DA DB DC DD` + zeros, command `01` at byte 12. The reply starts
//!   with the same magic, has `01` at byte 12 (and `10` at byte 16); byte 5 is the PM (model),
//!   byte 4 the SUB (variant), bytes 20..36 a serial.
//! - Frame: a 20-byte header + the JPEG, padded to a multiple of 512 bytes and written as
//!   512-byte reports (the firmware latches a frame only when it arrives in such pieces).
//!   Header: `DA DB DC DD | 02 00 | 00 00 | width LE16 | height LE16 | 02 00 00 00 | length LE32`.
//!   (`00 00` at bytes 6-7 means JPEG; `01 00` is the RGB565 mode of the small 320x240 screens,
//!   which this program does not use.)

use crate::Resolution;

pub const VENDOR_ID: u16 = 0x0416;
pub const PRODUCT_ID: u16 = 0x5302;

pub const MAGIC: [u8; 4] = [0xDA, 0xDB, 0xDC, 0xDD];
pub const REPORT_SIZE: usize = 512;
pub const HEADER_SIZE: usize = 20;

/// The handshake request: magic, command 1 at byte 12, zero-padded to 512 bytes.
pub fn init_packet() -> Vec<u8> {
    let mut p = vec![0u8; REPORT_SIZE];
    p[..4].copy_from_slice(&MAGIC);
    p[12] = 0x01;
    p
}

/// What the screen answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub pm: u8,
    pub sub: u8,
    pub serial: String,
}

/// Check and read the handshake reply. `None` when it is not a Type 2 reply.
pub fn parse_reply(resp: &[u8]) -> Option<Identity> {
    if resp.len() < 20 || resp[..4] != MAGIC || resp[12] != 0x01 {
        return None;
    }
    let serial = if resp.len() >= 36 && resp[16] == 0x10 {
        resp[20..36].iter().map(|b| format!("{b:02X}")).collect()
    } else {
        String::new()
    };
    Some(Identity { pm: resp[5], sub: resp[4], serial })
}

/// The picture size a model (PM byte) shows, from the vendor's table. `None` = unknown model.
/// The 6.86" Trofeo Vision answers PM 128 (seen in other projects' logs); the vendor table
/// lists 68 for a 1280x480 panel too.
pub fn resolution_for(pm: u8, sub: u8) -> Option<Resolution> {
    let (w, h) = match pm {
        5 | 50 => (320, 240),
        7 | 14 => (640, 480),
        9 | 11 => (854, 480),
        10 | 16 => (960, 540),
        12 => (800, 480),
        13 | 17 => (960, 320),
        15 => (640, 172),
        32 => (320, 320),
        63 | 64 => (1600, 720),
        65 | 66 => (1920, 462),
        68 | 128 => (1280, 480),
        69 => (1920, 440),
        1 if sub == 48 => (1600, 720),
        1 if sub == 49 => (1920, 462),
        _ => return None,
    };
    Some(Resolution::new(w, h))
}

/// Models that take RGB565 instead of JPEG: not supported by this program.
pub fn needs_rgb565(pm: u8) -> bool {
    matches!(pm, 5 | 32 | 50)
}

/// How the panel is mounted, from the SUB byte: the 6.86" with SUB = 2 wants the picture
/// turned by 90° (other projects found the landscape/portrait swapped without it). The right
/// direction can differ from unit to unit: `hid_mount` overrides it, `--lcd-test` finds it.
pub fn default_mount(pm: u8, sub: u8) -> crate::Rotation {
    match (resolution_for(pm, sub), sub) {
        (Some(r), 2) if r == Resolution::new(1280, 480) => crate::Rotation::R90,
        _ => crate::Rotation::R0,
    }
}

/// The 20-byte frame header for a JPEG of `len` bytes whose picture is `width`x`height`.
pub fn frame_header(width: u16, height: u16, len: u32) -> [u8; HEADER_SIZE] {
    let mut h = [0u8; HEADER_SIZE];
    h[..4].copy_from_slice(&MAGIC);
    h[4] = 0x02; // command: picture
    // h[6..8] = 0: JPEG
    h[8..10].copy_from_slice(&width.to_le_bytes());
    h[10..12].copy_from_slice(&height.to_le_bytes());
    h[12] = 0x02;
    h[16..20].copy_from_slice(&len.to_le_bytes());
    h
}

/// Header + JPEG, zero-padded to a multiple of 512 bytes.
pub fn build_frame(width: u16, height: u16, jpeg: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(HEADER_SIZE + jpeg.len() + REPORT_SIZE);
    f.extend_from_slice(&frame_header(width, height, jpeg.len() as u32));
    f.extend_from_slice(jpeg);
    let padded = f.len().div_ceil(REPORT_SIZE) * REPORT_SIZE;
    f.resize(padded, 0);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handshake_request_is_512_bytes_with_the_magic_and_the_command() {
        let p = init_packet();
        assert_eq!(p.len(), 512);
        assert_eq!(&p[..4], &[0xDA, 0xDB, 0xDC, 0xDD]);
        assert_eq!(p[12], 1);
        assert!(p[4..12].iter().all(|&b| b == 0) && p[13..].iter().all(|&b| b == 0));
    }

    #[test]
    fn the_reply_is_checked_and_read() {
        let mut r = vec![0u8; 512];
        assert_eq!(parse_reply(&r), None, "no magic");
        r[..4].copy_from_slice(&MAGIC);
        assert_eq!(parse_reply(&r), None, "no command echo");
        r[12] = 1;
        r[4] = 2;
        r[5] = 128;
        assert_eq!(parse_reply(&r), Some(Identity { pm: 128, sub: 2, serial: String::new() }));
        r[16] = 0x10;
        r[20] = 0xAB;
        let id = parse_reply(&r).unwrap();
        assert!(id.serial.starts_with("AB") && id.serial.len() == 32);
        assert_eq!(parse_reply(&r[..19]), None);
    }

    #[test]
    fn the_models_map_to_their_size() {
        assert_eq!(resolution_for(128, 1), Some(Resolution::new(1280, 480)));
        assert_eq!(resolution_for(68, 0), Some(Resolution::new(1280, 480)));
        assert_eq!(resolution_for(65, 5), Some(Resolution::new(1920, 462)));
        assert_eq!(resolution_for(1, 48), Some(Resolution::new(1600, 720)));
        assert_eq!(resolution_for(1, 7), None);
        assert_eq!(resolution_for(99, 0), None);
        assert!(needs_rgb565(32) && !needs_rgb565(128));
    }

    #[test]
    fn the_mount_follows_the_sub_byte_of_the_6_86() {
        assert_eq!(default_mount(128, 2), crate::Rotation::R90);
        assert_eq!(default_mount(128, 1), crate::Rotation::R0);
        assert_eq!(default_mount(65, 2), crate::Rotation::R0);
    }

    #[test]
    fn the_frame_has_the_20_byte_header_and_is_a_multiple_of_512() {
        let jpeg = vec![0xFFu8; 1000];
        let f = build_frame(1280, 480, &jpeg);
        assert_eq!(f.len(), 1024);
        assert_eq!(&f[..4], &MAGIC);
        assert_eq!(f[4], 2);
        assert_eq!(&f[6..8], &[0, 0]);
        assert_eq!(u16::from_le_bytes([f[8], f[9]]), 1280);
        assert_eq!(u16::from_le_bytes([f[10], f[11]]), 480);
        assert_eq!(f[12], 2);
        assert_eq!(u32::from_le_bytes([f[16], f[17], f[18], f[19]]), 1000);
        assert_eq!(&f[20..1020], &jpeg[..]);
        assert!(f[1020..].iter().all(|&b| b == 0));
        // an exact multiple is not padded further
        assert_eq!(build_frame(1, 1, &vec![0u8; 492]).len(), 512);
    }

    #[test]
    fn the_header_matches_the_capture_of_the_320x240_screen_apart_from_the_mode() {
        // From another project's capture: dadbdcdd 0200 0100 f000 4001 0200 0000 00580200 ...
        // (RGB565 mode `01 00`, 240 x 320, 153600 bytes). Ours differs only in the mode bytes.
        let ours = frame_header(240, 320, 153_600);
        let capture: [u8; 20] = [0xDA, 0xDB, 0xDC, 0xDD, 2, 0, 1, 0, 0xF0, 0, 0x40, 1, 2, 0, 0, 0, 0, 0x58, 2, 0];
        for i in 0..20 {
            if i != 6 {
                assert_eq!(ours[i], capture[i], "byte {i}");
            }
        }
    }
}
