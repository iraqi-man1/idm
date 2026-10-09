# Velox Download Manager - checks after uninstalling on the clean VM.
#
#   powershell -ExecutionPolicy Bypass -File check-uninstall.ps1 -DownloadsDir <dir used for test downloads>
#
# Prints PASS/FAIL per check and exits with the number of failures.

param([Parameter(Mandatory = $true)][string]$DownloadsDir, [string]$InstallDir = "")

$failures = 0
function Check([string]$name, [scriptblock]$test) {
    if (& $test) { Write-Host "PASS  $name" -ForegroundColor Green } else { Write-Host "FAIL  $name" -ForegroundColor Red; $script:failures++ }
}

# Per-user and per-machine install locations (same list as check-install.ps1).
$dirs = @(
    "$env:LOCALAPPDATA\Programs\Velox Download Manager",
    "$env:LOCALAPPDATA\Velox Download Manager",
    "$env:ProgramFiles\Velox Download Manager"
)
if ($InstallDir) { $dirs = @($InstallDir) }
foreach ($d in $dirs) {
    Check "application files removed: $d" { -not (Test-Path "$d\velox-desktop.exe") }
}
foreach ($k in "HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.veloxdm.host",
               "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\com.veloxdm.host",
               "HKCU:\Software\Mozilla\NativeMessagingHosts\com.veloxdm.host") {
    Check "registry key removed: $k" { -not (Test-Path $k) }
}
Check "autostart entry removed" {
    -not (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -ErrorAction SilentlyContinue)."Velox Download Manager"
}
Check "start menu and desktop shortcuts removed" {
    $places = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs",
              "$env:ProgramData\Microsoft\Windows\Start Menu\Programs",
              [Environment]::GetFolderPath("Desktop"),
              "$env:PUBLIC\Desktop"
    -not ($places | ForEach-Object { Get-ChildItem $_ -Recurse -Filter "Velox*" -ErrorAction SilentlyContinue })
}
Check "downloaded files preserved" { (Get-ChildItem $DownloadsDir -File -ErrorAction SilentlyContinue).Count -gt 0 }

Write-Host ""
Write-Host "$failures check(s) failed"
exit $failures
