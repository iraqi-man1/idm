# Velox Download Manager - clean-machine installation checks.
#
# Run in PowerShell on a clean Windows 10/11 VM AFTER installing Velox with
# the installer (and with the network cable unplugged for the offline
# checks). It only reads state; it does not change the system.
#
#   powershell -ExecutionPolicy Bypass -File check-install.ps1 [-InstallDir <dir>]
#
# Prints PASS/FAIL per check and exits with the number of failures.

param([string]$InstallDir = "")

$ErrorActionPreference = "Continue"
$failures = 0
function Check([string]$name, [scriptblock]$test) {
    try { $ok = & $test } catch { $ok = $false; $err = $_.Exception.Message }
    if ($ok) { Write-Host "PASS  $name" -ForegroundColor Green }
    else { Write-Host "FAIL  $name $err" -ForegroundColor Red; $script:failures++ }
}

if (-not $InstallDir) {
    $candidates = @(
        "$env:LOCALAPPDATA\Programs\Velox Download Manager",
        "$env:LOCALAPPDATA\Velox Download Manager",
        "$env:ProgramFiles\Velox Download Manager"
    )
    $InstallDir = $candidates | Where-Object { Test-Path "$_\velox-desktop.exe" } | Select-Object -First 1
}
Write-Host "Install directory: $InstallDir"

Check "application installed" { Test-Path "$InstallDir\velox-desktop.exe" }
foreach ($exe in "velox-nmh.exe", "ffmpeg.exe", "ffprobe.exe", "yt-dlp.exe") {
    Check "bundled $exe present" { Test-Path "$InstallDir\$exe" }
}
Check "browser extensions bundled" {
    (Test-Path "$InstallDir\extensions\chromium\manifest.json") -and (Test-Path "$InstallDir\extensions\firefox\manifest.json")
}
Check "third-party notices bundled" { Test-Path "$InstallDir\THIRD_PARTY_NOTICES.md" }

# Bundled tools run from the install directory (no PATH, no Python, offline).
Check "ffmpeg runs" { (& "$InstallDir\ffmpeg.exe" -hide_banner -version | Select-Object -First 1) -match "ffmpeg version" }
Check "ffprobe runs" { (& "$InstallDir\ffprobe.exe" -hide_banner -version | Select-Object -First 1) -match "ffprobe version" }
Check "yt-dlp runs without Python" { (& "$InstallDir\yt-dlp.exe" --version) -match "^\d{4}\.\d{2}\.\d{2}" }
Check "tools run with a minimal PATH (no Python or other runtimes)" {
    $saved = $env:PATH
    try {
        $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
        ((& "$InstallDir\yt-dlp.exe" --version) -match "^\d{4}") -and
            ((& "$InstallDir\ffmpeg.exe" -hide_banner -version | Select-Object -First 1) -match "ffmpeg")
    } finally { $env:PATH = $saved }
}

# WebView2 runtime (installed offline by the installer when missing).
Check "WebView2 runtime installed" {
    $keys = @(
        "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
    )
    ($keys | Where-Object { (Get-ItemProperty $_ -ErrorAction SilentlyContinue).pv }).Count -gt 0
}

# Native messaging registration (written by the installer hook).
$hosts = @{
    "Chrome"  = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.veloxdm.host";
    "Edge"    = "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\com.veloxdm.host";
    "Firefox" = "HKCU:\Software\Mozilla\NativeMessagingHosts\com.veloxdm.host"
}
foreach ($b in $hosts.Keys) {
    Check "native messaging host registered for $b" {
        $manifest = (Get-ItemProperty $hosts[$b] -ErrorAction Stop).'(default)'
        $json = Get-Content $manifest -Raw | ConvertFrom-Json
        (Test-Path $json.path) -and ($json.name -eq "com.veloxdm.host")
    }
}

Write-Host ""
Write-Host "$failures check(s) failed"
exit $failures
