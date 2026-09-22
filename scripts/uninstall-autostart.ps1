<#
.SYNOPSIS
  Removes the trofeo_lcd autostart entry created by install-autostart.ps1
  (either the Scheduled Task or the Startup-folder shortcut).

.PARAMETER Method
  "Task" (default), "Shortcut", or "Both" to remove whichever exists.

.PARAMETER TaskName
  Name used at install time. Default: "TrofeoLCD".

.EXAMPLE
  .\uninstall-autostart.ps1
.EXAMPLE
  .\uninstall-autostart.ps1 -Method Both
#>

[CmdletBinding()]
param(
    [ValidateSet("Task", "Shortcut", "Both")]
    [string]$Method = "Both",
    [string]$TaskName = "TrofeoLCD"
)

$ErrorActionPreference = "Stop"
$removedSomething = $false

if ($Method -eq "Task" -or $Method -eq "Both") {
    $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if ($task) {
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
        Write-Host "Removed scheduled task '$TaskName'." -ForegroundColor Green
        $removedSomething = $true
    }
}

if ($Method -eq "Shortcut" -or $Method -eq "Both") {
    $startupFolder = [Environment]::GetFolderPath("Startup")
    $shortcutPath = Join-Path $startupFolder "$TaskName.lnk"
    if (Test-Path $shortcutPath) {
        Remove-Item $shortcutPath -Force
        Write-Host "Removed startup shortcut '$shortcutPath'." -ForegroundColor Green
        $removedSomething = $true
    }
}

if (-not $removedSomething) {
    Write-Host "Nothing found for '$TaskName' (no scheduled task, no startup shortcut). Nothing to do." -ForegroundColor Yellow
}
