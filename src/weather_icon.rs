//! Small stylized weather icons: an 8x8 pixel bitmap per condition (same
//! blit technique as the bitmap font in `font.rs`), single color. Not meant
//! to be photorealistic — just enough shape to read at a glance next to the
//! condition text, in the same blocky/retro style as the rest of the UI.

use crate::Framebuffer;

pub const ICON_SIZE: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherIcon {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

/// Maps an Open-Meteo / WMO weather code to an icon.
/// <https://open-meteo.com/en/docs> (WMO Weather interpretation codes table).
pub fn icon_for_code(code: u16) -> WeatherIcon {
    match code {
        0 => WeatherIcon::Clear,
        1 | 2 => WeatherIcon::PartlyCloudy,
        3 => WeatherIcon::Cloudy,
        45 | 48 => WeatherIcon::Fog,
        51 | 53 | 55 | 56 | 57 => WeatherIcon::Drizzle,
        61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => WeatherIcon::Rain,
        71 | 73 | 75 | 77 | 85 | 86 => WeatherIcon::Snow,
        95 | 96 | 99 => WeatherIcon::Thunder,
        _ => WeatherIcon::Cloudy,
    }
}

/// Index matching the order of `i18n::Strings::weather_conditions`.
fn index(icon: WeatherIcon) -> usize {
    match icon {
        WeatherIcon::Clear => 0,
        WeatherIcon::PartlyCloudy => 1,
        WeatherIcon::Cloudy => 2,
        WeatherIcon::Fog => 3,
        WeatherIcon::Drizzle => 4,
        WeatherIcon::Rain => 5,
        WeatherIcon::Snow => 6,
        WeatherIcon::Thunder => 7,
    }
}

/// Localized condition name ("CLEAR" / "SERENO", etc.), in the current UI language.
pub fn condition_name(icon: WeatherIcon) -> &'static str {
    crate::i18n::t().weather_conditions[index(icon)]
}

/// A reasonable default color per condition (used unless overridden).
pub fn default_color(icon: WeatherIcon) -> (u8, u8, u8) {
    match icon {
        WeatherIcon::Clear => (0xFF, 0xC8, 0x00),
        WeatherIcon::PartlyCloudy => (0xE0, 0xC0, 0x60),
        WeatherIcon::Cloudy => (0xA0, 0xA0, 0xA8),
        WeatherIcon::Fog => (0x90, 0x98, 0xA0),
        WeatherIcon::Drizzle => (0x60, 0xB0, 0xE0),
        WeatherIcon::Rain => (0x30, 0x90, 0xE0),
        WeatherIcon::Snow => (0xF0, 0xF0, 0xF8),
        WeatherIcon::Thunder => (0xF0, 0xC0, 0x20),
    }
}

fn bitmap(icon: WeatherIcon) -> [u8; 8] {
    match icon {
        // A little sunburst: filled square body + N/S/E/W and diagonal single-pixel rays.
        WeatherIcon::Clear => [
            0b00011000,
            0b01000010,
            0b00111100,
            0b10111101,
            0b10111101,
            0b00111100,
            0b01000010,
            0b00011000,
        ],
        // Sun peeking out from behind a cloud.
        WeatherIcon::PartlyCloudy => [
            0b00000110,
            0b00001100,
            0b00011110,
            0b00111100,
            0b01111110,
            0b11111111,
            0b01111110,
            0b00000000,
        ],
        WeatherIcon::Cloudy => [
            0b00000000,
            0b00111000,
            0b01111100,
            0b11111110,
            0b11111111,
            0b01111110,
            0b00000000,
            0b00000000,
        ],
        // Horizontal fog bands.
        WeatherIcon::Fog => [
            0b00000000,
            0b00000000,
            0b01111110,
            0b00000000,
            0b11111111,
            0b00000000,
            0b01111110,
            0b00000000,
        ],
        // Cloud with a couple of light drizzle dots below.
        WeatherIcon::Drizzle => [
            0b00111000,
            0b01111100,
            0b11111110,
            0b11111111,
            0b01111100,
            0b00000000,
            0b00100010,
            0b00000000,
        ],
        // Cloud with heavier, staggered rain streaks.
        WeatherIcon::Rain => [
            0b00111000,
            0b01111100,
            0b11111110,
            0b11111111,
            0b01111100,
            0b01010100,
            0b00101010,
            0b00000000,
        ],
        // Cloud with scattered snowflakes.
        WeatherIcon::Snow => [
            0b00111000,
            0b01111100,
            0b11111110,
            0b11111111,
            0b01111100,
            0b01000100,
            0b00101000,
            0b00000000,
        ],
        // Cloud with a small lightning bolt.
        WeatherIcon::Thunder => [
            0b00111000,
            0b01111100,
            0b11111110,
            0b11111111,
            0b01111100,
            0b00011000,
            0b00110000,
            0b00000000,
        ],
    }
}

/// Draws the icon at `(x, y)`, each bitmap pixel scaled to `scale` device pixels.
pub fn draw(fb: &mut Framebuffer, x: u32, y: u32, scale: u32, icon: WeatherIcon, r: u8, g: u8, b: u8) {
    let scale = scale.max(1);
    for (row_idx, bits) in bitmap(icon).iter().enumerate() {
        for col in 0..ICON_SIZE {
            let bit = ICON_SIZE - 1 - col;
            if (bits >> bit) & 1 == 1 {
                fb.fill_rect(x + col * scale, y + row_idx as u32 * scale, scale, scale, r, g, b);
            }
        }
    }
}

pub fn size(scale: u32) -> u32 {
    ICON_SIZE * scale.max(1)
}
