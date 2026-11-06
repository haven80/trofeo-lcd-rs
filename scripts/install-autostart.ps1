<#
.SYNOPSIS
  Adds trofeo_lcd (or trofeo_screen) to Windows startup.

.DESCRIPTION
  Two methods are available:

    -Method Task     (default, recommended) Registers a Scheduled Task that
                      starts the program at logon. This lets it start
                      already ELEVATED ("Run as administrator") without a
                      UAC prompt at every login - needed for the in-game
                      FPS dashboard (fps_monitor) and AMD sensors (PawnIO).
                      A plain Startup-folder shortcut can't do this: it
                      would either fail silently or prompt UAC every time.

    -Method Shortcut  Classic method: a shortcut in the per-user Startup
                      folder (shell:startup). Simpler, no admin rights
                      needed to install, but the program itself then runs
                      WITHOUT elevation (fine if you don't use fps_monitor
                      or PawnIO).

.PARAMETER Exe
  Path to trofeo_lcd.exe (or trofeo_screen.exe). If you leave it out the
  script looks for trofeo_lcd.exe next to itself, then one folder up (the
  release zip keeps the scripts in a "scripts" folder beside the exe), then
  in the folder you are in. Put the path in "quotes" if it has spaces.

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
  .\scripts\install-autostart.ps1

.EXAMPLE
  .\install-autostart.ps1 -Exe "C:\Tools\trofeo_lcd\trofeo_lcd.exe" -Arguments "--hide-console"

.EXAMPLE
  # No admin rights needed, no elevation at startup:
  .\install-autostart.ps1 -Method Shortcut
#>

[CmdletBinding()]
param(
    [string]$Exe = "",
    [string]$Arguments = "--hide-console",
    [ValidateSet("Task", "Shortcut")]
    [string]$Method = "Task",
    [bool]$Elevated = $true,
    [string]$TaskName = "TrofeoLCD"
)

$ErrorActionPreference = "Stop"

# Where this script is. $PSScriptRoot can come out empty in some hosts and ways of starting a
# script (it made "Join-Path" fail on an empty path in version 2.9.x), so it is never used
# in the parameter defaults and there are fallbacks here.
$ScriptPath = $PSCommandPath
if (-not $ScriptPath) { $ScriptPath = $MyInvocation.MyCommand.Path }
$ScriptDir = $PSScriptRoot
if (-not $ScriptDir -and $ScriptPath) { $ScriptDir = Split-Path -Parent $ScriptPath }
if (-not $ScriptDir) { $ScriptDir = (Get-Location).Path }

if (-not $Exe) {
    $places = @($ScriptDir)
    $parent = Split-Path -Parent $ScriptDir
    if ($parent) { $places += $parent }
    $places += (Get-Location).Path
    foreach ($place in $places) {
        $candidate = Join-Path $place "trofeo_lcd.exe"
        if (Test-Path -LiteralPath $candidate) { $Exe = $candidate; break }
    }
}
if (-not $Exe -or -not (Test-Path -LiteralPath $Exe)) {
    Write-Host "trofeo_lcd.exe not found (looked next to this script, one folder up and in the current folder)." -ForegroundColor Red
    Write-Host "Pass its path:  -Exe `"C:\path\to\trofeo_lcd.exe`"   (in quotes if the path has spaces)"
    exit 1
}
$Exe = (Resolve-Path -LiteralPath $Exe).Path
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
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$ScriptPath`"",
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
