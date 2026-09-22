# Guide: Using the Trofeo Vision 9.16 as a Second Monitor

This guide explains how to turn the **Thermalright Trofeo Vision 9.16 LCD**
into a **real second monitor on Windows**, so you can drag app windows (like
Spotify, Discord, a browser, or Task Manager) directly onto the Trofeo
screen.

---

## 1. How it works

The Trofeo screen connects to the PC only through a USB cable (no HDMI /
DisplayPort). Therefore:

1. We create a **Virtual Monitor** in Windows at the native resolution
   **1920×462**.
2. The **`trofeo_screen`** program captures that virtual monitor in real
   time via the GPU (DXGI Desktop Duplication API) and sends it to the Trofeo
   screen over USB.

---

## 2. Step 1: Installing the Virtual Display Driver (VDD)

1. Visit the open-source **[Virtual-Display-Driver (VDD)](https://github.com/itsmebias/virtual-display-driver/releases)** releases page.
2. Download the latest `.zip` and extract it (e.g. to `C:\VirtualDisplayDriver`).
3. Open the config file `vdd.xml` (or `options.txt`, depending on the VDD
   version) with Notepad.
4. Add the special Trofeo Vision resolution:
   ```xml
   <resolution>
     <width>1920</width>
     <height>462</height>
     <refresh_rate>60</refresh_rate>
   </resolution>
   ```
5. Right-click `install.bat` (or run the VDD install command) as
   Administrator.
6. Open **Windows Settings → System → Display**:
   - A new monitor (**Display 2**) should appear.
   - Set its resolution to **1920 × 462**.
   - Position the second monitor above, below, or beside your main monitor
     as you wish.

---

## 3. Step 2: Running `trofeo_screen`

### Check detected monitors

```bash
cargo run --bin trofeo_screen -- --list-displays
```

Or from the release binary (`.\target\release\trofeo_screen.exe --list-displays`):

```text
Detected Monitors:
---------------------------------------------------------------------------
INDEX  ADAPTER                  DEVICE           RESOLUTION  STATUS
---------------------------------------------------------------------------
0      AMD Radeon RX 6600       \\.\DISPLAY1     1920x1080    Active
1      IddCx Virtual Display    \\.\DISPLAY2     1920x462     Active [MATCH 1920x462]
---------------------------------------------------------------------------
```

*The program automatically detects the display at 1920×462!*

### Start streaming to the Trofeo screen

```bash
cargo run --release --bin trofeo_screen
```

Or just run the compiled binary:

```powershell
.\target\release\trofeo_screen.exe
```

The program will automatically:

1. Connect to the Trofeo screen over USB bulk (`0416:5408`).
2. Capture the virtual monitor in real time.
3. Stream window movement and the mouse cursor to the Trofeo screen.

---

## 4. Options & command-line arguments

| Option | Purpose | Default |
|---|---|---|
| `-l`, `--list-displays` | List detected monitors and exit | - |
| `-d`, `--display <N>` | Pick a monitor index manually when there are several | Auto (1920x462) |
| `--fps <N>` | Maximum target frame rate while the screen moves | `30` |
| `--idle-fps <N>` | Polling speed while the screen is static (saves CPU) | `10` |
| `-q`, `--quality <1-100>` | JPEG image compression quality | `75` |
| `-r`, `--rotate` | Rotate the image 180° if the screen is installed upside down | `false` |
| `--hide-console` | Hide the black terminal window (nice for startup) | `false` |
| `-k`, `--screenshot-key <KEY>` | Global hotkey to save a screenshot of the current LCD frame (f1-f12, `printscreen`). Lossless PNG files are saved to your **Desktop** | off |

### Example use cases

* **If the screen is mounted upside down**:
  ```powershell
  .\target\release\trofeo_screen.exe --rotate
  ```
* **Sharper images (e.g. for small text)**:
  ```powershell
  .\target\release\trofeo_screen.exe --quality 85 --fps 25
  ```
* **Auto-start at Windows login with no black window**:
  ```powershell
  .\target\release\trofeo_screen.exe --hide-console
  ```

---

## 5. Auto-start at Windows boot / login

1. Press `Win + R`, type `shell:startup`, and press **Enter**.
2. Right-click inside that folder → **New** → **Shortcut**.
3. Point the target at the executable location:
   ```text
   "C:\Users\USER\Documents\Default Project\trofeo-lcd\target\release\trofeo_screen.exe" --hide-console
   ```
4. Click **Next** and name it `Trofeo Screen`. The second monitor will now
   activate automatically every time you log into Windows!

---

> ⚠️ **Important:** `trofeo_screen` and `trofeo_lcd` use the same LCD — run
> **one of them**, never both at the same time.