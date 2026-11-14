# Which sensors you need and what to install

[← Back to the README](../README.md) · [All the docs](./README.md)

Most readings work on their own. A few need a helper program, and which one depends on your
hardware. If a value shows `N/A`, stays empty, or a tile never appears, find it in this table.

| What | Needs | What to install |
|---|---|---|
| CPU load, frequency, RAM, network, disk, clock, weather, music | nothing | nothing |
| **NVIDIA** graphics card (temperature, load, power, fan, memory) | the NVIDIA driver | the normal NVIDIA driver (it contains `nvml.dll`) |
| **AMD Radeon** graphics card (temperature, load, power, fan, hot spot) | the Radeon driver | the normal AMD Adrenalin driver. The usage % is the driver's own figure; `gpu_usage_source = windows` goes back to the Windows counter. Some cards (the RX 9070 XT) do not report their power: the power tile stays empty, the rest works |
| **AMD Ryzen** CPU temperature and power | PawnIO + Administrator | [PawnIO](https://github.com/namazso/PawnIO), then start the program **as administrator** |
| **Intel** CPU temperature and power | LibreHardwareMonitor | see below |
| Fans (CPU, case, pump), motherboard and disk temperature, GPU hot spot | LibreHardwareMonitor | see below |
| Any CPU the built-in reader does not know (it covers Ryzen Zen 1 to 4; if `--diag` says `Sensor chosen: none`) | LibreHardwareMonitor | see below |

**Do Intel users have to use LibreHardwareMonitor?** For the **CPU temperature and power on
Windows, yes.** The program's own CPU sensor reads AMD Ryzen only, so on an Intel CPU those two
values stay empty until LibreHardwareMonitor provides them. Everything else (CPU load and
frequency, RAM, network, disk, and the graphics card) does not need it. On **Linux** the CPU
temperature of an Intel processor is not read at all, and LibreHardwareMonitor does not exist
there, so those two values stay empty.

**Setting up LibreHardwareMonitor (Windows), once:**

1. Download [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases)
   and unzip it anywhere.
2. Start it **as administrator** (right click → *Run as administrator*). Without administrator
   rights it cannot read the sensors.
3. Open *Options → Remote Web Server → Run*. The default port is `8085`; if you change it, set
   `lhm_url` to match (see [Fans and sensors](CONFIGURATION.md#fans-and-sensors-librehardwaremonitor)).
4. Leave it running, it can sit in the tray. Tick *Run on Windows Startup* and
   *Start Minimized* in its *Options* menu if you want it to start with the PC.

Nothing else to configure: when the program sees the web server it takes the CPU temperature and
power from it (only when its own sensor has nothing, so AMD users with PawnIO are not affected),
plus the fans, motherboard and disk temperature and the GPU hot spot.

**If a value is still missing:**

- `trofeo_lcd --diag` prints what the program's own sensors find (on Windows also the GPU usage from the driver and from the Windows counter, and which GPU engines are busy): the NVIDIA card, the Radeon card and the CPU sensor it picked (`Sensor chosen: none` on an Intel CPU, which is normal and means LibreHardwareMonitor is needed).
- `trofeo_lcd --lhm-list` prints everything LibreHardwareMonitor reports and which sensor each
  role uses. If the list is empty, the web server is not running (step 3) or is on another port.
  If your board names a sensor differently, point the role at it with `lhm_cpu_fan = "Fan #2"`
  and the other `lhm_*` keys.
- A tile can only show what it is told to: check that the item is in `tiles`, `show` or the
  screen you picked (the settings editor lists them all).
- Two programs cannot drive the same screen: close the vendor software (TRCC) first.
