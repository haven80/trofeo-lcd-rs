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
  Indonesian to English (the UI itself is still selectable between English
  and Italian via `language = en | it`).
- Added a **weather module**: current temperature + a stylized pixel-art
  condition icon, via Open-Meteo, with automatic IP-based geolocation and an
  optional fixed-city override (`weather_city`), shown as a per-item line
  and/or as its own full-panel layout (`layout = weather`).
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
