# AirTrafficHost / Apple DLL notes

Running log of what we learn about Apple's DLLs (CLAUDE.md rule 3). Each entry: what, where, evidence, confidence.

## DLL locations (F0.3) — 2026-10-20, dev PC (Windows 11 Pro 26300)

Installed: **Microsoft Store iTunes** `AppleInc.iTunes_12139.11001.62009.0_x64__nzyj5cx40ttqa`.
No classic iTunes / Apple Mobile Device Support install; `HKLM\SOFTWARE\Apple Inc.` holds only `CoreFP`.

Package root: `C:\Program Files\WindowsApps\AppleInc.iTunes_12139.11001.62009.0_x64__nzyj5cx40ttqa\`

| File (relative to package root) | Arch | Size | File version |
|---|---|---|---|
| `AMDS64\AirTrafficHost.dll` | x64 | 322,512 | 4026.110.1.6 |
| `AMDS64\MobileDevice.dll` | x64 | 4,035,536 | 1860.2.5.1 |
| `CoreFoundation.dll` | x64 | 2,238,816 | 1750.443.0 |
| `AMDS32\AirTrafficHost.dll` | x86 | 299,984 | 4026.110.1.6 |
| `AMDS32\MobileDevice.dll` | x86 | 3,300,816 | 1860.2.5.1 |
| `VFS\ProgramFilesCommonX86\Apple\Apple Application Support\CoreFoundation.dll` | x86 | 1,736,032 | 1750.443.0 |

Findings:
- **Both 32- and 64-bit copies ship side by side.** The locator checks the PE machine type and only accepts
  DLLs matching our build (x64). Confidence: high (read from PE headers).
- In the Store package the mobile-device DLL is named **`MobileDevice.dll`**; classic installs reportedly use
  `iTunesMobileDevice.dll`. We accept both. Confidence: high for Store, medium for classic (not seen yet).
- **CoreFoundation.dll is not next to the AMDS64 DLLs** (it's at the package root). F1.1 must add both the
  `AMDS64` folder and the CoreFoundation folder to the DLL search path (`AddDllDirectory`) before loading.
  Confidence: high on location, unverified that loading works this way.
- Store packages are found through
  `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages\<full name>`
  → `PackageRootFolder` (no admin needed). Search order: registry `InstallDir` → `Common Files\Apple\…` →
  Store packages (Apple Devices before iTunes). Confidence: high (works on this PC).
- Not yet checked: the **Apple Devices** Store app layout, and whether `WindowsApps` DLLs can be loaded
  from a non-packaged process (expected yes — 3uTools-style tools do this — verify in F1.1).

## Export map (F2.1) — 2026-10-20

Tool: `cargo run --bin dll-exports` (in `src-tauri/`) → full lists in `research/dll-exports/*.txt` (gitignored).
Required sets are pinned in `src-tauri/src/apple/exports.rs` and checked by an `#[ignore]` machine test.

| DLL | Version | Exports | Notable groups |
|---|---|---|---|
| `AirTrafficHost.dll` (x64) | 4026.110.1.6 | 59 | `ATHost*` 42, `ATCFMessage*` 5, `ATProcessLink*` 9 |
| `MobileDevice.dll` (x64) | 1860.2.5.1 | 586 | `AMDevice*` 88, `AFC*` 135, `AMDServiceConnection*` 9, `AMRestorableDevice*` 34 |
| `CoreFoundation.dll` (x64) | 1750.443.0 | 2021 | `CF*` 858 |

**All 9 `ATHostConnection*` names the plan listed as "unverified" exist** (confidence: high, read from the export table):
`CreateWithLibrary`, `SendHostInfo`, `ReadMessage`, `SendSyncRequest`, `SendFileBegin`, `SendAssetCompleted`,
`GetGrappaSessionId`, `Invalidate`, `Release`. Signatures are still unknown — exports carry names only.

Other `AirTrafficHost.dll` exports worth knowing (names only, purpose guessed from the name, confidence low):
- Connection variants: `ATHostConnectionCreate`, `…CreateWithCallbacks`, `…CreateWithQueryCallback`, `…Destroy`, `…Retain`
- Messaging: `ATHostConnectionSendMessage`, `…SendPing`, `…SendStatusMessage`, `…SendPowerAssertion`,
  `…SendConnectionInvalid`, `…GetCurrentSessionNumber`, `ATHostWaitForQueueToDrain`
- Asset flow: `…SendFileProgress`, `…SendFileError`, `…SendFileErrorWithParams`,
  `…SendAssetCompletedWithMetadata`, `…SendAssetMetricsRequest`, `…SendMetadataSyncFinished`, `…SendSyncFailed`
- Message objects: `ATCFMessageCreate`, `ATCFMessageGetName`, `ATCFMessageGetParam`, `ATCFMessageGetSessionNumber`,
  `ATCFMessageVerify` — likely how we read the name/params of what `ReadMessage` returns
- Device side: `ATHostDeviceObserverCreate/Destroy/IsAttached`, `ATHostDeviceNotificationObserverCreate/Destroy`,
  `ATHostCreateDeviceServiceConnection`, `ATHostAMDeviceSocketCreate/Destroy/Write`
- Environment: `ATAddAppleSearchPathsToEnvironmentFromReg` — probably sets the DLL search path from the registry
  (classic installs); may matter for F1.1 with the Store layout.

`MobileDevice.dll` has the classic names we need for F1.3–F1.5: `AMDeviceNotificationSubscribe`, `AMDeviceConnect`,
`AMDeviceIsPaired`, `AMDeviceValidatePairing`, `AMDeviceStartSession`/`StopSession`, `AMDeviceCopyValue`,
`AMDeviceCopyDeviceIdentifier`, `AMDeviceSecureStartService`, `AMDeviceStartService`, `AMDServiceConnectionGetSocket`,
`AFCConnectionOpen`, `AFCDirectoryOpen/Read`, `AFCFileRefOpen/Read/Write/Close`, `AFCRemovePath`, `AFCRenamePath`.

Next (F1.1/F1.2): find published signatures for these (open-source projects that call them), record each with source
and confidence here before writing FFI declarations.
