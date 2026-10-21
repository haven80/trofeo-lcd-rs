# Notice

This project is a **fork** of [sukualam/trofeo-lcd](https://github.com/sukualam/trofeo-lcd),
which in turn is based on the byte-for-byte protocol reverse-engineering done
in [thermalright-trcc-linux](https://github.com/Lexonight1/thermalright-trcc-linux)
by Lexonight1.

It is distributed, like the projects above, under the
**[GNU GPL v3.0-or-later](./LICENSE)**. Modifying and redistributing this
software is welcome and encouraged under the terms of that license — this
file exists only to keep the provenance of the code clear, not to add any
restriction beyond the GPL itself.

## What changed compared to upstream

Starting from the `sukualam/trofeo-lcd` codebase, this fork has:

- Translated the entire source tree, comments and user-facing strings from
  Indonesian to English (the UI itself is separately selectable between
  English, Italian, German, Spanish, French and Portuguese via
  `language = en | it | de | es | fr | pt`).
- Added a **weather module**: current temperature + a stylized pixel-art
  condition icon, via Open-Meteo, with automatic IP-based geolocation and an
  optional fixed-city override (`weather_city`), shown as a per-item line
  and/or as its own full-panel layout (`layout = weather`) — which also
  includes a **7-day forecast strip** (day, icon, high/low, rain probability).
- Added **per-item styling**: every status item (CPU, GPU, RAM, clock,
  weather, ...) can be given its own size, position, color and backdrop
  independently (`<item>_size`, `<item>_position`, `<item>_color`,
  `<item>_backdrop`), instead of one shared style for the whole info block.
- Added **live config hot-reload**: `trofeo.conf` is re-read every second and
  changes apply immediately, without restarting the program (a small, clearly
  documented set of options still require a restart).
- Added **per-layout rotation duration**: when several preset layouts rotate
  in sequence, each one can be given its own number of seconds on screen
  (`layout = default:30, weather:5`) instead of splitting the time equally.
- Added new preset panel layouts, panel transparency (`panel_opacity`) and
  text backdrops (`text_backdrop`) for readability over a background image.
- Added a **news ticker**: RSS/Atom feeds, any plain-text URL, or a local
  file, shown as a scrolling chyron on top of any screen and/or its own full
  panel (`layout = news`).
- Added **spectrum styles and palettes**: five drawing styles (classic bars,
  LED segments, peak caps, filled wave, mirror), seven gradient presets plus a
  scrolling rainbow and custom gradients, with optional timed rotation of both,
  and `layout_hold_on_music` to keep the first layout on screen while music plays.
- Added a **music screen**: a Spotify-style now-playing card (album cover with
  glow, artist/title/album, progress bar, side spectrum, colors taken from the
  cover, and a configurable background: gradient, blurred cover, flat color or none, and a selectable footer of CPU/GPU/RAM stats, and its own spectrum color: from the cover or fixed) fed by the Windows media controls or
  `playerctl` on Linux.
- Added a **pixel-art mini game** under the clock: a self-playing platformer (original
  knight and cat heroes) whose speed, jumps, coins, fireworks, sky and weather react to CPU
  load, music, network/disk activity, the clock and the weather module.
- Added **animated background effects** (plasma, aurora, lava lamp, waves) as an
  alternative to an image: colors that blend and drift slowly, with 12 color presets
  (pastel to fluorescent) or your own, adjustable speed, size and brightness, and a
  very low CPU cost (a coarse field stretched through a color table); also available
  behind the music screen.
- Added a **solid background color** (`background_color`, a name such as `black`,
  `white`, `pink` or a hex code) and two minimal animated **digital rain** backgrounds
  (`matrix`, `rain`) with green, amber, violet, crimson and white presets; the list of
  color names understood by every color option was extended.
- Added an **aquarium** background (`background_effect = aquarium`): a side view of
  the water with a moving, clearly visible surface, light rays, caustics and bubbles; the
  water level is adjustable (`effect_level`) and there are three water color presets.
- Added a **light (white) variant** of the matrix, rain and aquarium backgrounds
  (`effect_theme = light`).
- Added a **font option for every text** (`font = pixel | smooth`, `font_weight`): the anti-aliased
  Poppins font, until now only in the tiles layout, can be used in all screens (status lines, clock,
  layouts, music screen, ticker, weather), with smooth edges over any background.
- Added **network** and **globe** backgrounds (`background_effect = network | globe`): nodes joined
  by thin lines, drifting on a plane or turning on a sphere, in dark and light variants, with the
  same color palettes as the other effects.
- Added a **digital picture frame** layout (`layout = frame`): a collage of the pictures of a
  folder, as a tidy mosaic or as scattered polaroid prints, one picture changing at a time over a
  blurred copy of the photo, with glass tiles at the sides. Pictures are decoded with the
  [image](https://github.com/image-rs/image) crate (MIT or Apache-2.0; JPEG, PNG, WebP, BMP, GIF)
  on a background thread.
- Added **per-tile colors from the color presets** to the tiles layout (`tiles_palette`): every tile
  has a color of its own, drifting along the preset and fading from preset to preset.
- Added **fan speeds** from [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)
  (MPL-2.0; only its web server is used, nothing of its code): CPU, case, pump and GPU RPM on the tiles, the
  status line, the new screens and the DeepCool case display (`deepcool_mode`), with a small JSON reader of
  our own.
- Added three more screens, **deck**, **cards** and **orbit** (`layout = deck | cards | orbit`), with
  graphs of the last minutes, drawn with the same pieces as the tiles.
- Added a **graphical settings editor** (`trofeo_config`, built with egui): a separate
  small program that only creates and edits `trofeo.conf` (comments preserved), with a live
  preview drawn by `trofeo_lcd` itself (`--preview-png`). It uses the
  [egui/eframe](https://github.com/emilk/egui) GUI library (MIT or Apache-2.0) and, on
  Windows, [rfd](https://github.com/PolyMeilex/rfd) (MIT) for the file dialogs.
- Added an **OpenRGB** sync option, a global **screenshot hotkey**
  (lossless PNG), a **console-hide** option, and an adaptive idle/active FPS
  system.
- Added a comprehensive **[options reference guide](./OPTIONS_GUIDE.md)**
  documenting every config-file key and command-line flag.
- General cleanup, additional unit tests, and packaging changes for Windows
  releases.

None of this would exist without the original reverse-engineering and
implementation work in the two upstream projects — thank you to their
authors.
