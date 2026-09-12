# build-portable.ps1
# Build a portable (green) output locally and create a desktop shortcut.
# Usage:
#   powershell -ExecutionPolicy Bypass -File build-portable.ps1
# Requires compiled artifacts under src-tauri/target/release (run: npx tauri build).

$ErrorActionPreference = 'Stop'

# Resolve project root (works with powershell -File and pwsh -File)
if ($PSScriptRoot) {
  $root = $PSScriptRoot
} else {
  $root = Split-Path -Parent $PSCommandPath
}
Set-Location $root

$src = Join-Path $root 'src-tauri\target\release'
# Read as UTF-8 bytes so non-ASCII content in the JSON does not break parsing
$confJson = [System.IO.File]::ReadAllText((Join-Path $root 'src-tauri\tauri.conf.json'), [System.Text.Encoding]::UTF8)
$ver = ($confJson | ConvertFrom-Json).version

Write-Host "Version: $ver"

$exe = Join-Path $src 'dbclient.exe'
$wv2 = Join-Path $src 'WebView2Loader.dll'
if (-not (Test-Path $exe)) { throw "Missing $exe . Run first: npx tauri build" }

# 1) portable output folder
$portDir = Join-Path $root "dist-portable\dbclient-$ver-win-x64"
New-Item -ItemType Directory -Force $portDir | Out-Null
Copy-Item $exe $portDir -Force
if (Test-Path $wv2) { Copy-Item $wv2 $portDir -Force }

# 2) zip archive
$zip = Join-Path (Join-Path $root 'dist-portable') "dbclient_${ver}_portable_win-x64.zip"
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path (Join-Path $portDir '*') -DestinationPath $zip
Write-Host "Portable dir: $portDir"
Write-Host "Portable zip: $zip"

# 3) desktop shortcut
$desktop = [Environment]::GetFolderPath('Desktop')
$lnk     = Join-Path $desktop "dbclient.lnk"
$ws = New-Object -ComObject WScript.Shell
$sh = $ws.CreateShortcut($lnk)
$sh.TargetPath       = $exe
$sh.WorkingDirectory = $src
$sh.Description      = "DBClient $ver (portable)"
$sh.Save()
Write-Host "Desktop shortcut: $lnk  ->  $exe"
