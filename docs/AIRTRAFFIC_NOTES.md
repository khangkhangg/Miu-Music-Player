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

## Loading the DLLs from the Store package (F1.1) — 2026-10-20

- `LoadLibraryExW` on `…\WindowsApps\AppleInc.iTunes_…\CoreFoundation.dll` from our (unpackaged) process fails
  with **error 5 (access denied)**, although `BUILTIN\Users` has RX on the file. The package files carry the
  `S-1-19-512-4096` trust label; Windows blocks loading package binaries into non-package processes.
  Confidence: high (reproduced with plain `LoadLibraryExW`).
- **A private copy loads fine.** Copying the DLLs out and loading them with
  `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS` works for all three.
  Needed set (transitive closure of imports found in the package, x64): `AMDS64\{MobileDevice, AirTrafficHost}.dll`
  + root `CoreFoundation.dll, objc.dll, ASL.dll, libdispatch.dll, libicuin.dll, libicuuc.dll, icudt62.dll,
  CFNetwork.dll, pthreadVC2.dll, zlib1.dll, libxml2.dll, SQLite3.dll` (~40 MB). VC runtime comes from the system.
- The **Apple Mobile Device service from the Store package is running** (listening on `127.0.0.1:27015`), so the
  copied `MobileDevice.dll` should reach the phone through it. Not yet verified with a device (F1.3).
