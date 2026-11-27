# All the features

[← Back to the README](../README.md) · [All the docs](./README.md)

- EQ bars (48) from currently playing audio (WASAPI loopback on Windows,
  PulseAudio/PipeWire on Linux), colored green→yellow→red — or any of 9 color
  palettes and 5 drawing styles.
- System info: CPU %, real-time CPU frequency, RAM, uptime, clock & date.
- **Settings editor** (`trofeo_config`): a small graphical program to create and edit
  `trofeo.conf` — every option in sections, with a **live preview drawn by the real
  program**. It only edits the file: it is not needed (and not in memory) while the display
  runs (see [Settings editor](../README.md#settings-editor-trofeo_config)).
- **Music screen**: a Spotify-style now-playing card (album cover, artist/title,
  progress bar, spectrum, colors taken from the cover).
- **Two fonts for every text**: the classic blocky pixel font, or smooth anti-aliased
  **Poppins** in all screens (`font = smooth`) — see [Font](CONFIGURATION.md#font).
- **Pixel game** under the clock: a self-playing pixel-art platformer (a knight
  or a cat) that reacts to CPU load, music, network/disk bursts, the hour, and
  the real weather and time of day.
- **Tiles layout**: a modern glass dashboard (rounded tiles with big numbers, bars and
  rings) where you pick the tiles, the font (smooth or pixel), the bar colors and the
  background — and, if you like, a **color of its own for every tile, taken from the color
  presets**, drifting and fading from preset to preset (see
  [Tiles layout](CONFIGURATION.md#tiles-layout-glass-dashboard)).
- **Fan speeds** from [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor):
  RPM of the CPU fan, the case fans, the pump and the GPU on the tiles, in the status line and on
  the **DeepCool case display** (frequency and RPM in turn, `deepcool_mode = alternate`) — see
  [Fans and sensors](CONFIGURATION.md#fans-and-sensors-librehardwaremonitor).
- **Deck, Cards, Orbit and Console**: four more full screens with graphs of the last minutes —
  instrument columns, colourful cards, concentric rings and a console-style row of tiles with a
  focus that moves (see [Deck, Cards, Orbit and Console](CONFIGURATION.md#deck-cards-orbit-and-console)).
- **Pi-hole statistics** (version 6): queries, blocked, share blocked and domains on the block lists on seven screens (`pihole`, `pihole-cards`, `pihole-ring`, `pihole-console`, `pihole-minimal`, `pihole-light`, `pihole-flow`), as a `pihole` panel of deck / cards / orbit / console and a tile of tiles, and as `{pihole_...}` texts and `value=pihole_percent` bars in your own layouts. It logs in once and uses the same session again (it logs in again only on a 401), refreshes every `pihole_interval` seconds (15) on a thread of its own, and address and password are settings, not code. See [the options guide](../OPTIONS_GUIDE.md#pi-hole).
- **A spectrum in your layouts** (`spectrum` element): a row, a column or a circle; `channel=left|right|mix`; `band=bass|mid|treble`; mirrored (`sym`), upside down (`flip`); styles `bars`, `led`, `peaks`, `area`, `mirror`; your own colours, gradients that flow and a hot colour. Gain (`spectrum_gain`), compressor (`spectrum_compress`), fixed or automatic level (`spectrum_auto_gain`) and the audio device to listen to (`audio_device`: an output, an input or a PulseAudio source). See [the options guide](../OPTIONS_GUIDE.md#the-spectrum-in-a-layout).
- **Flowing colours on the classic screens**: `dash_flow` (orbit, deck, console, cards) and `tiles_flow` (tiles), off by default.
- **Thirty-three built-in free layouts**, nine of them with bars, rings or spectrums whose gradient flows (`bar_animation`): `pulse`, `radial`, `stereo`, `bands`, `swiss` with bars and rings whose gradient flows (`bar_animation`): `swiss`, `dusk`, `aurora`, `carbon`, `eclipse`, `ledger`, `haiku`, `peach`, `atelier`, `noir`, `pearl`, `volt`, `blaze`, `studio`, `board`, `mineral`, `cluster`, `halo`, `gauges`, `terminal`, `horizon` and `ribbon` (the first four with a music spectrum), ready to use with
  `layout = atelier` and to copy in the layout builder (see [the options guide](../OPTIONS_GUIDE.md#built-in-designs)).
- **Gaming screens and the Steam cover flow**: nine designs for the in-game screen (arena, zen, cockpit, tactical, neon,
  telemetry, quest, arcade, classic) with the name of the game and facts about it, and `layout = steam`, a cover flow
  of your Steam library (see [Gaming screens](CONFIGURATION.md#gaming-screens) and [Steam cover flow](CONFIGURATION.md#steam-cover-flow)).
- **Digital picture frame**: `layout = frame` turns the LCD into a photo frame — a collage of
  the pictures of a folder, either a tidy **mosaic** or scattered **polaroid** prints, one
  picture changing at a time over a blurred copy of the photo, with slim glass tiles (clock,
  CPU, GPU, RAM, ...) at the sides, at almost no CPU cost (see
  [Digital frame](CONFIGURATION.md#digital-frame-picture-collage)).
- **Backgrounds**: an image, GIF or video; a **solid color** (any color name or hex code);
  or an **animated effect** with no file needed — plasma, aurora, lava lamp, silk waves, a
  minimal **matrix / digital rain**, an **aquarium** seen from the side with a moving water
  surface, light rays, caustics and bubbles, a **network** of nodes and thin lines, or a turning
  **globe**. Rain, aquarium, network and globe also come in a **white (light) variant**. Colors from pastel to fluorescent, or your own; adjustable speed
  and brightness, at almost no CPU cost (see
  [Backgrounds](CONFIGURATION.md#backgrounds-solid-color-and-animated-effects)).
- Can also drive a **DeepCool** display (sends CPU data over HID).
- **Second monitor mode** (`trofeo_screen`): the LCD becomes a real second
  monitor.
- Current **weather** (temperature + a stylized condition icon) and a
  **7-day forecast strip**, auto-detected from your IP or a city you set.
- **News ticker**: RSS/Atom feeds, any plain-text URL, or a local file — as a
  scrolling line on top of any screen, and/or its own full panel.
- Sync EQ bar color with an **OpenRGB** device.
- Adaptive FPS: drops to idle when silent → saves CPU.
