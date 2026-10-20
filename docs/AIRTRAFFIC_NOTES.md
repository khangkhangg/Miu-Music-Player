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

## Device info (F1.4) — 2026-10-20

- Sequence that works: `AMDeviceConnect` → `IsPaired` / `ValidatePairing` → `AMDeviceStartSession` →
  `AMDeviceCopyValue(dev, NULL, NULL)` (whole default domain as a dict) and
  `AMDeviceCopyValue(dev, "com.apple.disk_usage", NULL)` → `StopSession` → `Disconnect`. Whole step: ~250 ms.
  Confidence: high (machine test against the iPhone 5s).
- Keys used: `DeviceName` ("khang’s iPhone", note U+2019), `ProductType` (iPhone6,1), `ProductVersion` (12.5.8);
  disk: `TotalDataCapacity` 27,687,563,264, `TotalDataAvailable` 25,046,335,488 (fallback `AmountDataAvailable`).
- Song count on the phone needs the media DB → comes with F1.6.

## AFC (F1.5) — 2026-10-20, iPhone 5s iOS 12.5.8

- Open: `AMDeviceConnect` → `ValidatePairing` → `StartSession` → `AMDeviceSecureStartService(dev, "com.apple.afc", NULL, &svc)`
  → `StopSession` → `Disconnect`; then `AFCConnectionOpen(AMDServiceConnectionGetSocket(svc), 0, &conn)`.
  **Answer to open question 5:** passing the `svc` object itself makes `AFCConnectionOpen` return 0 but every request
  then fails with AFC error 1. The raw socket works. Confidence: high (machine tests, both variants tried).
- Media root `/` lists `iTunes_Control`; `/iTunes_Control/iTunes/` holds `MediaLibrary.sqlitedb`.
- `AFCFileRefOpen` mode 1 = read, mode 3 = write/create/truncate (libimobiledevice constants; the research table's
  "2 write, 3 rw" was wrong for our use). `AFCDirectoryCreate(conn, path)` works. UTF-8 paths with Vietnamese
  characters work. 300 KB write/read round trip byte-identical.

## Music DB backup (F1.6) — 2026-10-20

- Read over AFC from `/iTunes_Control/iTunes/`: `MediaLibrary.sqlitedb` 557,056 B, `-shm` 32,768 B,
  `-wal` 7,622,032 B. The WAL is ~14x the main file: most recent writes (3uTools sync) still live in the WAL,
  confirming the schema note "always read with the WAL present".
