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
