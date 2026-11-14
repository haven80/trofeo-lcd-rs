# Command-line options

[← Back to the README](../README.md) · [All the docs](./README.md)

For every option the program supports — config-file keys and command-line
flags, with defaults, valid values and examples — see the
**[Options guide](../OPTIONS_GUIDE.md)**, or open the [settings editor](../README.md#settings-editor-trofeo_config).
The table below covers just the handful of flags that only exist on the command line.

| Option | Purpose | Default |
|---|---|---|
| `--idle-fps` / `--active-fps` | FPS when idle / has sound | `2` / `15` |
| `--update-demo` | Show the [new version banner](CONFIGURATION.md#new-version-notice) now, with a made-up version | off |
| `--no-deepcool` | Turn off DeepCool integration | enabled |
| `--deepcool-update-ms` | DeepCool send interval (100–2000 ms) | `1000` |
| `--openrgb-device <NAME>` | Sync color with an OpenRGB device | disabled |
| `--lcd-test` | `0416:5302` screens (Trofeo Vision 6.86 ...): shows numbered test pictures (5 s each), asks which one is upright and writes `hid_mount` / `hid_header` into `trofeo.conf` (the settings editor has a button for it) | off |
| `--weather-city <NAME>` | Fixed city for the weather module | auto (IP-based) |
| `--hide-console` | Hide the console window (Windows) | disabled |
| `--start-in-tray` | Hide the console window and leave an icon in the tray that shows it again (Windows); same as `start_in_tray = true` | disabled |
| `-k, --screenshot-key` | Global hotkey to save the current LCD frame as a lossless PNG screenshot (f1-f12, printscreen) | off |
| `--preview-png <FILE>` | Draw one frame with sample data into a PNG and exit — no USB, audio or sensors (this is what the settings editor uses; with `--preview-state`, `--preview-layout`, `--preview-time`) | off |