- Opening a copy with all three files: `SELECT count(*) FROM item WHERE media_type = 8` = **69** (matches the device).
- Baseline: `%APPDATA%\MiuMusic\backups\e51e30a5\20261019-180954\` and a copy in `research/db-baseline/` (gitignored).
- Tools: `device-backup` (read-only), `db-diff A B`, `device-restore <dir> --yes-write-to-phone` (backs up the
  phone first; **never run yet** — needs the user's go-ahead each time).

## ATC message logger (F2.2) — 2026-10-20, read-only (nothing sent)

Call sequence used (Rust, `apple/airtraffic.rs`, tool `cargo run --bin atc-log`):
1. MobileDevice notifications running (`apple::device::watch`, CF run loop on its own thread), phone trusted.
2. `ATHostConnectionCreate(CFString udid)` → non-null connection. CFString released right after. Confidence: high.
3. Reader thread loops `ATHostConnectionReadMessage(conn)` (blocking, +1 dict, released by us) and
   `ATCFMessageGetName(msg)` (borrowed). Confidence: high.
4. Stop: `ATHostConnectionInvalidate(conn)` unblocks the pending read; then `ATHostConnectionRelease(conn)`.
   Test run finished cleanly. Confidence: medium (works; whether Invalidate *always* unblocks is unproven).

Messages the phone sends on connect, **without any host info or sync request** (2 runs, ~230-250 ms after Create):

| # | Command | Params (shape) |
|---|---|---|
| 1 | `InstalledAssets` | `Application._Deleted`: [] |
| 2 | `AssetMetrics` | per dataclass `_Count`, `_PhysicalSize` (+`_PurgeableSize`): Application, Book, Logs, **Media.Music** (69 items, 851,976,192 B), Photo.CameraRoll, Photo.Photo, Proofing, Ringtone, UserData, VoiceMemo; totals `_FreeSize`, `_PhysicalSize`, `_PurgeableSize` |
| 3 | `SyncAllowed` | AppInstallationAllowed, AppRemovalAllowed, **AutoSync false**, DataProtected false, **DisabledAssetTypes [Movie, TVEpisode, HomeVideo, Photo]**, EducationModeEnabled, **ManualSync true**, PurgeAllowed true |

Every message also has `Command`, `Id` (748, 749, 750 — a phone-side counter that persists across sessions, so
earlier syncs used this channel), `Session` = 1, `Type` = 0.

Conclusions:
- Music sync is allowed (Music not in DisabledAssetTypes); the phone is in manual-management mode.
- **No `GrappaSupportInfo`** in SyncAllowed on iOS 12.5.8 → the Grappa requirement seen on iOS 27 (S6) very likely
  does not apply here (open question 7: probably no; confirm in F2.3).
- After SyncAllowed the phone goes quiet: it waits for the host (HostInfo, then SyncRequest — F2.3).
- Captures: `research/captures/*-atc.jsonl` (gitignored).

## Side effect of a read-only ATC connection — 2026-10-20

Backups at 18:09:54Z and 18:21:50Z bracket the F2.2 capture runs. db-diff: only `_MLDatabaseProperties` changed
(5 rows replaced): `MLUbiquitousBookmarkEntityRevisionAnchor` (390 → 390), `MPUbiquitousBookmarkDomainVersionAnchorToken`
(0 → 0), `MPUbiquitousBookmarkDomainVersionRemoteToken` (0 → 0), `MPDateToSyncWithUbiquitousStore` (private), and
**`MPDateLastSynced` 814114539 → 814126261 = 2026-10-19 18:11:01 UTC**, the moment the capture test ran.
So opening an ATC connection (without HostInfo or SyncRequest) makes iOS record a sync. Songs untouched (69).
Confidence: high on the change, medium on the cause (timing matches; nothing else touched the phone).

## Media sync handshake (F2.3) — 2026-10-20 18:27Z, run once with approval

Sent exactly the payloads in `docs/plans/F2.3-sync-handshake.md` (LibraryID `31140337351CE5CE`, kept in settings).

| ms | dir | message |
|---|---|---|
| 515 | ← | InstalledAssets |
| 516 | ← | AssetMetrics |
| 516 | ← | SyncAllowed (Id 758) |
| 519 | → | HostInfo |
| 3564 | → | SyncRequest(["Media"], {}, HostInfo) |
| 3584 | ← | **SyncFailed { ErrorCode: 4 }** (Id 759) |

- Phone DB **unchanged**: backups 18:27:27Z and 18:27:40Z, db-diff "no differences", 69 → 69 songs
  (not even `MPDateLastSynced` moved this time).
- **ErrorCode 4 = the Grappa failure signature reported in S6** ("Grappa session could not be established").
  Correction to the F2.2 note: `SyncAllowed` has no `GrappaSupportInfo` on iOS 12.5.8, but the phone still
  refuses the sync — it most likely requires a Grappa session in HostInfo. Confidence: medium (same code as S6;
  not yet confirmed from the phone's own log).
- The refusal came 20 ms after the request: the phone checks HostInfo/SyncRequest up front, before any data.

Next questions:
1. Confirm the cause from the phone's syslog (`com.apple.syslog_relay` over the same AFC-style service start).
2. How iTunes / 3uTools obtain the Grappa session on Windows: `ATHostConnectionGetGrappaSessionId` (signature
   unknown), the `CoreFP` component (`HKLM\SOFTWARE\Apple Inc.\CoreFP` exists on this PC), and which HostInfo key
   carries it.
3. If Grappa is out of reach → Phase 2 gate: FB.1 (direct DB write) or FB.2 (player-app Documents).

## Grappa / CoreFP investigation — 2026-10-20 (read-only; nothing sent to the phone)

### Local evidence (our own copies of Apple's DLLs and this PC's registry)
- `AirTrafficHost.dll` (x64) **implements Grappa itself**: strings "Failed to initialize grappa, err=%d",
  "Grappa key could not be established, %d", "Grappa host verify fail: %d", "Grappa could not verify message, sending
  auth error", `GrappaSupportInfo`, and an embedded Apple FairPlay certificate chain for **"GrappaForATH"**.
- It needs **CoreFP** (FairPlay): it contains the ASCII registry path `Software\Apple Inc.\CoreFP`, the name
  `CoreFP.dll`, and the identifier `LibraryPath` (most likely the value it reads; medium confidence). It does not
  import CoreFP.dll statically, so it loads it at run time.
- `iTunes.exe` imports `ATHostConnectionGetGrappaSessionId` → iTunes asks ATH for the session.
- ATH also logs "Creating ATHost Connection to device %@, library %@" → `ATHostConnectionCreateWithLibrary` takes a
  library identifier. Other protocol vocabulary in the DLL: `RequestingSync`, `ReadyForSync`, `DataclassAnchors`,
  `Anchor`, `Capabilities`, `DeviceInfo`, `FinishedSyncingMetadata`, `AssetManifest`, `FileBegin/Progress/Complete/Error`,
  `SyncStatus`, `PowerAssertion`, `Ping`/`Pong`, `LocalCloudDownload*`.
- **This PC's CoreFP key was rewritten by 3uTools**:
  `HKLM\SOFTWARE\Apple Inc.\CoreFP` → `Libi4CFPath` / `LibiiiiPath` = `C:\Program Files\3uTools9\itunesDll\CoreFP.dll`
  (WOW6432Node: same names, 3uJailbreak folder). **That file does not exist.** There is no `LibraryPath` value.
  The Store iTunes package ships a genuine Apple-signed x64 `CoreFP.dll` 2.20.6 (36.7 MB) but, being packaged,
  cannot register it in the real HKLM.
- **Working hypothesis (H1):** ATH can't find CoreFP → Grappa init fails → HostInfo has no valid Grappa → the phone
  answers `SyncFailed {ErrorCode: 4}`. Consistent with everything above; not yet confirmed by the phone's log.

### Web research summary (background agent; see sources)
- Grappa = per-sync FairPlay credential required by ATC; on Windows produced via CoreFP.dll, located through
  `HKLM\SOFTWARE\Apple Inc.\CoreFP`. Projects report that i4Tools/3uTools overwrite that key with their own value
  names, often pointing to a missing DLL, and that the fix was installing **standalone x64 iTunes from apple.com**
  instead of the Store version. (https://github.com/BB0813/AirCard-Windows/releases/tag/v1.3.0 — medium)
- Grappa gating reported on iOS 16–27; nothing specific on iOS 12 (low).
- `ATHostConnectionCreateWithLibrary(CFStringRef, CFStringRef, int)` per a Python binding
  (https://github.com/peter158/MobileDevice-1/blob/master/AirTrafficHost.py — high for arg count, low for meaning);
  working forks use it plus a `RequestingSync` flow.
- No open-source Grappa or `item_extra.integrity` generator exists (high). Some projects ship **pre-generated Grappa
  tokens**; we will not use those (they bypass Apple's authentication rather than using Apple's own CoreFP).
- FB.1 (direct DB writes) is likely a dead end: `integrity` needs a FairPlay signature (medium).

### Tools ready
- `device::syslog` (parser, unit-tested) + `apple::syslog_relay` (live reader, machine-tested: 91 lines in 5 s,
  read-only). Can capture `atc`'s own error during a handshake.

### Options (need the user's decision; none executed)
1. Repeat the identical F2.3 handshake once **with the phone's log captured** → confirm H1 from `atc`'s message.
2. **Private CoreFP, process-local:** copy the package's CoreFP.dll into `%LOCALAPPDATA%\MiuMusic\apple\…` (same as
   the other DLLs) and make it findable *for our process only* (process PATH / pre-load). No system change. Then a
   handshake attempt. Unproven: depends on whether ATH falls back to loading `CoreFP.dll` by name.
3. **Registry value** `HKLM\SOFTWARE\Apple Inc.\CoreFP\LibraryPath` → our private CoreFP copy. System-wide change,
   needs admin; leaves 3uTools' values alone. The user would have to make this change.
4. **Install standalone x64 iTunes from apple.com** (the fix reported by other projects); the user would install it.

## Grappa confirmation runs — 2026-10-20 (approved; backups before/after each, DB unchanged both times)

**Run 1 (18:43:50Z) — identical F2.3 handshake + phone syslog** (`research/captures/20261019-184350-*`):
- Phone log: `atc(AirTrafficDevice)[33] <Error>: Grappa session could not be established. Aborting` →
  `SyncFailed { ErrorCode = 4 }`. **Cause confirmed: Grappa.** Confidence: high.
- The phone's first message is `Capabilities` with `GrappaSupportInfo = { deviceType = 0; protocolVersion = 1;
  version = 1; }` — AirTrafficHost consumes it internally (our reader never sees it). So iOS 12.5.8 *does* advertise
  and require Grappa; the F2.2 note "no GrappaSupportInfo" was wrong (it is in Capabilities, not SyncAllowed).
- On the wire: `HostInfo` (ATH adds `LocalCloudSupport = 0`), then `RequestingSync { Dataclasses = (Media);
  DataclassAnchors = {}; HostInfo = … }`. **No Grappa field** → ATH produced no Grappa session.
- The phone shows **two host links** (`0x1391340f0` and ours); the other one also gets InstalledAssets/
  AssetMetrics/SyncAllowed/Ping — another program on this PC (3uTools service or an Apple helper) is connected.
- db-diff before/after: no differences; 69 songs.

**Run 2 (18:46:50Z) — same, with `--private-corefp`** (package CoreFP.dll 2.20.6 + imports copied to the private
folder, first on this process's PATH, pre-loaded before AirTrafficHost):
- CoreFP loads fine from the private copy, but the phone logs the same Grappa failure; HostInfo still has no Grappa.
  → AirTrafficHost does **not** fall back to a CoreFP that is merely loaded / on PATH; it most likely needs the
  registry value (`HKLM\SOFTWARE\Apple Inc.\CoreFP` → probably `LibraryPath`). Confidence: medium.
- db-diff: no differences; 69 songs.

Also fixed: `AMDServiceConnectionReceive` returns a 32-bit `int`; declared as 64-bit, `-1` became 4294967295 and
the syslog reader thread panicked at shutdown (lines were already saved).

## Incident: `CoreFP\LibraryPath` registry value broke Apple's device service — 2026-10-20 (resolved)

What happened:
1. ~18:50Z the user added (on my suggestion, option 3 above)
   `HKLM\SOFTWARE\Apple Inc.\CoreFP` → `LibraryPath` = our private copy
   `%LOCALAPPDATA%\MiuMusic\apple\AppleInc.iTunes_…\CoreFP.dll`.
2. Apple's device service (`AppleMobileDeviceProcess`, Store iTunes package, port 27015) stopped. Our tool then got
   `AMDeviceNotificationSubscribe failed: 0xe8000063` (no service) and exited **before contacting the phone**.
3. After opening iTunes the service ran again, but **iTunes and our code no longer saw the iPhone**, even after
   reinstalling iTunes (the value lives in the real HKLM, which a Store reinstall does not touch).
4. User deleted the value (`reg delete … /v LibraryPath /f`) and restarted → iTunes shows the phone again.

Verified read-only afterwards (19:05Z): registry back to 3uTools' two values only; service listening (pid 18936);
our discovery sees the trusted iPhone 5s (in 0.4 s); fresh backup `20261019-190509` = 69 songs, db-diff vs
`20261019-184707`: no differences. Phone data was never at risk (nothing was sent in this attempt).

Lessons:
- Apple's own Store-packaged device stack **also reads `HKLM\…\CoreFP\LibraryPath`** — pointing it at a copy outside
  the package breaks device detection for everything on the PC. Option 3 is ruled out.
- The approved "run 3" never reached the phone; it is still unused.
- New hard rule in CLAUDE.md: Miu never asks the user to change machine-wide Apple registry keys or services.

Remaining paths for Grappa (decision pending): install standalone x64 iTunes from apple.com (registers CoreFP the
supported way), investigate `ATHostConnectionCreateWithLibrary`, or Phase 2 gate → fallbacks.

## 2026-10-20 — Sync spec review and option A probe

The user's spec (static analysis of iTunes 12139.11001.62009.0) confirms two obstacles for our own
ATC engine: Grappa (session authentication) and the `.cig` signature on each `Sync_XXXXXXXX.plist`
plan, computed inside iTunes.exe by an obfuscated routine that no DLL exports. We do not lift,
emulate or forge either (CLAUDE.md, the spec's legal note). Chosen path: **option A**, drive iTunes.

Probe (read-only, no sync): the Store build declares a packaged COM ExeServer
`iTunes.Application` (CLSID `DC0C2640-1415-4644-875C-6F4D769839BA`) in its AppxManifest; it is not
in HKCR, so create it by CLSID. Result: version 12.13.11.1, library 68 tracks, sources Library /
Internet Radio / iTunes Store. iPhone **not plugged in** during the probe, so whether it appears
as an `IITIPodSource` (kind 2) is still open (checklist S.1).

### 2026-10-20 — probe with the iPhone 5s on USB (read-only)
iTunes lists the phone through COM: source **'khang’s iPhone'**, kind 2 (`IITIPodSource`),
29.8 GB, 23.27 GB free, playlists `khang’s iPhone` and `Music` with **69 tracks** (the 3uTools
songs), plus empty Movies / TV Shows / Books / Audiobooks. Methods: `UpdateIPod`, `EjectIPod`,
`GetITObjectIDs`. So option A is viable on this machine.

Two ways forward with option A:
1. **Playlist sync:** mirror liked songs into a library playlist "Miu Music", tell iTunes (once)
   to sync that playlist, then `UpdateIPod`. Risk: iTunes replaces what is on the phone with its
   own library's selection — the 69 3uTools songs would go unless they are also in the playlist.
2. **Manual management:** with "Manually manage music" on for the phone in iTunes, the COM SDK lets
   a host add a file straight to the device playlist (`IITUserPlaylist.AddFile`) and delete device
   tracks one by one, leaving everything else alone. Needs one setting change in iTunes by the user;
   to verify with one test song before relying on it.

### 2026-10-20 — first song onto the phone through iTunes (manual management)
Backup first: `%APPDATA%\MiuMusic\backups\e51e30a5\20261020-040321` (69 songs).
With "Manually manage music" on, `devicePlaylist.AddFile(path)` returns nothing for the phone.
What works is iTunes' own drag-and-drop path:
1. `iTunes.LibraryPlaylist.AddFile(path)` → IITOperationStatus; wait while `InProgress`; take the track.
2. `phoneSource.Playlists.Item(1).AddTrack(track)` → true; the phone's library went 69 → 70.
Track on phone: "Ai Rồi Cũng Sẽ Khác" (Hà Nhi), kind "MPEG audio file". The 69 3uTools songs stayed.
Side effect: the song is now also in the PC's iTunes library (68 → 69).
