# Options guide

Every setting trofeo_lcd supports, in one place, with examples. For a shorter,
inline-commented version you can copy and edit directly, see
[`trofeo.conf.example`](./trofeo.conf.example).

## Settings editor (`trofeo_config`)

If you prefer clicking to typing, `trofeo_config` (`trofeo_config.exe` on Windows) edits
the same file for you.

- **Where to put it**: in the same folder as `trofeo_lcd.exe` (and `trofeo.conf`). It
  opens `trofeo.conf` from its own folder; to edit another file, start it with the path
  as its first argument, or use *Open...* in the window.
- **The preview** at the top is drawn by `trofeo_lcd` itself (it needs
  `trofeo_lcd.exe` next to the editor), with sample data (clock, CPU/GPU values, a song,
  ...), so backgrounds, effects, layouts, tiles, colors and fonts look exactly as on the
  display. Choose *Nothing playing* / *Music playing (spectrum)* / *Music card* to see the other
  states, *Animate* to see the movement, and the layout selector to see each screen
  of a rotation.
- **Sections** on the left cover everything in this guide. An asterisk marks the settings
  that need a restart of `trofeo_lcd`; a number shows how many settings of that section
  you changed. A setting you leave at its default is not written to the file; the small
  `x` next to a setting removes it from the file again. *Raw file* shows the text of the
  file, for anything the form does not cover.
- **Saving** keeps your comments and every line the editor does not touch, writes the
  changes it makes under a "Added by the settings editor" comment (or in place, when the
  key is already in the file), and copies the old file to `trofeo.conf.bak` the first time you save in each session.
  A running `trofeo_lcd` applies the saved changes within a second (hot reload).
- If the preview shows a red message, that is the real program refusing a value, in its
  own words; nothing is saved until you press *Save*.
- The editor is not resident: close it when you are done, `trofeo_lcd` does not need it.
  On Linux the file dialogs are replaced by a text box with the path.

## How settings work

- **Config file**: copy `trofeo.conf.example` to `trofeo.conf`, next to the
  executable (or in the folder you launch it from), or point to any file with
  `--config <FILE>`. `trofeo.config` also works, if you prefer that name.
- **Priority**: built-in defaults < `trofeo.conf` < command-line arguments.
  A command-line flag always wins over the config file.
- **Every `key = value` line has a matching CLI flag**: replace `_` with `-`
  and add `--` in front (`text_color = red` ↔ `--text-color red`). This guide
  gives the config-file form; assume the CLI form exists unless noted. The
  reverse isn't always true: a handful of flags (screenshot hotkey, OpenRGB
  sync, `hide_console`, the FPS/silence tuning flags, `deepcool_update_ms`)
  are command-line-only — each is marked below.
