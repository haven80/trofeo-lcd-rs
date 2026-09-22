# Options guide

Every setting trofeo_lcd supports, in one place, with examples. For a shorter,
inline-commented version you can copy and edit directly, see
[`trofeo.conf.example`](./trofeo.conf.example).

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
- **Comments and quoting**: `#` starts a comment, except when a value itself
  starts with `#` (a hex color, e.g. `cpu_color = #FFC800`). Values can be
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
*except* `deepcool`, `fps_monitor`, and `weather_city` — change one of these
in the file and the program tells you on the next reload that a restart is
needed, while the rest of the file still applies live.

Separately, a handful of options only exist as command-line flags at all
(`--openrgb-device`, `--openrgb-poll-ms`, `--hide-console`, `-k`/
`--screenshot-key`, `--idle-fps`, `--active-fps`, `--silence-threshold`,
`--silence-timeout-ms`, `--deepcool-update-ms`) — since they're never read
from `trofeo.conf`, the only way to change them is to relaunch the program
with different arguments. Each is marked "command-line only" below.

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
| `language` | `--language` | `en`, `it` | `en` |

Only the on-screen text (weekday/month names, labels like "NOW PLAYING",
weather conditions) follows this. Console/log output is always in English.

```ini
language = it
```

## Background

| Key | CLI | Values | Default |
|---|---|---|---|
| `background` | `--background <FILE>` | path to an image (jpg/png/bmp), animated GIF, or video | none |
| `background_dim` | `--background-dim` | `0`-`100` | `40` |
| `ffmpeg` | `--ffmpeg <PATH>` | path to `ffmpeg.exe` | next to the exe, then `PATH` |
| `background_fit` | `--background-fit` | `cover`, `stretch`, `contain`, `original` | `cover` |
| `background_position` | `--background-position` | a [position](#positions) | `center` |
| `background_offset_x`/`background_offset_y` | `--background-offset-x/-y` | integer px (+ = right/down) | `0` |

Video backgrounds need ffmpeg; `background_dim` darkens the image so text
stays readable over it. With an animated background (GIF or video) the idle
FPS is automatically raised to the active FPS, so the animation stays smooth
even while nothing else is happening.

```ini
background = C:\Pictures\space.mp4
background_dim = 55
background_fit = cover
ffmpeg = C:\ffmpeg\bin\ffmpeg.exe
```

## What to show — `show` / `hide`

| Key | CLI | Values | Default |
|---|---|---|---|
| `show` | `--show <LIST>` | comma-separated element names — show **only** these | everything on |
| `hide` | `--hide <LIST>` | comma-separated element names — turn these **off** | nothing hidden |

Element names: `cpu, gpu, uptime, time, date, mem` (or `ram`)`, net, disk,
volume, nowplaying, weather, spectrum, clock, clock_date, dashboard`.

Detail fields inside the CPU/GPU lines can also be individually hidden:
`cpu_freq, cpu_temp, cpu_power, gpu_temp, gpu_power, gpu_fan, gpu_clock, gpu_fps`.

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

```ini
weather_city = Milano
weather_unit = f
layout = default, weather:5
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

`auto` for network shows KB/s below 1 MB/s, then switches to MB/s.

```ini
net_unit = auto
mem_unit = gb
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

Names (`trofeo_lcd.exe --layout list` prints these with descriptions):
`default, default2, cpu-gpu, temps, overview, grid6, clock-center, io, music,
gaming, cpu, gpu, weather`.

- `default` = the standard screen (status lines, spectrum, big clock).
- `default2` = the standard screen with [per-item](#per-item-control) styling.
- Everything else replaces the clock with big panel(s); the top status block
  and spectrum turn off (use `show = nowplaying, time` etc. to add lines back
  on top of a panel layout; the game dashboard stays on regardless — turn it
  off with `hide = dashboard`). Set `layout_spectrum = true` to keep the audio
  spectrum visible on top of a panel layout while music is playing.

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

## Audio / adaptive FPS — command-line only

These rarely need changing, and — like the DeepCool interval, OpenRGB and
screenshot-hotkey options above — only exist as command-line flags, not as
`trofeo.conf` keys:

| CLI | Values | Default |
|---|---|---|
| `--idle-fps <N>` | frames/sec | `2` |
| `--active-fps <N>` | frames/sec | `15` |
| `--silence-threshold <N>` | `0.0`-`1.0` peak amplitude | `0.005` |
| `--silence-timeout-ms <N>` | milliseconds | `800` |

The panel sends frames at `idle_fps` while there's no sound (to save CPU —
JPEG encoding + USB transfer are the main cost) and jumps to `active_fps` as
soon as sound is detected again; `silence_threshold`/`silence_timeout_ms`
tune how "silence" is detected.

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

# --- What to show ---
show = cpu, gpu, mem, nowplaying, clock, spectrum, weather
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

# --- Rotation: the per-item screen for 30s, then the weather panel for 5s ---
# (must say "default2" explicitly here for the per-item styling above to
# apply — plain "default" would show the classic screen instead)
layout = default2:30, weather:5
layout_interval = 15

# --- Panels ---
panel_opacity = 60
text_backdrop = true

# --- Units ---
net_unit = auto
mem_unit = gb

# --- Integrations (restart to apply) ---
deepcool = true
fps_monitor = true
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
