# Read-only: write iTunes' view of the plugged-in iPhone (persistent ID, name, size per track) as JSON,
# for `sync-guard before --itunes <file>`. Stopgap until S.2's Rust COM client reads this directly.
# Usage: powershell -File tools\itunes-view.ps1 <out.json>
param([Parameter(Mandatory = $true)][string]$Out)
$it = [Activator]::CreateInstance([Type]::GetTypeFromCLSID([Guid]'DC0C2640-1415-4644-875C-6F4D769839BA'))
$phone = $null
foreach ($s in $it.Sources) { if ($s.Kind -eq 2) { $phone = $s } }
if (-not $phone) { Write-Error 'iTunes does not see an iPhone'; exit 1 }
$rows = foreach ($t in $phone.Playlists.Item(1).Tracks) {
  $hi = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int32]$it.ITObjectPersistentIDHigh($t)), 0)
  $lo = [BitConverter]::ToUInt32([BitConverter]::GetBytes([int32]$it.ITObjectPersistentIDLow($t)), 0)
  [pscustomobject]@{ hex = ('{0:X8}{1:X8}' -f $hi, $lo); name = $t.Name; size = $t.Size }
}
ConvertTo-Json @($rows) | Out-File -Encoding utf8 $Out
"iTunes lists $(@($rows).Count) tracks on $($phone.Name) -> $Out"
