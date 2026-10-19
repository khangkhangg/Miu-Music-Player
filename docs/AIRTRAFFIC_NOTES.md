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
