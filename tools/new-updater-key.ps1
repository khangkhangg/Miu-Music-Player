# Create the updater signing key once: .\tools\new-updater-key.ps1
# The private key goes OUTSIDE the repo (default %USERPROFILE%\.tauri\miu-music.key); back it up with its password.
# Losing either means installed copies of Miu can never verify another update.
# The public key it prints belongs in src-tauri/tauri.conf.json (plugins.updater.pubkey).
param([string]$KeyPath = "$env:USERPROFILE\.tauri\miu-music.key")
$ErrorActionPreference = 'Stop'
if (Test-Path $KeyPath) { throw "$KeyPath already exists; not overwriting it." }
$dir = Split-Path $PSScriptRoot
while ($dir -and -not (Test-Path "$dir\node_modules\@tauri-apps\cli\tauri.js")) { $dir = Split-Path $dir }
if (-not $dir) { throw 'Tauri CLI not found: run npm install.' }
New-Item -ItemType Directory -Force (Split-Path $KeyPath) | Out-Null

$plain = { param($s) [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($s)) }
$first = & $plain (Read-Host 'Choose a password for the key' -AsSecureString)
$second = & $plain (Read-Host 'Type it again' -AsSecureString)
if ($first -ne $second) { throw 'The two passwords differ; nothing was written.' }
if (-not $first) { throw 'Use a password.' }
node "$dir\node_modules\@tauri-apps\cli\tauri.js" signer generate --ci -p $first -w $KeyPath
if ($LASTEXITCODE -ne 0) { throw "signer generate failed ($LASTEXITCODE)" }
Write-Host "Private key: $KeyPath (back it up with the password). Public key: $KeyPath.pub"