- Decision (user, 2026-10-20): **private copy** into `%LOCALAPPDATA%\MiuMusic\apple\<package folder name>\`, refreshed
  when the package version changes. Classic `Common Files\Apple` installs are loaded in place. DLLs are never shipped.

## Published signatures (research, 2026-10-20)

Gathered by web research; nothing below is verified against our DLLs yet except that every name exists in our export
map. Reading notes for our project:
- The only published `ATHostConnection*` prototypes come from the **airlift** family of repos (macOS ObjC + Windows
  ctypes ports) which drive a **"Book"** sync. Our **"Media"** (music) flow is assumed analogous but unproven.
- The Grappa findings (S6) concern **iOS 27.x**. Our device runs **iOS 12.5.8**; whether it demands Grappa is unknown
  (the 57-byte `integrity` blob in its DB suggests some signing happens) — find out in F2.2/F2.3 by logging messages.
- Calling convention: `cdecl` (single convention on x64). All `*Ref` types are opaque pointers.

Sources:
- S1 airlift `cleanroom/host.m` — https://raw.githubusercontent.com/0xjohnnydev/airlift/main/cleanroom/host.m
- S2 airlift `archive/original/Sources/airtraffic_host.m` — https://raw.githubusercontent.com/0xjohnnydev/airlift/main/archive/original/Sources/airtraffic_host.m
- S3 AirCard-Windows `aircard/airtraffic.py` (x64 ctypes) — https://raw.githubusercontent.com/alrrstd/AirCard-Windows/main/aircard/airtraffic.py
- S4 airlift-windows `airlift.py` (ctypes) — https://github.com/DahanGuy/airlift-windows
- S5 ios-deploy `MobileDevice.h` — https://raw.githubusercontent.com/ios-control/ios-deploy/master/src/ios-deploy/MobileDevice.h
- S6 AirliftSilence README (Grappa) — https://github.com/YiHoooong/AirliftSilence
- S7 opendll AirTrafficHost export list (names only; page not fetchable directly)

Confidence: high = 2+ independent sources agree; medium = one credible source; low = name only / inferred.

### A) AirTrafficHost.dll

| Function | Prototype | Sources | Confidence | Notes |
|---|---|---|---|---|
| ATHostConnectionCreate | `ATHostConnectionRef ATHostConnectionCreate(CFStringRef udid)` | S1–S4 | high | UDID as CFString |
| ATHostConnectionCreateWithLibrary | not found | S7 | low | maybe `(CFStringRef udid, CFTypeRef library)` — do not rely on |
| ATHostConnectionRelease | `void ATHostConnectionRelease(ATHostConnectionRef)` | S1–S4 | high | |
| ATHostConnectionInvalidate | not found | S7 | low | probably `void (ATHostConnectionRef)` |
| ATHostConnectionReadMessage | `CFDictionaryRef ATHostConnectionReadMessage(ATHostConnectionRef)` | S1–S4 | high | blocking; caller CFReleases |
| ATHostConnectionSendHostInfo | `void ATHostConnectionSendHostInfo(ATHostConnectionRef, CFDictionaryRef hostInfo)` | S1–S4 | high | |
| ATHostConnectionSendSyncRequest | `void ATHostConnectionSendSyncRequest(ATHostConnectionRef, CFArrayRef dataclasses, CFDictionaryRef anchors, CFDictionaryRef hostInfo)` | S1–S4 | high | e.g. `["Book"]`, anchors `{}` |
| ATHostConnectionSendMetadataSyncFinished | `void ATHostConnectionSendMetadataSyncFinished(ATHostConnectionRef, CFDictionaryRef syncTypes, CFDictionaryRef anchors)` | S1–S4 | high | e.g. `{Book: 1}` |
| ATHostConnectionSendAssetCompleted | `void ATHostConnectionSendAssetCompleted(ATHostConnectionRef, CFStringRef assetId, CFStringRef dataclass, CFStringRef devicePath)` | S1–S4 | high | after the file is staged over AFC |
| ATHostConnectionSendFileBegin / SendFileProgress | not found | S7 | low | airlift never calls them |
| ATHostConnectionGetGrappaSessionId | not found | S6, S7 | low | S6: returns 0 when no session |
| ATHostConnectionSendMessage / SendPowerAssertion / GetCurrentSessionNumber | not found | S7 | low | |
| ATCFMessageGetName | `CFStringRef ATCFMessageGetName(CFDictionaryRef msg)` | S1–S4 | high | |
| ATCFMessageGetParam | `CFTypeRef ATCFMessageGetParam(CFDictionaryRef msg, CFStringRef key)` | S1–S4 | high | |
| ATCFMessageCreate / GetSessionNumber | not found | — | low | |

### B) MobileDevice.dll

`mach_error_t` / `afc_error_t` are 32-bit int, 0 = success.

| Function | Prototype | Sources | Confidence | Notes |
|---|---|---|---|---|
| AMDeviceNotificationSubscribe | `mach_error_t (am_device_notification_callback cb, unsigned u0, unsigned u1, void *ctx, am_device_notification **out)` | S1, S5 | medium | S5/S1 use `…WithOptions` (+ trailing `CFDictionaryRef`, pass NULL) |
| callback | `void (*)(struct am_device_notification_callback_info *info, void *ctx)` | S5 | high | |
| am_device_notification_callback_info | `{ struct am_device *dev; unsigned int msg; }` packed | S5 | high | msg 1 = connected, 2 = disconnected, 3 = unsubscribed |
| AMDeviceConnect / Disconnect / ValidatePairing / StartSession / StopSession | `mach_error_t (struct am_device *)` | S1, S5 | high | |
| AMDeviceIsPaired | `int (struct am_device *)` | S1, S5 | high | |
| AMDeviceCopyValue | `CFTypeRef (struct am_device *, CFStringRef domain /*NULL*/, CFStringRef key)` | S1, S5 | high | caller releases |
| AMDeviceCopyDeviceIdentifier | `CFStringRef (struct am_device *)` | S1 | medium | UDID |
| AMDeviceSecureStartService | `mach_error_t (struct am_device *, CFStringRef service, CFDictionaryRef opts, ServiceConnRef *out)` | S1 | high | modern path, returns AMDServiceConnection |
| AMDeviceStartService | `mach_error_t (struct am_device *, CFStringRef service, ServiceConnRef *out, unsigned *unknown)` | S5 | medium | older path |
| AMDServiceConnectionGetSocket | `int (ServiceConnRef)` | S1, S5 | high | |
| AFCConnectionOpen | `afc_error_t (ServiceConnRef or int socket, unsigned io_timeout, AFCConnectionRef *out)` | S1, S5 | high | **first arg depends on start-service path** — ServiceConnRef (8 bytes on x64) with SecureStartService |
| AFCDirectoryOpen | `afc_error_t (AFCConnectionRef, const char *path, afc_directory **dir)` | S1, S5 | high | |
| AFCDirectoryRead | `afc_error_t (AFCConnectionRef, afc_directory *, char **entry)` | S1, S5 | high | NULL entry = end |
| AFCDirectoryClose | `afc_error_t (AFCConnectionRef, afc_directory *)` | S1, S5 | high | |
| AFCFileRefOpen | `afc_error_t (AFCConnectionRef, const char *path, uint64 mode, afc_file_ref *ref)` | S1, S5 | high | mode 1 read, 2 write, 3 rw |
| AFCFileRefRead | `afc_error_t (AFCConnectionRef, afc_file_ref, void *buf, size_t *len)` | S1, S5 | high | len in/out, 8 bytes on x64 |
| AFCFileRefWrite | `afc_error_t (AFCConnectionRef, afc_file_ref, const void *buf, size_t len)` | S1, S5 | high | |
| AFCFileRefClose | `afc_error_t (AFCConnectionRef, afc_file_ref)` | S1, S5 | high | |
| AFCRemovePath | `afc_error_t (AFCConnectionRef, const char *path)` | S1, S5 | high | |
| AFCRenamePath | `afc_error_t (AFCConnectionRef, const char *from, const char *to)` | S5 | medium | |
| AFCFileInfoOpen / AFCKeyValueRead / AFCKeyValueClose | `(AFCConnectionRef, const char *, afc_dictionary **)` / `(afc_dictionary *, char **key, char **val)` / `(afc_dictionary *)` | S1 | high | |

### C) CoreFoundation on Windows
- Notification callbacks are delivered through the CF run loop: run `CFRunLoopRun`/`CFRunLoopRunInMode` on the
  subscribing thread (S1, macOS — verify on Windows). Confidence: medium.
- Build CFStrings with `CFStringCreateWithCString(NULL, s, kCFStringEncodingUTF8)` and release them (S3, S4). Medium.
- No source calls `__CFInitialize` manually. Medium.

### Message flow seen in sources (Book sync, S1–S4)
1. `ATHostConnectionCreate(udid)` → read messages until **`SyncAllowed`** (`SyncFailed` = abort).
2. `SendHostInfo(hostInfo)` → `SendSyncRequest(["Book"], {}, hostInfo)` → read until **`ReadyForSync`**.
3. `SendMetadataSyncFinished({Book: 1}, {})` → read **`AssetManifest`** (param `AssetManifest`: dataclass → array
   of `{AssetID, IsDownload}`) until **`SyncFinished`** / `SyncFailed`.
4. Copy each file over **AFC**, then `SendAssetCompleted(assetId, "Book", devicePath)`; finally `Release`.

Grappa (S6, iOS 27.x): the device advertises `GrappaSupportInfo` and fails with `SyncFailed {ErrorCode: 4}` without a
Grappa session; the DLL does not create one by itself. Mechanism unpublished.

### Open questions
1. Prototypes still unknown: `CreateWithLibrary`, `GetGrappaSessionId`, `Invalidate`, `SendFileBegin`, `SendFileProgress`,
   `SendMessage`, `SendPowerAssertion`, `GetCurrentSessionNumber`, `ATCFMessageCreate`, `ATCFMessageGetSessionNumber`.
2. Does a Media sync stream files with `SendFileBegin`/`Progress`, or stage over AFC then `SendAssetCompleted`?
3. `HostInfo`, `SyncRequest` and anchor contents for the `"Media"` dataclass.
4. Plain `AMDeviceNotificationSubscribe` (exported) vs `…WithOptions`: check arg count on 1860.2.5.1.
5. `AFCConnectionOpen` first argument with `AMDeviceSecureStartService` on our DLL.
6. CF run loop needed for notifications on Windows?
7. Does iOS 12.5.8 require Grappa?

## Device discovery (F1.3) — 2026-10-20, iPhone 5s iOS 12.5.8 over USB

- `AMDeviceNotificationSubscribe(cb, 0, 0, ctx, &handle)` (5 args) returns 0 with the private-copy DLL. Callbacks
  arrive on our subscribing thread while it runs `CFRunLoopRun()`; we never tested without the run loop
  (answers open question 4; question 6 stays open). Confidence: high (works).
- `am_device_notification_callback_info` read as packed `{ device*, u32 msg }`: msg 1 arrived for the plugged phone.
- `AMDeviceCopyDeviceIdentifier` → 24+ char UDID string; `AMDeviceGetInterfaceType` → 1 for USB.
- Trust check `AMDeviceConnect` → `AMDeviceIsPaired` → `AMDeviceValidatePairing` → `AMDeviceDisconnect` reports
  trusted = true for this PC.
- The first "connected" event comes **~11 s after subscribing**; the delay is in the notification itself
  (trust check measured at 148-261 ms). Expect an 11 s "No device" moment at app start.
- Unplug / re-plug verified 2026-10-20 (17:51-17:52Z): Disconnected (msg 2) arrives within the same millisecond
  as the unplug is processed; re-plug Connected arrives a few seconds after plugging in; trust check 148 ms.
  An earlier "verification" coincided with dev-mode app restarts, which reset the card the same way — check logs.
