# Build a signed release for GitHub: the installer, its .sig and latest.json for the in-app updater.
# Usage (from the repo root): .\tools\build-release.ps1 [-KeyPath <path to miu-music.key>]
# Needs the updater private key (default %USERPROFILE%\.tauri\miu-music.key, else the only C:\Users\*\.tauri\miu-music.key;
# never in the repo),
# docs/release-notes/<version>.md and the ffmpeg sidecar in src-tauri/binaries (see docs/CHECKLIST.md F6.1).
# The key's password is asked for here and only lives in this process's environment during the build.
param([string]$KeyPath = "$env:USERPROFILE\.tauri\miu-music.key")
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$version = (Get-Content "$root\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version
$notes = "$root\docs\release-notes\$version.md"
if (-not (Test-Path $notes)) { throw "Write $notes first." }
if (-not (Test-Path $KeyPath)) {
  # The key may sit under another Windows account's profile on this PC (it was made as duykh, terminals run as khang).
  $found = @(Get-ChildItem 'C:\Users\*\.tauri\miu-music.key' -ErrorAction SilentlyContinue)
  if ($found.Count -ne 1) { throw "No updater key at $KeyPath; pass -KeyPath <path to miu-music.key>." }
  $KeyPath = $found[0].FullName
  Write-Host "Using the updater key at $KeyPath"
}

# The Tauri CLI: this checkout's node_modules, else the nearest one above (worktrees use the main checkout's).
$dir = $root
while ($dir -and -not (Test-Path "$dir\node_modules\@tauri-apps\cli\tauri.js")) { $dir = Split-Path $dir }
if (-not $dir) { throw 'Tauri CLI not found: run npm install.' }
$cli = "$dir\node_modules\@tauri-apps\cli\tauri.js"

$secure = Read-Host "Password for $KeyPath" -AsSecureString
$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content $KeyPath -Raw).Trim()
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
Push-Location $root
try {
  node $cli build
  if ($LASTEXITCODE -ne 0) { throw "tauri build failed ($LASTEXITCODE)" }
} finally {
  Pop-Location
  Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY, Env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD -ErrorAction SilentlyContinue
}

$exe = "$root\src-tauri\target\release\bundle\nsis\Miu Music_${version}_x64-setup.exe"
if (-not (Test-Path "$exe.sig")) { throw "No signature next to $exe." }
$out = "$root\src-tauri\target\release\github\$version"
New-Item -ItemType Directory -Force $out | Out-Null
$asset = "MiuMusic_${version}_x64-setup.exe"
Copy-Item $exe "$out\$asset"
Copy-Item "$exe.sig" "$out\$asset.sig"
node "$root\tools\latest-json.mjs" $version "$exe.sig" $notes "$out\latest.json"
if ($LASTEXITCODE -ne 0) { throw 'latest.json failed' }
Write-Host "Release files for v$version are in $out"
