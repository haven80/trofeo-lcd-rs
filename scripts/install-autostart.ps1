<#
.SYNOPSIS
  Adds trofeo_lcd (or trofeo_screen) to Windows startup.

.DESCRIPTION
  Two methods are available:

    -Method Task     (default, recommended) Registers a Scheduled Task that
                      starts the program at logon. This lets it start
                      already ELEVATED ("Run as administrator") without a
                      UAC prompt at every login — needed for the in-game
                      FPS dashboard (fps_monitor) and AMD sensors (PawnIO).
                      A plain Startup-folder shortcut can't do this: it
                      would either fail silently or prompt UAC every time.

    -Method Shortcut  Classic method: a shortcut in the per-user Startup
                      folder (shell:startup). Simpler, no admin rights
                      needed to install, but the program itself then runs
                      WITHOUT elevation (fine if you don't use fps_monitor
                      or PawnIO).

.PARAMETER Exe
  Path to trofeo_lcd.exe (or trofeo_screen.exe). Defaults to
  trofeo_lcd.exe next to this script.

.PARAMETER Arguments
  Extra command-line arguments passed to the program at startup, e.g.
  "--hide-console" to start with no visible console window.

.PARAMETER Method
  "Task" (default) or "Shortcut". See DESCRIPTION above.

.PARAMETER Elevated
  Only for -Method Task. $true (default) = run with highest privileges.
  Set -Elevated:$false to run as a normal (non-admin) user instead.

.PARAMETER TaskName
  Name of the Scheduled Task / shortcut file. Default: "TrofeoLCD".

.EXAMPLE
  # From the folder with trofeo_lcd.exe (a UAC prompt will appear once):
  .\install-autostart.ps1

.EXAMPLE
  .\install-autostart.ps1 -Exe "C:\Tools\trofeo_lcd\trofeo_lcd.exe" -Arguments "--hide-console"

.EXAMPLE
  # No admin rights needed, no elevation at startup:
  .\install-autostart.ps1 -Method Shortcut
#>

[CmdletBinding()]
param(
    [string]$Exe = (Join-Path $PSScriptRoot "trofeo_lcd.exe"),
    [string]$Arguments = "--hide-console",
    [ValidateSet("Task", "Shortcut")]
    [string]$Method = "Task",
    [bool]$Elevated = $true,
    [string]$TaskName = "TrofeoLCD"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $Exe)) {
    Write-Error "Executable not found: $Exe`nPass -Exe <path> or run this script from the folder that contains trofeo_lcd.exe."
    exit 1
}
$Exe = (Resolve-Path $Exe).Path
$WorkDir = Split-Path $Exe -Parent

function Test-IsAdmin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $p = New-Object Security.Principal.WindowsPrincipal($id)
    return $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if ($Method -eq "Task") {
    if ($Elevated -and -not (Test-IsAdmin)) {
        Write-Warning "Registering an elevated task requires Administrator rights: relaunching this script elevated (a UAC prompt will appear once, only now)."
        $psArgs = @(
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$PSCommandPath`"",
            "-Exe", "`"$Exe`"", "-Arguments", "`"$Arguments`"",
            "-Method", "Task", "-Elevated", "$Elevated", "-TaskName", "`"$TaskName`""
        )
        Start-Process -FilePath "powershell.exe" -ArgumentList $psArgs -Verb RunAs
        exit
    }

    $action = New-ScheduledTaskAction -Execute $Exe -Argument $Arguments -WorkingDirectory $WorkDir
    $trigger = New-ScheduledTaskTrigger -AtLogOn
    $settings = New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable `
        -ExecutionTimeLimit ([TimeSpan]::Zero)
    $runLevel = if ($Elevated) { "Highest" } else { "Limited" }
    $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel $runLevel

    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue

    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger `
        -Settings $settings -Principal $principal `
        -Description "Starts $(Split-Path $Exe -Leaf) at logon (installed by trofeo_lcd's install-autostart.ps1)." | Out-Null

    Write-Host "Done. Scheduled task '$TaskName' will start:" -ForegroundColor Green
    Write-Host "  $Exe $Arguments"
    Write-Host "at every logon$(if ($Elevated) {' (elevated, no UAC prompt at logon)'})."
    Write-Host "Start it right now:  Start-ScheduledTask -TaskName '$TaskName'"
    Write-Host "Remove it:           .\uninstall-autostart.ps1"
}
else {
    $startupFolder = [Environment]::GetFolderPath("Startup")
    $shortcutPath = Join-Path $startupFolder "$TaskName.lnk"

    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $Exe
    $shortcut.Arguments = $Arguments
    $shortcut.WorkingDirectory = $WorkDir
    $shortcut.Description = "Starts $(Split-Path $Exe -Leaf) at logon"
    $shortcut.Save()

    Write-Host "Done. Shortcut created:" -ForegroundColor Green
    Write-Host "  $shortcutPath -> $Exe $Arguments"
    Write-Host "It will run as a normal user (not elevated) at every logon."
    Write-Host "Remove it:  .\uninstall-autostart.ps1 -Method Shortcut"
}
