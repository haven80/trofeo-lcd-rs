# Building and code structure

[← Back to the README](../README.md) · [All the docs](./README.md)

## Building

```bash
cargo build --release          # builds trofeo_lcd, trofeo_screen and trofeo_config
cargo test --release           # the tests (they also redraw the pictures in img/)
```

On Linux it needs `pkg-config` and the PulseAudio headers. Windows builds are made by the
[release workflow](../.github/workflows/release.yml) when a tag such as `v2.18.0` (lowercase `v`) is pushed.

## Code structure

- `src/lib.rs` — USB driver: handshake, chunking, JPEG encode, `Framebuffer`.
- `src/audio.rs` — audio capture (WASAPI / PulseAudio-PipeWire).
- `src/cpu_sensor.rs`, `src/cpu_freq.rs` — CPU temp/power & frequency.
- `src/deepcool/` — DeepCool display drivers (HID).
- `src/dxgi_capture.rs` + `src/bin/screen.rs` — second monitor mode.
- `src/weather.rs` + `src/weather_icon.rs` — weather (Open-Meteo) and its condition icons.
- `src/ticker.rs` — news ticker: RSS/Atom feeds, plain-text URLs, or a local file.
- `src/spectrum.rs` — spectrum styles and color palettes.
- `src/media.rs` + `src/music_screen.rs` — now-playing info (Windows media controls / `playerctl`) and the Spotify-style music screen.
- `src/pixel_game.rs` — the self-playing pixel-art platformer under the clock.
- `src/effect.rs` — the animated backgrounds (plasma, aurora, lava, waves, matrix, rain, aquarium, network, globe and their light variants).
- `src/frame.rs` — the digital picture frame (folder scan, picture loader thread, mosaic and polaroid collages, transitions).
- `src/tiles.rs` — the tiles layout; `src/smooth_font.rs` + `assets/` — the anti-aliased text, used by the tiles and (`font = smooth`) by every other text (embedded Poppins font, SIL OFL: see `assets/OFL.txt`).
- `src/bin/config_editor.rs` — the settings editor window (`trofeo_config`, built with egui).
- `src/schema.rs` — the list of every setting the editor shows (names, ranges, defaults, texts in English and Italian); tests check it against the real parser, so the editor can only write what the program accepts.
- `src/conf_edit.rs` — line-by-line editing of `trofeo.conf` that keeps comments and untouched lines.
- `src/hid2.rs` — the HID family (Trofeo Vision 6.86): handshake and frames.
- `src/fade.rs` — the fade between screens.
- `src/themes.rs` — the themes: the built-in ones, and the ones of your own (`themes/<name>.theme`: save, list, load, export, checks).
- `src/free.rs` — free layouts: the file format (read, write, check), the placeholders and metrics.
- `src/free_draw.rs` — draws a free layout (text, bar, ring, rect, image) with live values.
- `src/bin/config_editor/builder.rs` — the visual builder of free layouts in the settings editor.
- `src/profiles.rs` — profiles: the files, the choice (rule, hand, file), the rules and the hold, the hotkey order.
- `src/alerts.rs` — threshold alerts: the rules, the delay and hysteresis, the border and bar.
- `src/share.rs` — "Share your design": the request file, the settings of the look, the gallery page.
- `src/tray.rs` — the tray icon (Windows).
- `src/layouts.rs`, `src/tiles/` — the screens (tiles, deck/cards/orbit/console, gaming, music, Steam).
- `src/main.rs` — main loop: audio → FFT → EQ bars → send to screen (and `--preview-png`, which draws one frame for the editor's preview).