- **Live reload**: `trofeo.conf` is re-read once a second. Save the file and
  changes apply immediately — no restart needed — *except* for the handful of
  options listed in [Requires a restart](#requires-a-restart). If a saved file
  has an error, the previous (working) settings stay active, and the reason is
  written to `trofeo-errors.txt` next to the executable (useful with
  `hide_console = true`, where you can't see the console).
- **Comments and quoting**: `#` starts a comment, except when it begins a hex
  color: at the start of a value (`cpu_color = #FFC800`) or right after a comma
  in a list (`spectrum_gradient = #FF0080, #00FFFF`). Values can be
  quoted (`orientation = "portrait"`) or not — quotes are optional and
  stripped either way.
- **Booleans** accept `true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`.

### Minimal example

```ini
# trofeo.conf
orientation = landscape
language = en
text_color = #FFFFFF
clock_color = #00C8FF
brightness = 90
```

That alone is a valid file — every option not mentioned just keeps its
default.

## Requires a restart

Of the options that actually live in `trofeo.conf`, everything reloads live
*except* `deepcool`, `fps_monitor`, `weather_city`, `ticker_source` and
`ticker_refresh_min` — change one of these in the file and the program tells
you on the next reload that a restart is needed, while the rest of the file
still applies live.

Separately, a handful of options only exist as command-line flags at all
(`--openrgb-device`, `--openrgb-poll-ms`, `--hide-console`, `-k`/
`--screenshot-key`, `--idle-fps`, `--active-fps`, `--deepcool-update-ms`) —
since they're never read from `trofeo.conf`, the only way to change them is
to relaunch the program with different arguments. Each is marked
"command-line only" below. `--silence-threshold`/`--silence-timeout-ms` used
to be command-line only too, but are now also `trofeo.conf` keys
(`silence_threshold`, `silence_timeout_ms`) that apply live, like everything
else in the file — the CLI flags still work and take priority if both are set.

## Orientation, flip, margins

| Key | CLI | Values | Default |
|---|---|---|---|
| `orientation` | `--orientation` | `landscape`, `portrait` | `landscape` |
| `flip` | `--flip` | `true`/`false` | `false` |
| `margin` | `--margin <PX>` | integer px, all sides | `0` |
| `margin_top`/`margin_bottom`/`margin_left`/`margin_right` | `--margin-top` etc. | integer px, one side (overrides `margin` for that side) | `0` |

`portrait` draws the UI on a 462×1920 canvas (the panel mounted upright) instead
of the native 1920×462. `flip` adds a further 180° on top of either orientation
— use it if the image comes out upside down.

```ini
orientation = portrait
flip = true          # upright, but rotated 180° (upside down mount)
margin_top = 30       # push everything down 30px (e.g. to clear a bezel)
```

## Language

| Key | CLI | Values | Default |
|---|---|---|---|
| `language` | `--language` | `en`, `it`, `de`, `es`, `fr`, `pt` | `en` |

Only the on-screen text (weekday/month names, labels like "NOW PLAYING",
weather conditions) follows this. Console/log output is always in English.
The weather city-search (geocoding) also prefers place names in the chosen
language. Since the bitmap font has no accented letters, each translation is
written accent-free: German spells out umlauts (`AE`/`OE`/`UE`) and `ß` as
`SS`; Italian, Spanish, French and Portuguese simply drop the accent (Italian
keeps its existing apostrophe-for-accent day names, e.g. `Lunedi'`).

```ini
language = it
# language = de   # German
# language = es   # Spanish
# language = fr   # French
# language = pt   # Portuguese
```

## Background

| Key | CLI | Values | Default |
|---|---|---|---|
| `background` | `--background <FILE>` | path to an image (jpg/png/bmp), animated GIF, or video | none |
| `background_color` | `--background-color` | a solid color: a name (`black`, `white`, `green`, `pink`, …), `#RRGGBB` or `R,G,B` | none (dark navy) |
| `background_dim` | `--background-dim` | `0`-`100` | `40` |
| `ffmpeg` | `--ffmpeg <PATH>` | path to `ffmpeg.exe` | next to the exe, then `PATH` |
| `background_fit` | `--background-fit` | `cover`, `stretch`, `contain`, `original` | `cover` |
| `background_position` | `--background-position` | a [position](#positions) | `center` |
| `background_offset_x`/`background_offset_y` | `--background-offset-x/-y` | integer px (+ = right/down) | `0` |

**Solid color.** `background_color` fills the whole panel with one flat color
(behind the margins, the panels and everything else). Names are not case sensitive,
spaces / `-` / `_` are ignored (`dark green` = `darkgreen`) and Italian and Indonesian
names work too. Available names: `black`, `white`, `gray`, `lightgray`, `darkgray`,
`silver`, `red`, `darkred`, `green`, `darkgreen`, `lightgreen`, `lime`, `olive`, `blue`,
`navy`, `lightblue`, `skyblue`, `yellow`, `gold`, `cyan`, `teal`, `turquoise`, `magenta`,
`pink` (pale pink), `hotpink`, `salmon`, `coral`, `orange`, `brown`, `beige`, `purple`,
`violet`, `indigo`, `lavender`. For anything else use hex: `#1B2A4A`. An image, GIF or
video (`background`) or an animated effect, if you also set one, is shown instead.
The same names and hex codes work for every color option (`text_color`, `clock_color`,
`tiles_bg_color`, …).

Video backgrounds need ffmpeg; `background_dim` darkens the image so text
stays readable over it. With an animated background (GIF or video) the idle
FPS is automatically raised to the active FPS, so the animation stays smooth
even while nothing else is happening. For a soft moving background that needs
no file at all see [Animated background effect](#animated-background-effect-plasma-aurora-lava-waves-matrix-rain-aquarium-network-globe).

```ini
background = C:\Pictures\space.mp4
background_dim = 55
background_fit = cover
ffmpeg = C:\ffmpeg\bin\ffmpeg.exe

# or a flat color instead of a picture:
# background_color = black        # a name ...
# background_color = #0B1D3A      # ... or a hex code
```

## Animated background effect (plasma, aurora, lava, waves, matrix, rain, aquarium, network, globe)

Instead of a picture, the background can be a slow, soft effect whose colors
blend into each other and drift: a plasma, curtains of aurora, a lava lamp or
silk-like waves — a minimal **digital rain** (`matrix`, `rain`) an **aquarium** seen from the side, a **network** of nodes and lines or a turning **globe**. It is drawn behind everything (clock, status lines, spectrum,
the pixel game, layouts), costs almost no CPU, and you choose the colors, the
speed and the brightness.

| Key | CLI | Values | Default |
|---|---|---|---|
| `background_effect` | `--background-effect` | `plasma` / `aurora` / `lava` / `waves` / `matrix` / `rain` / `aquarium` / `network` / `globe` / `none` | `none` |
| `effect_palette` | `--effect-palette` | `pastel` / `fluo` / `neon` / `candy` / `sunset` / `ocean` / `aurora` / `fire` / `forest` / `ice` / `mono` / `rainbow` / `custom`; for `matrix` / `rain` also `matrix` (green) / `amber` / `violet` / `crimson` / `snow` (white); for `aquarium` also `lagoon` / `deep` / `tropical`; `network` and `globe` use `ice` unless you choose; several (comma-separated) or `all` rotate | `neon` |
| `effect_colors` | `--effect-colors` | 2 or more colors, e.g. `#FF0080, #00FFFF` | — |
| `effect_palette_interval` | `--effect-palette-interval` | `5`-`3600` seconds (rotation period) | `60` |
| `effect_speed` | `--effect-speed` | `0` (frozen) - `100` (fast) | `30` |
| `effect_scale` | `--effect-scale` | `25`-`400` % (size of the features; bigger = larger, calmer shapes) | `100` |
| `effect_brightness` | `--effect-brightness` | `0`-`100` | `40` |
| `effect_fps` | `--effect-fps` | `1`-`30` | `8` |
| `effect_theme` | `--effect-theme` | `dark` / `light` (`matrix`, `rain`, `aquarium`, `network` and `globe`: a white background instead of a dark one; the other styles ignore it) | `dark` |
| `effect_level` | `--effect-level` | `0`-`100` % (`aquarium`: how high the water reaches, `0` empty, `100` full; `network` and `globe`: how many nodes, `0` few, `100` many) | `50` |

- **Styles** (`background_effect`): `plasma` is the classic demoscene look (a
  sum of waves and a moving ring); `aurora` is undulating bands of light; `lava`
  is soft blobs that drift and merge; `waves` is smooth diagonal ripples (silk).
  `matrix` is a minimal digital rain: columns of characters (digits and letters)
  falling at different speeds, each with a bright head and a fading tail, on a dark
  background. `rain` is the same idea reduced to the bare minimum: thin streaks of
  light with no characters. Both are sparse (well under a quarter of the panel is lit),
  so text stays readable, and they cost less than the soft styles. `effect_scale`
  sets the size of the characters / thickness of the streaks, `effect_speed` how fast
  they fall (`30` is a calm rain).
- **Aquarium** (`background_effect = aquarium`): the panel becomes the glass of an
  aquarium seen from the side. A moving, slightly irregular **water surface** with a
  bright edge separates the dark air from the water, so you always see where the water
  ends; under it the water darkens with depth, soft **rays of light** slant down from the
  surface, a fine net of **caustics** shimmers near the top and a few **bubbles** rise and
  fade as they reach the surface. `effect_level` decides how much of the panel is water
  (`50` = half; `80` = nearly full with the surface in the top fifth; `0` = empty). The
  surface works in portrait as well. `effect_speed` is the speed of the waves and bubbles,
  `effect_scale` makes waves, bubbles and light patterns larger (and calmer).
  Colors: `lagoon` (turquoise, the default), `deep` (blue to navy), `tropical`
  (green-turquoise); any other palette works too — its lightest color is the surface and
  its darkest the bottom, whatever order the colors are written in (so `ocean`, `ice` or your own
  `effect_colors = #FFD28A, #7A2E00` give water of those colors). If you do not set
  `effect_brightness`, the aquarium uses `65` instead of `40` (water needs more light to look
  like water). Cost: about 5 ms per frame, a bit more than the soft styles; lower
  `effect_fps` to 5 if you want it lighter.
- **Network** (`background_effect = network`): points drifting slowly on small orbits, joined by
  thin lines while they are near each other, so the net keeps changing shape; the lines fade as they
  stretch. A few nodes have a ring around them. There is no text, only lines and dots.
  `effect_level` is the number of nodes (`50` is about one every 120 pixels), `effect_scale` the
  spacing and size of everything (bigger = fewer, larger nodes), `effect_speed` how fast they drift.
- **Globe** (`background_effect = globe`): the same idea on a sphere that turns slowly and tilts a
  little, with the near side clearer than the far one and a faint outline. It sits in the middle of
  the panel; `effect_scale` makes it larger (above 100 it grows beyond the top and bottom edges)
  or smaller, `effect_level` the number of nodes.
- **Colors of network and globe**: lines take a grey tinted by the colors of the nodes they join,
  and the nodes are drawn in the palette's colors. The default is `ice` (blue-grey, like a
  blueprint); a one-colour palette such as `crimson` gives grey lines with red nodes, and a
  several-colour one (`neon`, `rainbow`) gives nodes of different colors. If you set no
  `effect_brightness` they use `70`. Cost: about 1.5 ms (network) and 3 ms (globe) per frame.
- **Light (white) variants** (`effect_theme = light`): `matrix` and `rain` become dark
  characters / streaks on a white page (the tails fade into the white; a faint tint of the
  palette is added to it), and the `aquarium` becomes a white room with pale water that gets
  deeper in color towards the bottom, a dark edge on the surface so it still stands out, white
  rays and caustics. The same palettes are used (`matrix` green, `amber`, `violet`, `crimson`,
  `lagoon`, `deep`, `tropical`, …), and `effect_brightness` now sets how saturated they are
  (higher = stronger colors; `65` by default in the light variants, `40` for the dark
  `matrix` / `rain`). Remember to give the text on top a dark color (`clock_color`, `text_color`).

  ```ini
  background_effect = aquarium
  effect_theme = light
  effect_palette = lagoon
  clock_color = #14323C
  text_color = #14323C
  ```
- **Rain colors**: if you set no palette, `matrix` and `rain` are green. One-colour
  presets made for them: `matrix` (green, also `green` / `verde`), `amber` (old
  terminal), `violet`, `crimson` and `snow` (white). Any other palette works too
  (`neon` gives multi-colour streaks), as do your own `effect_colors` (two or more). These five presets are not part of `all`.
- **Colors** (`effect_palette`): `pastel` soft light tones, `fluo` highlighter
  colors (hot pink, yellow, green, cyan, violet), `neon` magenta / violet / blue /
  cyan, `candy` sweet pinks and mints, `sunset`, `ocean`, `aurora` (greens and
  violet), `fire`, `forest`, `ice`, `mono` (silvery grey) and `rainbow` (the whole
  hue circle). Several names rotate with a smooth cross-fade every
  `effect_palette_interval` seconds, and `all` goes through every preset.
- **Your own colors**: `effect_colors = #FF0080, #00FFFF, #FFD000` (two or more,
  `#RRGGBB` or color names) makes a looping palette of its own. On its own it is
  used directly; write `custom` in the `effect_palette` list to rotate it with the
  presets (`effect_palette = pastel, custom`).
- **Speed**: `30` is a slow drift (a full change takes about a minute); `0` freezes
  the picture; `100` is lively. A calm speed looks smooth even at a few frames per second.
- **Brightness** (`effect_brightness`, 0-100): how strong the colors are. Lower it
  to keep text readable — dark palettes (`ocean`, `aurora`, `fire`, `sunset`,
  `neon`) read well at 35-55. Light palettes such as `pastel` and `candy` look
  best brighter (70-85) with a dark text color instead:

  ```ini
  background_effect = plasma
  effect_palette = pastel
  effect_brightness = 80
  clock_color = #28243C
  text_color = #28243C
  ```
- It replaces `background` (an image or video you also set is not used while an
  effect is on); `background_dim` does not apply, `effect_brightness` does the
  same job. It works with `margin`, `panel_opacity` and `text_backdrop` (panels
  and backdrops blend over it) and in portrait orientation.
- **Music screen**: `music_bg = effect` paints the same effect behind the album
  cover (style, palette, speed and size from the keys above, plasma if no style
  is set), with `music_bg_brightness` as its brightness. The cover and the text stay
  on top. `music_bg = none` instead lets the global effect show through.

**CPU cost.** The picture is only calculated at about 6 000 points (one every 12
pixels) and stretched to the whole panel through a color table, so building a
frame takes around a millisecond. The rest of the work — JPEG-encoding and sending
the frame — is the same as for any other frame; it is the **number of frames** that
matters, which is why the effect has its own `effect_fps` (8 by default,
`1`-`30`). While an effect is moving, the idle screen runs at that rate instead of
2 fps; with the pixel game on it runs at the active FPS anyway. To be as light as
possible, lower `effect_fps` (4-5 is still smooth at the default speed) or set
`effect_speed = 0`, which freezes the picture and keeps the idle screen at its
normal 2 fps. A fast `effect_speed` needs a higher `effect_fps` to look smooth.

```ini
# minimal digital rain, amber like an old terminal
background_effect = matrix
effect_palette = amber

# a network of nodes and lines (or: globe), on white, grey lines with red nodes
# background_effect = network
# effect_theme = light
# effect_palette = crimson
# clock_color = #202833
# text_color = #202833

# an aquarium, water up to 60% of the height
# background_effect = aquarium
# effect_palette = lagoon
# effect_level = 60

background_effect = aurora
effect_palette = ocean, aurora, neon     # rotates, cross-fading
effect_palette_interval = 90
effect_speed = 25
effect_brightness = 45
effect_fps = 6
```

## What to show — `show` / `hide`

| Key | CLI | Values | Default |
|---|---|---|---|
| `show` | `--show <LIST>` | comma-separated element names — show **only** these | everything on |
| `hide` | `--hide <LIST>` | comma-separated element names — turn these **off** | nothing hidden |

Element names: `cpu, gpu, uptime, time, date, mem` (or `ram`)`, net, disk,
volume, nowplaying, weather, spectrum, ticker, clock, clock_date, dashboard`.

Detail fields inside the CPU/GPU lines can also be individually hidden:
`cpu_freq, cpu_temp, cpu_power, gpu_temp, gpu_power, gpu_fan, gpu_clock, gpu_fps`, and the fan speeds
of [LibreHardwareMonitor](#fans-and-sensors-librehardwaremonitor) in the CPU line: `cpu_fan, case_fan, pump`.

`show` and `hide` can combine: `show` first restricts to a subset, `hide` then
removes from what's left.

```ini
# Only these four, nothing else:
show = cpu, mem, clock, nowplaying

# Everything except these two:
hide = disk, volume

# Keep the big clock visible during playback (normally the spectrum takes over):
hide = spectrum

# Hide GPU fan speed and clock from the CPU/GPU detail line:
hide = gpu_fan, gpu_clock
```

## Colors

| Key | CLI | Applies to |
|---|---|---|
| `color` | `--color` | EQ spectrum bars |
| `text_color` | `--text-color` | info block + dashboard text |
| `clock_color` | `--clock-color` | the big clock |

Accepted formats: `default` (spectrum only — green→yellow→red gradient),
a hex code `#RRGGBB`, `R,G,B` (0-255 each), or a name: `red, green, blue,
yellow, cyan, magenta, white, orange, purple`.

```ini
color = default          # gradient spectrum
text_color = #E0E0E0
clock_color = orange
```

## Font

| Key | CLI | Values | Default |
|---|---|---|---|
| `font` | `--font` | `pixel` (the blocky 5x7 font) / `smooth` (anti-aliased Poppins) | `pixel` |
| `font_weight` | `--font-weight` | `auto` / `light` / `regular` / `medium` (only for `smooth`) | `auto` |

`font = smooth` changes the font of **every** text of the program: the status lines, the big
clock and the date, all the preset layouts, the music screen, the ticker, the weather panel and
the game's coin counter. Capital letters keep the same height as in the pixel font, so each
screen keeps its proportions, but the text is mixed case ("Thursday, 15 October 2026"), has
accents and the degree sign, and its edges are smooth also over a picture or an animated
background. `font_weight = auto` uses a stronger weight (medium) for small text and a lighter
one for big text such as the clock; the other values force one weight everywhere. With
`smooth`, the lines of the status block are a little farther apart, to leave room for letters
such as g, p and y.

The `tiles` layout is not affected: it has its own `tiles_font` (`smooth` or `pixel`, smooth by
default). The characters of the `matrix` background and the pixel game's drawings are always
pixel art.

```ini
font = smooth
# font_weight = medium
```

## Spectrum styles and colors

The EQ spectrum (shown while music plays) can be drawn in several styles and
with several color palettes. All keys apply live (no restart) and are also CLI
flags (`--spectrum-style led`, ...). With none of them set, the spectrum looks
exactly as it always did.

| Key | CLI | Values | Default |
|---|---|---|---|
| `spectrum_style` | `--spectrum-style` | `bars`, `led`, `peaks`, `area`, `mirror`, a comma list, or `all` | `bars` |
| `spectrum_style_interval` | `--spectrum-style-interval` | `2`-`3600` seconds | `30` |
| `spectrum_palette` | `--spectrum-palette` | `default`, `rainbow`, `fire`, `ocean`, `sunset`, `neon`, `ice`, `matrix`, `purple`, `custom`, a comma list, or `all` | unset (uses `color`) |
| `spectrum_palette_interval` | `--spectrum-palette-interval` | `2`-`3600` seconds | `20` |
| `spectrum_gradient` | `--spectrum-gradient` | 2 or more colors, comma-separated | none |
| `spectrum_rainbow_speed` | `--spectrum-rainbow-speed` | `0`-`360` degrees/second | `30` |

**Styles**

- `bars` — the classic solid bars.
- `led` — bars made of separate LED-like segments, with dim "ghost" segments
  above the lit ones (hi-fi VU look).
- `peaks` — solid bars plus a small cap that holds at the loudest recent
  level, then falls slowly.
- `area` — a smooth filled wave with a bright outline.
- `mirror` — bars growing up *and* down from a centre line.

One style stays fixed. Several (`spectrum_style = led, area, mirror`) or `all`
rotate every `spectrum_style_interval` seconds.

**Palettes**

- `default` — the original green → yellow → red gradient by level.
- `rainbow` — hue by position across the bars, scrolling over time
  (`spectrum_rainbow_speed`; `0` keeps it still).
- `fire`, `ocean`, `sunset`, `neon`, `ice`, `matrix`, `purple` — ready-made
  gradients from quiet (bottom/low) to loud (top/high).
- `custom` — your own gradient from `spectrum_gradient`. Colors are hex
  (`#FF0080`) or names (`red, orange, ...`), at least 2, spread evenly from
  quiet to loud. Setting `spectrum_gradient` without `spectrum_palette` uses
  it directly.

One palette stays fixed. Several (`spectrum_palette = fire, ocean, neon`) or
`all` (every preset, including the rainbow) rotate every
`spectrum_palette_interval` seconds, with a 2-second cross-fade between them.
A palette overrides `color` (and the OpenRGB color) for the spectrum only;
`color` still applies to everything else it did before.

```ini
spectrum_style = led
spectrum_palette = neon

# Rotate everything: a new look every 30s, new colors every 20s
spectrum_style = all
spectrum_palette = all

# Your own colors, hot pink to cyan
spectrum_gradient = #FF0080, #00FFFF

# Slow rainbow, mirrored
spectrum_style = mirror
spectrum_palette = rainbow
spectrum_rainbow_speed = 10
```

## Music screen (now-playing card)

| Key | CLI | Values | Default |
|---|---|---|---|
| `music_screen` | `--music-screen` | `true`/`false` | `false` |
| `music_bg` | `--music-bg` | `gradient` / `blur` / `color` / `solid` / `none` / `effect` | `gradient` |
| `music_bg_brightness` | `--music-bg-brightness` | `0`-`100` | `40` |
| `music_bg_blur` | `--music-bg-blur` | `0`-`100` (only `blur`) | `70` |
| `music_bg_fit` | `--music-bg-fit` | `center` / `stretch` (only `blur`) | `center` |
| `music_bg_color` | `--music-bg-color` | color (only `solid`) | `#121218` |
| `music_progress` | `--music-progress` | `true`/`false` | `true` |
| `music_spectrum_color` | `--music-spectrum-color` | `cover` / `main` / a color / several colors / preset names / `all` | `cover` |
| `music_stats` | `--music-stats` | list of `cpu`, `gpu`, `ram`, `net`, `disk`, `time`, `uptime`; or `all` / `none` | `cpu, gpu, ram` |

With `music_screen = true`, while a track is playing the whole panel becomes a
Spotify-style card: the **album cover** on the left (rounded, with a soft glow),
**artist, title and album** next to it, a **progress bar with times**, and the
**spectrum on the side**. The background follows the cover (see below) and
the accent color (artist line, progress bar, spectrum) is taken
from the cover, so every album gets its own look. Titles too long to fit scroll.
In portrait orientation the cover goes on top, the text below it and the
spectrum at the bottom. When the music stops the normal screen/layout comes back.

- The card replaces the status lines and the ticker while it is shown (it is a
  full screen of its own). The in-game dashboard still wins while you are gaming.
- **Spectrum color** (`music_spectrum_color`), separate from the main screen's
  `spectrum_palette`:
  - `cover` (default) — tinted with the cover's accent color, so every album looks different.
  - a **fixed color**, `music_spectrum_color = #1DB954` (or a name such as `cyan`) — flat, the same for every track.
  - **two or more colors**, `music_spectrum_color = #FF0080, #00FFFF` — a gradient, quiet to loud.
  - **preset names**, `music_spectrum_color = fire` or `fire, ocean, neon` (or `all`) — the same
    presets as `spectrum_palette`; several rotate every `spectrum_palette_interval` seconds.
  - `main` — use the main screen's `spectrum_palette` / `spectrum_gradient` (the cover color
    if those are not set). This is how version 1.0.33 and earlier behaved.

  Presets and colors cannot be mixed in one list. `purple` is a preset; for the plain
  color write `#800080`. The spectrum *style* (`spectrum_style`) is still shared with the main screen.
- **Background** (`music_bg`):
  - `gradient` (default) — the cover's dominant color, brightest next to the
    cover and fading to near-black toward the spectrum (the Spotify look).
  - `blur` — the cover itself, blurred. `music_bg_blur` sets how much (0 = barely,
    100 = a soft color wash). `music_bg_fit = center` takes a centred slice of the
    cover with the panel's proportions; `stretch` squeezes the whole cover to the
    panel (the look of versions up to 1.0.31).
  - `color` — one flat color, the cover's dominant one.
  - `solid` — one flat color of your choice, `music_bg_color` (`#RRGGBB`, `R,G,B`
    or a name), the same for every track.
  - `none` — keep whatever is behind (solid color, your `background` image or the
    [background effect](#animated-background-effect-plasma-aurora-lava-waves-matrix-rain-aquarium-network-globe));
    the cover and text are drawn on top.
  - `effect` — the animated background effect (plasma, aurora, lava, waves, matrix, rain, aquarium, network or globe, with
    the style, palette, speed and size you set in its section; plasma if you set none),
    drawn just for this screen. `music_bg_brightness` is its brightness.

  `music_bg_brightness` (0-100) is how strong the cover color is in `gradient`,
  `blur` and `color` (lower = darker, more contrast for the text; it does not
  affect `solid`). The glow around the cover is kept in every mode except `none` and `effect`.
  The old `music_blur = false` still works (it means `music_bg = none`) when
  `music_bg` is not set. Without a cover, `blur` falls back to the gradient.
- **Footer** (`music_stats`): a small line of system info in the strip at the very
  bottom, e.g. `CPU 23% 54C   GPU 61% 48C   MEM 12000/32000MB`. Pick what you want and
  the order: `music_stats = cpu, gpu, ram, net`; `all` shows everything (also `disk`,
  `time` and `uptime`); `none` turns it off. Temperatures appear when the sensors
  provide them, units follow `net_unit` / `mem_unit`, and an item with no data
  (e.g. no GPU reading) is skipped. If the line is too wide for the panel (portrait
  mode, long lists) it shrinks to the small font and then drops the last items.
- `music_progress = false` hides the bar and times. They only appear when the
  player reports the track length; some players don't.
- If the player provides no cover, a stylised record is drawn instead.
- **Windows**: cover, title and position come from the system media controls
  (the same source as the NOW PLAYING line), so any player that shows up in the
  Windows volume/media pop-up works — Spotify, browsers, most players.
- **Linux**: needs `playerctl` (as for NOW PLAYING); covers are loaded from the
  player's `file://` or `https://` art URL.
- The font only has uppercase letters without accents, so titles appear in capitals.

```ini
music_screen = true
# optional: the background and the spectrum look
music_bg = gradient          # gradient | blur | color | solid | none
music_bg_brightness = 40
music_stats = cpu, gpu, ram, net     # footer line; none = off
music_spectrum_color = cover         # cover | main | #RRGGBB | fire, ocean...
spectrum_style = mirror
spectrum_palette = neon

# a softer blurred cover instead of the gradient
# music_bg = blur
# music_bg_blur = 60
# music_bg_brightness = 45
# music_bg_fit = center
```

## Pixel game (mini platformer under the clock)

| Key | CLI | Values | Default |
|---|---|---|---|
| `pixel_game` | `--pixel-game` | `true`/`false` | `true` |
| `pixel_game_hero` | `--pixel-game-hero` | `knight` / `cat` / `both` | `both` |
| `pixel_game_height` | `--pixel-game-height` | `20`-`70` (% of the area) | `46` |
| `pixel_game_hud` | `--pixel-game-hud` | `true`/`false` | `true` |

On the **default screen** (and on `default2`, the one with per-item styling) a strip of
pixel-art game is drawn **under the clock**: the clock shrinks to make room, and the game
sits just above the info line when that is at the bottom (below it when the info is on
top). A little knight or cat runs along a scrolling landscape and jumps over rocks, bushes,
crates, slimes and holes by itself. It never shows on the preset layout panels, the music
screen or the in-game dashboard, and it needs the clock to be shown (`hide = clock` turns it off too).

The world reacts to what your computer is doing:

- **CPU load** sets the running speed; above 80 % the hero sprints and kicks up dust.
- **Music**: while sound plays the hero hops on the bass and little notes float up.
  (On the default screen the spectrum then takes the clock's place above the game.)
- **Network bursts** drop a row of coins to collect; **disk bursts** drop a treasure chest.
  The counter in the top-left corner (`pixel_game_hud`) shows the coins collected since start.
- **On the hour** there are fireworks and a victory dance.
- **The sky follows the real time**: dawn, day with sun and birds, dusk, night with moon and stars.
- **The weather** (the same data as the `weather` item) decides sun / partly cloudy / overcast,
  rain, snow, fog and thunderstorms with lightning. Without weather data the sky is clear.
- The knight cuts slimes down when running slowly; the cat just jumps. With `both`
  they take turns every 5 minutes (with a little puff of smoke).

`pixel_game_height` is the share of the free area the strip takes: more makes the game
bigger and the clock smaller. If the area is too short for it (a very tall info block, a
tiny panel) the game is simply not drawn.

**Frame rate:** an animation needs smooth frames, so while the game is on screen the
idle screen runs at `--active-fps` (15 by default) instead of `--idle-fps` (2). Drawing the
game itself is cheap (about 0.3 ms per frame); the cost is JPEG-encoding and sending more
frames to the panel. If you prefer the lowest possible load, set `pixel_game = false`.

```ini
pixel_game = true
pixel_game_hero = cat         # knight | cat | both
pixel_game_height = 50
pixel_game_hud = false
```

## Positions

Anywhere a "position" is accepted (`status_position`, `clock_position`,
`spectrum_position`, `background_position`, and every `<item>_position`),
it's one of a 3×3 grid:

```
top-left      top      top-right
center-left   center   center-right
bottom-left   bottom   bottom-right
```

| Key | CLI | Default |
|---|---|---|
| `status_position` | `--status-position` | `top-left` |
| `clock_position` | `--clock-position` | `center` |
| `spectrum_position` | `--spectrum-position` | `center` |
| `spectrum_width`/`spectrum_height` | `--spectrum-width/-height` | `100` — `1`-`100`, % of the available area |

```ini
status_position = bottom-left
clock_position = top-right
spectrum_position = center
spectrum_width = 70
spectrum_height = 60
```

## Info block style

| Key | CLI | Values | Default |
|---|---|---|---|
| `status_style` | `--status-style` | `auto`, `lines`, `list`, `items` | `auto` |

`lines` packs everything into up to 3 long lines (classic landscape look);
`list` is a short vertical list (handy in portrait, or when pinned to a
corner); `items` is per-item mode. `auto` picks `lines`/`list` based on
orientation. You don't normally need to set `items` explicitly — setting any
`<item>_*` option (see [Per-item control](#per-item-control)) switches to it
automatically.

```ini
status_style = list
status_position = bottom-left
```

## Brightness

| Key | CLI | Values | Default |
|---|---|---|---|
| `brightness` | `--brightness` | `0`-`100` | `100` |

Software dimming — the LY protocol has no brightness command, so the image is
darkened before being sent.

## DeepCool integration *(restart required)*

| Key | CLI | Values | Default | In `trofeo.conf`? |
|---|---|---|---|---|
| `deepcool` | `--no-deepcool` (flag) / `--deepcool <bool>` | `true`/`false` | `true` (enabled) | yes |
| — | `--deepcool-update-ms` | `100`-`2000` | `1000` | **CLI only** |

Sends CPU temperature/usage/power/frequency to a DeepCool cooler/case display
over HID, on a background thread. Harmless to leave on even without a
DeepCool device attached (it just won't find one and stays idle).
`deepcool_update_ms` (the send interval) is a command-line-only tuning knob —
there's no matching config-file key for it.

```ini
deepcool = false          # never touch a DeepCool display
```

### What the case display shows (CH170 / CH270 / CH690 DIGITAL) — applies live

| Key | Values | Default |
|---|---|---|
| `deepcool_mode` | `freq` (the CPU frequency) \| `fan` (the fan speed, RPM) \| `alternate` (the two in turn) | `freq` |
| `deepcool_fan` | the fan for the RPM: `cpu` \| `case` \| `pump` \| `gpu` \| `both` (CPU fan and case fans in turn) | `cpu` |
| `deepcool_alternate` | seconds before it switches, `2`-`120` | `6` |

The RPM comes from [LibreHardwareMonitor](#fans-and-sensors-librehardwaremonitor); with no reading the
display shows `0`. With `deepcool_mode = alternate` and `deepcool_fan = both` the display cycles
frequency, CPU fan, case fans. The display protocol for the fan is not documented by DeepCool: the
number is sent in the place of the frequency and in the two bytes after it; if it does not appear on
your model, please report it. Other DeepCool models (coolers, CH360/CH560, CH510) have no RPM field.

```ini
deepcool_mode = alternate
deepcool_fan = case
deepcool_alternate = 8
```

## In-game FPS dashboard *(restart required)*

| Key | CLI | Values | Default |
|---|---|---|---|
| `fps_monitor` | `--fps-monitor` | `true`/`false` | `true` |

Measures FPS for any DirectX 9-12 game via ETW. **Requires running as
Administrator** — without it, FPS just won't populate (nothing else breaks).
Vulkan/OpenGL-native games aren't covered. The dashboard itself (FPS + GPU +
CPU + RAM while a game is in the foreground) is controlled by the `dashboard`
element in `show`/`hide`, independent of whether `fps_monitor` succeeds.

## OpenRGB sync — command-line only *(restart required)*

| CLI | Values | Default |
|---|---|---|
| `--openrgb-device <NAME>` | partial, case-insensitive device name | disabled |
| `--openrgb-poll-ms <N>` | milliseconds | `300` |

There's no `trofeo.conf` key for either of these — they only work as
command-line flags (e.g. in a shortcut's target, or a launch script).

Syncs the EQ spectrum color with an OpenRGB device's current color (polling —
not a live registration, so fast animated effects on the source device won't
be reflected smoothly). Requires OpenRGB running with **Settings → SDK
Server → Enable**. Until a matching device is found, falls back to `color`.

```
trofeo_lcd.exe --openrgb-device Aorus --openrgb-poll-ms 200
```

## Weather

| Key | CLI | Values | Default | Restart? |
|---|---|---|---|---|
| `weather_city` | `--weather-city <NAME>` | any city name | auto-detect from public IP | **yes** |
| `weather_unit` | `--weather-unit` | `c`/`celsius`, `f`/`fahrenheit` | `c` | no |

Current temperature + a small pixel-art condition icon, from
[Open-Meteo](https://open-meteo.com/) (free, no key). Two independent ways to
show it: as a `weather` line in per-item mode (`default2`/`items`) — where
`show`/`hide = weather` turns that line on/off, alongside `weather_size`/
`weather_position`/`weather_color`/`weather_backdrop` — and/or as its own big
panel (`layout = weather`), which always shows once that layout is active,
regardless of `show`/`hide` (same as every other panel layout). While the
network is unreachable or before the first successful fetch, it just shows
"N/A" instead of blocking anything else.

The `weather` panel additionally shows a **7-day forecast strip** below the
current conditions — one column per day with the abbreviated day name, a
small condition icon, the high/low temperature (in whatever `weather_unit`
picks) and, when the model provides it, the rain probability. It comes from
the same request as the current conditions, so there's nothing extra to
configure; a day is simply omitted if Open-Meteo doesn't have data for it. In
portrait mode the columns automatically shrink to fit the narrower panel.

```ini
weather_city = Milano
weather_unit = f
layout = default, weather:5
```

## Ticker

| Key | CLI | Values | Default | Restart? |
|---|---|---|---|---|
| `ticker_source` | — (config-file only) | one or more sources, comma-separated | none (ticker disabled) | **yes** |
| `ticker_refresh_min` | — (config-file only) | `1`-`1440` (minutes) | `10` | **yes** |
| `ticker_position` | `--ticker-position` | a [position](#positions) | `bottom` |  no |
| `ticker_size` | `--ticker-size` | `1`-`30` | `3` | no |
| `ticker_color` | `--ticker-color` | a [color](#colors) | follows `text_color` | no |
| `ticker_backdrop` | `--ticker-backdrop` | `true`/`false` | follows `text_backdrop` | no |
| `ticker_width` | `--ticker-width` | `1`-`100` (% of the screen) | `100` | no |

Scrolling headlines (news, or literally anything else) from one or more
sources, refreshed on a background thread — same idea as the weather module,
but you control what feeds it. Each entry in `ticker_source` can be:

- an **RSS/Atom feed URL** (`http://`/`https://`): headlines are taken from
  each item's `<title>`;
- **any other URL**: not a feed, so each non-empty line of the response
  becomes one ticker item — a simple way to point it at your own script,
  webhook, or plain-text endpoint without a dedicated integration;
- a **local file path**: each non-empty line (not starting with `#`) is one
  item, re-read on every refresh — handy if a script of yours writes its own
  text file.

Multiple sources are combined into one pool (up to 30 items; very long items
are truncated). There are two independent ways to display it, and you can use
either or both together:

- a scrolling line (a "chyron", like a news channel) drawn on top of
  whatever else is on screen — turned on/off with `show`/`hide = ticker`,
  positioned and styled with the `ticker_*` keys above;
- a dedicated full panel (`layout = news`), like any other [preset
  layout](#preset-layouts).

If no fetch has succeeded yet (first startup, or the sources are
unreachable), the ticker line/panel simply shows nothing — nothing else in
the program is affected. `ticker_source` and `ticker_refresh_min` need a
restart because they re-spawn the background fetch thread, same as
`weather_city`; everything else in this section applies live.

```ini
ticker_source = https://www.ansa.it/sito/ansait_rss.xml
ticker_refresh_min = 10
show = ticker
ticker_position = bottom
ticker_color = #FFC800

# Combine an RSS feed with your own text file:
ticker_source = https://example.com/feed.xml, C:\ticker\notes.txt

# Dedicated full-screen panel, rotating with the standard screen:
layout = default, news:20
```

## Per-item control

Every info item can be turned on/off individually with `show`/`hide`
(element names: `cpu, gpu, uptime, time, date, mem`/`ram, net, disk, volume,
nowplaying, weather`) and, independently, styled on its own. **Setting any one
of the options below switches the info block into per-item mode** (same as
`layout = default2`); items placed at the same position stack automatically.

| Option | Values | Default |
|---|---|---|
| `<item>_size` | `1`-`30` | `3` |
| `<item>_position` | a [position](#positions) | follows `status_position` |
| `<item>_color` | a [color](#colors) | follows `text_color` |
| `<item>_backdrop` | `true`/`false` | follows `text_backdrop` |

`ram_*` also works as an alias for `mem_*`.

```ini
cpu_size = 5
cpu_color = #FFC800
cpu_position = top-left

ram_size = 2
ram_position = bottom-left

gpu_size = 4
gpu_position = top-right

weather_size = 3
weather_position = bottom-right
weather_color = #60B0E0
weather_backdrop = true

nowplaying_position = bottom
```

Two options are specific to the `nowplaying` item, since the track title
scrolls:

| Key | Values | Default |
|---|---|---|
| `nowplaying_label` | `true`/`false` | `true` |
| `nowplaying_width` | `1`-`100` (% of the screen) | `100` |

```ini
nowplaying_label = false   # drop the "NOW PLAYING:" prefix, title only
nowplaying_width = 40      # cap the track's width so it can't cover neighbors
```

The track always scrolls without overlapping items that share its row —
`nowplaying_width` just lets you reserve less room for it up front.

## Units

| Key | CLI | Values | Default |
|---|---|---|---|
| `net_unit` | `--net-unit` | `kb`, `mb`, `auto` | `kb` |
| `mem_unit` (or `ram_unit`) | `--mem-unit` | `mb`, `gb` | `mb` |
| `percent_decimals` | `--percent-decimals` | `0`, `1` | `0` |

`auto` for network shows KB/s below 1 MB/s, then switches to MB/s.
`percent_decimals` adds one decimal place to every CPU/GPU/RAM usage
percentage shown anywhere in the program (`37%` → `37.4%`) — capped at 1,
since a second decimal (`37.42%`) is just noise at this panel's size.

```ini
net_unit = auto
mem_unit = gb
percent_decimals = 1
```

## Big clock

| Key | CLI | Values | Default |
|---|---|---|---|
| `clock_time_size` | `--clock-time-size` | `1`-`60` | `20` |
| `clock_date_size` | `--clock-date-size` | `1`-`30` | `6` |
| `clock_backdrop` | `--clock-backdrop` | `true`/`false` | follows `text_backdrop` |

```ini
clock_time_size = 28
clock_date_size = 8
clock_backdrop = true
```

## Panel transparency / backdrops

| Key | CLI | Values | Default |
|---|---|---|---|
| `panel_opacity` | `--panel-opacity` | `0`-`100` | `100` |
| `text_backdrop` | `--text-backdrop` | `true`/`false` | `false` |

`panel_opacity` is the background opacity of preset-layout panels and the
in-game dashboard (`100` = solid, `0` = outline only, background shows
through). `text_backdrop` puts a panel behind the status lines and the big
clock on the standard screen, at the same opacity as `panel_opacity`.
Individual `<item>_backdrop` and `clock_backdrop` override it per element.

```ini
panel_opacity = 55
text_backdrop = true
background = C:\Pictures\bg.jpg
```

## Preset layouts

| Key | CLI | Values | Default |
|---|---|---|---|
| `layout` | `--layout` | one or more names, comma-separated, each optionally `name:seconds` | none (standard screen) |
| `layout_interval` | `--layout-interval` | `2`-`3600` | `15` |
| `layout_spectrum` | `--layout-spectrum` | `true`/`false` | `false` |
| `layout_hold_on_music` | `--layout-hold-on-music` | `true`/`false` | `false` |

Names (`trofeo_lcd.exe --layout list` prints these with descriptions):
`default, default2, cpu-gpu, temps, overview, grid6, clock-center, io, music,
gaming, cpu, gpu, tiles, frame, weather, news`.

- `default` = the standard screen (status lines, spectrum, big clock).
- `default2` = the standard screen with [per-item](#per-item-control) styling.
- `news` = the [ticker](#ticker)'s headlines, big and scrolling, full screen.
- `tiles` = the [glass tiles](#tiles-layout) dashboard.
- `frame` = the [digital picture frame](#digital-frame): a collage of the pictures of a folder.
- Everything else replaces the clock with big panel(s); the top status block
  and spectrum turn off (use `show = nowplaying, time` etc. to add lines back
  on top of a panel layout; the game dashboard stays on regardless — turn it
  off with `hide = dashboard`). Set `layout_spectrum = true` to keep the audio
  spectrum visible on top of a panel layout while music is playing.

**Staying put while music plays**: with `layout_hold_on_music = true` the
rotation freezes on the *first* layout of the list as soon as music starts
(put the screen you want with the spectrum first, e.g. `layout = default,
weather, cpu-gpu`), instead of the spectrum covering whichever panel happened
to be up. When the music stops the rotation starts again from the first
layout.

**Rotation**: list several layouts to cycle through them. By default each
gets `layout_interval` seconds; give any entry its own duration with
`name:seconds` (2-3600) and it keeps that duration regardless of the others:

```ini
# Every layout gets 15s (layout_interval default):
layout = cpu-gpu, temps, music

# The standard screen for 30s straight, then the weather panel for just 5s,
# then back to 30s of the standard screen — not alternating every 5s:
layout = default:30, weather:5

# Mixing: cpu-gpu gets its own 20s, gpu falls back to layout_interval (40s here):
layout = cpu-gpu:20, gpu
layout_interval = 40
```

```ini
# Rotate the classic screen with the custom per-item one:
layout = default, default2
layout_interval = 20
```

## Tiles layout

`layout = tiles` shows rounded, semi-transparent tiles. Every key can also be a command-line flag
(`tiles_font` → `--tiles-font`), and all of them are applied live.

| Key | Values | Default |
|---|---|---|
| `tiles` | the tiles: `clock cpu cpu_temp cpu_watt gpu gpu_temp gpu_watt ram net disk fps weather music`, and with LibreHardwareMonitor `cpu_fan case_fan pump gpu_fan fans mb_temp disk_temp`, `,` between tiles, `\|` between rows, `:ring` for a ring, `clock:tall` (first tile only) as tall as all the rows; at most 16 | `clock:tall, cpu_temp, gpu_temp \| ram:ring, cpu:ring, gpu:ring` |
| `tiles_rpm` | a `CPU 1240 / CASE 780 RPM` line on the CPU tiles (needs LibreHardwareMonitor) | `true` |
| `tiles_fan_max` | the RPM that fills the bar of a fan tile, `500`-`10000` | `2500` |
| `tiles_font` | `smooth` \| `pixel` | `smooth` |
| `tiles_theme` | `dark` \| `light` | `dark` |
| `tiles_color` | the fill of the tiles | by theme |
| `tiles_text_color` | the text | by theme |
| `tiles_opacity` | `0`-`100` | by theme (66 / 52) |
| `tiles_radius` | corner radius, `0`-`80` | automatic |
| `tiles_gap` | space between tiles, `0`-`60` | `14` |
| `tiles_clock_seconds` | `true`/`false` | `false` |
| `tiles_cpu_watt_max`, `tiles_gpu_watt_max` | what 100% of the watt bar is, `10`-`2000` | `200`, `400` |
| `tiles_fps_max` | what 100% of the FPS bar is, `30`-`1000` | `240` |
| `tiles_bar_color` | `auto` (green-yellow-red by level), a color, or several (a gradient) | `auto` |
| `tile_<name>_color` | the same, for one tile (`tile_gpu_color`, `tile_cpu_temp_color`, ...) | `auto` |
| `tiles_bg` | `global` \| `color` \| `gradient` \| `effect` \| `image` | `global` |
| `tiles_bg_color` | one color, or several for a gradient | |
| `tiles_bg_image` | a picture, for `tiles_bg = image` | |
| `tiles_bg_dim` | darkens the picture / effect, `0`-`100` | `0` |
| `tiles_palette` | **a color of its own for every tile**, from the color presets: names separated by commas (`pastel fluo neon candy sunset ocean aurora fire forest ice mono rainbow` plus the one-color and water presets), `all`, `custom`; `off`/empty = one color for all | off |
| `tiles_colors` | your own colors (two or more), for `custom` | |
| `tiles_palette_interval` | seconds a preset stays before the next fades in, `5`-`3600` | `20` |
| `tiles_palette_stagger` | `0` = every tile changes preset at the same moment ... `100` = spread over the whole interval | `50` |

With `tiles_palette` the colors of the tiles drift slowly along the preset (a lap in 90 s) and,
with several presets, fade from one to the next. They are blended into the tile fill and kept dark
enough for the text (pale enough in the light theme). The screen asks for 5 frames a second.

```ini
layout = tiles
tiles_palette = pastel, ocean, sunset
tiles_palette_interval = 30
tiles_palette_stagger = 70
# one single preset: only the slow drift
# tiles_palette = aurora
# your own colors
# tiles_palette = custom
# tiles_colors = #FF0080, #00C8FF, #FFC800
```

## Fans and sensors (LibreHardwareMonitor)

Fan speeds come from [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)'s
web server: start it **as administrator**, then *Options → Remote Web Server → Run* (port 8085) and
leave it open. Nothing happens when it is not running. All of these apply live (a new address or
interval is picked up within a few seconds); every key can also be a command-line flag
(`lhm_cpu_fan` → `--lhm-cpu-fan`).

| Key | Values | Default |
|---|---|---|
| `lhm` | `true` \| `false` | `true` |
| `lhm_url` | the address of the web server | `http://127.0.0.1:8085` |
| `lhm_interval` | seconds between readings, `1`-`60` | `2` |
| `lhm_cpu_fan` | a part of the name of the CPU fan sensor (empty = automatic) | automatic |
| `lhm_case_fan` | the same, for the case fans (the average of the ones that spin) | automatic |
| `lhm_pump` | the same, for the pump / AIO | automatic |
| `lhm_mb_temp` | the same, for the motherboard temperature | automatic |

The automatic choice: the CPU fan is a fan whose name contains "cpu" (and is not a pump); the pump
one whose name contains "pump"; the case fans are the other fans of the board (the average of the ones
that spin; fans at 0 RPM are not plugged in); the GPU fan the one of the graphics card; the motherboard
temperature the one called motherboard / system / board; the disk temperature the hottest drive. Names are
matched on a part of "device / sensor", any case. `trofeo_lcd --lhm-list` prints what LibreHardwareMonitor
reports and what each role picked, so you can see which name to write. A value with a `#` in it goes in
quotes: `lhm_cpu_fan = "Fan #2"`.

Where the readings show up: the `cpu_fan`, `case_fan`, `pump`, `gpu_fan`, `fans`, `mb_temp` and
`disk_temp` [tiles](#tiles-layout), the `tiles_rpm` line on the CPU tiles, the CPU line of the status
screens (`hide = cpu_fan, case_fan, pump`), [Deck, Cards and Orbit](#deck-cards-and-orbit), and the
[DeepCool display](#deepcool-integration-restart-required). The GPU fan fills in when the driver does
not report one, and the CPU temperature and power when the built-in sensor does not know the CPU.

```ini
lhm_cpu_fan = "Fan #2"
lhm_pump = AIO
tiles = clock:tall, cpu_temp, fans | ram:ring, cpu:ring, cpu_fan:ring, mb_temp
```

## Deck, Cards and Orbit

Three more full screens (`layout = deck`, `cards`, `orbit`). Every key can also be a command-line
flag and all of them apply live. Panels: `cpu gpu ram fans net disk clock weather music` (`clock` also shows
the weather and the playing track). `fans` needs [LibreHardwareMonitor](#fans-and-sensors-librehardwaremonitor).

| Key | Values | Default |
|---|---|---|
| `deck` | 2 to 5 panels, one per column | `cpu, gpu, ram, clock` |
| `deck_gauge` | the gauge line on top of each column | `true` |
| `cards` | `\|` between columns, `,` stacks cards in a column; up to 6 columns of 3 cards | `clock \| cpu \| gpu \| ram, disk \| music, net` |
| `cards_radius` | corner radius, `0`-`60` | by size |
| `cards_gap` | pixels between cards, `0`-`40` | `12` |
| `orbit` | 1 to 4 devices, one cluster of rings each | `cpu, gpu, ram` |
| `orbit_clock` | the clock block on the right | `true` |
| `orbit_legend` | the names of the rings under each cluster | `true` |
| `dash_palette` | `default` or a preset of the animated backgrounds (`pastel`, `ocean`, `sunset`, ...) | `default` |
| `dash_colors` | your own accent colors (used instead of the preset) | |
| `dash_font` | `smooth` \| `pixel` | `smooth` |
| `dash_text_color` | the text | white |
| `dash_plate` | darkness of the plate behind `deck` and `orbit`, `0`-`100` | `50` |
| `dash_graph` | the graphs of the last minutes | `true` |
| `dash_history` | seconds of history, `20`-`600` | `90` |
| `dash_net_max` | network MB/s that fills a ring or gauge, `1`-`1000` | `25` |
| `dash_bg` | `global` \| `color` \| `gradient` \| `effect` \| `image` | `global` |
| `dash_bg_color` / `dash_bg_image` / `dash_bg_dim` | for `color`/`gradient`, for `image`, darkening `0`-`100` | |

The maxima of watts and fan speed are the ones of the tiles (`tiles_cpu_watt_max`, `tiles_gpu_watt_max`,
`tiles_fan_max`) and the seconds on the clock `tiles_clock_seconds`. The rings of `orbit` are: CPU load /
temperature / power, GPU load / temperature / power, RAM / disk / network, one per fan for `fans`. Each cluster
takes its own run of colors from the palette. A reading that does not exist (no temperature sensor, no
LibreHardwareMonitor) is left out and the rest rearranges itself. The screen is redrawn about once a second and
only when something changed; `dash_bg = effect` or `image` asks for the frame rate of that background.

```ini
layout = cards
cards = clock | cpu, ram | gpu, fans | net, disk, music
dash_palette = ocean
cards_gap = 16
```

## Digital frame

`layout = frame`: a collage of the pictures of a folder, with the glass tiles of the
[tiles layout](#tiles-layout) at the sides. Every key can also be a command-line flag
(`frame_folder` → `--frame-folder`), and all of them are applied live.

| Key | Values | Default |
|---|---|---|
| `frame_folder` | the folder with the pictures (JPG, PNG, WebP, BMP, GIF; not HEIC) | none: the screen asks for it |
| `frame_style` | `mosaic` (a tidy collage) \| `polaroid` (scattered prints with a white border) | `mosaic` |
| `frame_photos` | pictures on screen at once, `1`-`9` (never more than the folder has) | `5` |
| `frame_interval` | seconds between changes: each time ONE picture is replaced, `2`-`3600` | `8` |
| `frame_order` | `shuffle` (every picture comes up before any repeats) \| `name` (alphabetical, round and round) | `shuffle` |
| `frame_transition` | `fade` \| `slide` (a wipe in the mosaic, a print dropping in with polaroid) | `fade` |
| `frame_recursive` | include the subfolders | `false` |
| `frame_refresh` | seconds between two looks at the folder, `5`-`3600` | `60` |
| `frame_bg` | `blur` (the current picture, blurred and dark) \| `dark` \| `global` (the program's normal background, image or effect) | `blur` |
| `frame_dim` | how dark the blurred background is, `0`-`100` | `60` |
| `frame_gap` | mosaic: pixels between the pictures, `0`-`40` | `10` |
| `frame_radius` | mosaic: corner radius, `0`-`60` | `14` |
| `frame_left`, `frame_right` | the tiles of the side columns (names as in `tiles`, `:ring` allowed, `none` = no tiles: that side goes to the pictures) | `clock, ram:ring` / `cpu:ring, gpu:ring` |
| `frame_side_width` | width of a side in % of the long side of the screen, `8`-`35` | `17` |

- On a **portrait** screen the sides become a strip at the top (`frame_left`) and one at the
  bottom (`frame_right`).
- The look of the sides (theme, font, colors, `tiles_palette`, ...) is the one of the
  [tiles layout](#tiles-layout).
- Phone photos are shown upright (the EXIF rotation is applied). With fewer pictures than
  `frame_photos` the collage has as many places as pictures and nothing changes.
- A folder that is missing or has no pictures shows a message; files that cannot be read are
  skipped. In the settings editor, the *Digital frame* section has a folder picker and a preview
  with your own pictures.
- Cost: the pictures are decoded and scaled on a background thread, once per change; while nothing
  moves the screen idles at 2 frames a second, and a change runs at 20 for about a second and a
  half.

```ini
layout = frame
frame_folder = C:\Users\Me\Pictures\Holidays
frame_style = polaroid
frame_photos = 4
frame_interval = 12
frame_transition = slide
frame_left = clock
frame_right = cpu:ring, gpu:ring, ram:ring
tiles_theme = dark
tiles_palette = ocean, sunset      # the side tiles in changing colors
```

## Screenshot hotkey — command-line only *(restart required)*

| CLI | Values | Default |
|---|---|---|
| `-k`, `--screenshot-key <KEY>` | `f1`-`f12`, `printscreen` | disabled |

No `trofeo.conf` key for this one either — global hotkey (works even without
focus) that saves the current LCD frame as a lossless PNG to the Desktop.

```
trofeo_lcd.exe --screenshot-key f9
```

## Hide the console window — command-line only *(restart required)*

| CLI | Values | Default |
|---|---|---|
| `--hide-console` (flag, no value) | — | disabled |

Hides the terminal window right after startup (Windows only); no `trofeo.conf`
key for it. No log file is written in its place — if something goes wrong,
check `trofeo-errors.txt` next to the executable for config problems, since
nothing else prints anywhere visible.

```
trofeo_lcd.exe --hide-console
```

## Other flags (not settings, but useful)

| Flag | What it does |
|---|---|
| `-h`, `--help` | Prints the full command-line option list and exits. |
| `--layout list` | Prints every layout name with its description and exits. |
| `--diag` | Prints GPU sensor diagnostics (which source is used for NVIDIA/AMD temperature, power, clock) and weather diagnostics (resolved location + a sample reading), then exits. Useful to check `weather_city` actually resolves to the place you meant, or why a GPU reading shows N/A. |

## Config file location

| CLI | Purpose |
|---|---|
| `--config <FILE>` | use this file instead of searching the default locations |

Without `--config`, the program looks for `trofeo.conf` (then `trofeo.config`)
next to the executable, then in the current working folder.

## Audio / adaptive FPS

`--idle-fps`/`--active-fps` rarely need changing, and — like the DeepCool
interval, OpenRGB and screenshot-hotkey options above — only exist as
command-line flags, not as `trofeo.conf` keys. `silence_threshold` and
`silence_timeout_ms`, on the other hand, **are** `trofeo.conf` keys (they
also still work as CLI flags, which take priority if both are set) and
apply live, no restart needed:

| Key | CLI | Values | Default |
|---|---|---|---|
| — | `--idle-fps <N>` | frames/sec | `2` |
| — | `--active-fps <N>` | frames/sec | `15` |
| `silence_threshold` | `--silence-threshold <N>` | `0.0`-`1.0` peak amplitude | `0.005` |
| `silence_timeout_ms` | `--silence-timeout-ms <N>` | milliseconds | `800` |

The panel sends frames at `idle_fps` while there's no sound (to save CPU —
JPEG encoding + USB transfer are the main cost) and jumps to `active_fps` as
soon as sound is detected again; `silence_threshold`/`silence_timeout_ms`
tune how "silence" is detected: the loudest sample in each window is
compared against `silence_threshold`, and anything below it counts as
silence. Loopback capture reflects the actual system/app volume, so a quiet
listening level can end up under the default threshold and look like
silence — the EQ bars staying down even though something is playing is the
usual symptom. Lower the value (e.g. `0.001`) if that happens:

```ini
silence_threshold = 0.001
```

```
trofeo_lcd.exe --idle-fps 1 --active-fps 20
```

## Everything at a glance

```ini
# --- Look & feel ---
orientation = landscape
language = en
text_color = #FFFFFF
clock_color = #00C8FF
brightness = 95
background = C:\Pictures\bg.jpg
background_dim = 45
# ...or, instead of a picture, a soft animated effect (see "Animated background effect"):
# background_effect = plasma
# effect_palette = neon
# effect_speed = 30
# effect_brightness = 40

# --- What to show ---
show = cpu, gpu, mem, nowplaying, clock, spectrum, weather, ticker
hide = disk, gpu_fan

# --- Per-item styling (applies to default2, below) ---
cpu_size = 5
cpu_position = top-left
gpu_size = 4
gpu_position = top-right
weather_size = 3
weather_position = bottom-right
weather_color = #60B0E0
nowplaying_position = bottom
nowplaying_width = 45

# --- Weather ---
weather_city = Milano
weather_unit = c

# --- Ticker (restart to apply ticker_source/ticker_refresh_min) ---
ticker_source = https://www.ansa.it/sito/ansait_rss.xml
ticker_refresh_min = 10
ticker_position = bottom
ticker_color = #FFC800

# --- Rotation: the per-item screen for 30s, then the weather panel for 5s ---
# (must say "default2" explicitly here for the per-item styling above to
# apply — plain "default" would show the classic screen instead)
layout = default2:30, weather:5
layout_interval = 15

# --- Panels ---
panel_opacity = 60
text_backdrop = true

# --- Pixel game under the clock ---
pixel_game = true
pixel_game_hero = both

# --- Music screen ---
music_screen = true
music_bg = gradient
music_bg_brightness = 40
music_stats = cpu, gpu, ram

# --- Spectrum ---
spectrum_style = led, area, mirror
spectrum_palette = fire, ocean, neon
layout_hold_on_music = true

# --- Units ---
net_unit = auto
mem_unit = gb

# --- Integrations (restart to apply) ---
deepcool = true
fps_monitor = true
# ticker_source/ticker_refresh_min above also require a restart.
# screenshot hotkey, OpenRGB sync, idle/active FPS, hide_console: command-line
# only, not trofeo.conf keys — see the sections above.
```

```
:: launch shortcut / script, combining the command-line-only options above
trofeo_lcd.exe --screenshot-key f9 --openrgb-device Aorus --hide-console
```

See also [`trofeo.conf.example`](./trofeo.conf.example) for the same options
as inline comments you can uncomment directly, and the main
[`README.md`](./README.md) for setup, second-monitor mode, and build
instructions.
