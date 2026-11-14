# Autostart on Windows

[← Back to the README](../README.md) · [All the docs](./README.md)

`scripts/install-autostart.ps1` (included in the Windows release zip, in the `scripts` folder next to the exe) adds
`trofeo_lcd.exe` to Windows startup. By default it registers a **Scheduled
Task** that runs at logon with elevated ("Run as administrator")
privileges and no UAC prompt on every login — needed if you use
`fps_monitor` or PawnIO (AMD sensors). The script finds `trofeo_lcd.exe` by itself (next to the script, or one
folder up, or in the folder you are in).

The simplest way, step by step:

1. Open PowerShell **as administrator** (right-click Start, *Terminal (Admin)* or *Windows PowerShell (Admin)*).
2. Run the script through PowerShell with the execution policy bypassed for that one command. **Put the path in
   quotes**, always when a folder has a space in its name (`Sensor Panel`), or PowerShell stops at the space:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "D:\Sensor Panel\trofeo-lcd-windows\scripts\install-autostart.ps1"
```

That is all: it prints `Done. Scheduled task 'TrofeoLCD' will start ... at every logon`. You do not need to open
PowerShell in a particular folder, and you do not need to mark `trofeo_lcd.exe` as *Run as administrator* yourself:
the task already starts it elevated. If the script cannot find the exe (for example you moved it away from the
scripts), give its path:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "D:\Sensor Panel\trofeo-lcd-windows\scripts\install-autostart.ps1" -Exe "D:\Sensor Panel\trofeo-lcd-windows\trofeo_lcd.exe"
```

Other ways, from PowerShell opened in the folder of the exe (add the `powershell.exe -NoProfile -ExecutionPolicy Bypass -File`
prefix if Windows says that running scripts is disabled):

```powershell
.\scripts\install-autostart.ps1                              # elevated task, --hide-console
.\scripts\install-autostart.ps1 -Arguments ""                # elevated task, console visible
.\scripts\install-autostart.ps1 -Method Shortcut              # simple Startup-folder shortcut, no admin needed, not elevated
```

Remove it with `.\scripts\uninstall-autostart.ps1` (same quotes and `-ExecutionPolicy Bypass` rules; removing the elevated
task needs PowerShell as administrator; it removes whichever method was used). See the comments at the top of each
script (or `Get-Help .\scripts\install-autostart.ps1 -Full`) for every option.

**If you mark `trofeo_lcd.exe` as "Run as administrator" yourself** (Properties, Compatibility) Windows will not let
a normal program start it. From 2.9.3 the settings editor works around that for the preview, but if it still says
*"The requested operation requires elevation (os error 740)"* open `trofeo_config.exe` as administrator too.
Only `trofeo_lcd.exe` is started at logon: the settings editor is opened by hand when you want it.
