# Syncing to MTP devices (Android phones, Sony Walkman, newer SanDisk)

Written 2026-10-21 alongside the USB mass-storage player sync (`src-tauri/src/player/`).

## Where things stand

| | Mass storage (USB stick, SD card, MP3 player in "MSC" mode) | MTP (Android, Walkman NW-A/ZX, some SanDisk) |
|---|---|---|
| Found by Miu | Yes: removable drives with a `Music` folder, or any drive the user picks | Probe only (`cargo run --bin mtp-probe`) |
| Sync | Yes (`player::sync`) | No |
| Windows API | Plain file system on a drive letter | Windows Portable Devices (WPD) COM API, `PortableDeviceApi.dll`, in the box on Windows 10/11 |

Many players can do either: SanDisk Clip Jam / Sport Plus and older Walkmans (NWZ-) have a USB-mode
setting (MSC / MTP / Auto). Telling users to pick **MSC** gets them a working sync today.

## Probe result (2026-10-21, this PC)

`player::mtp::probe()` opens every device WPD knows, reads the `DEVICE` object's protocol, walks its
children and reads each storage's name, capacity and free space. It is read-only.

It works: run on this PC it found the Seagate 2 TB USB disk (WPD lists mass-storage drives too, with
protocol `MSC:`) and read its capacity and free space correctly. **No MTP device was plugged in**, so
nothing has been learnt yet about how a phone or Walkman answers. An iPhone shows up in WPD as a PTP
camera by "Apple Inc." (photos only); `mtp::kind` keeps it out, Miu reaches iPhones through Apple's DLLs.

Because no MTP device was available to test against, **MTP copying is not implemented**. Writing to a
phone without being able to watch what it does would break the rule of not guessing silently.

## How MTP differs from a drive

- **Objects, not paths.** Every file and folder has an object id (`o1A3`, opaque, device-chosen). A path
  like `Music\Artist\Album\03 Title.mp3` has to be resolved by listing children level by level
  (`IPortableDeviceContent::EnumObjects` + `WPD_OBJECT_ORIGINAL_FILE_NAME`). Listing big folders is slow,
  so the resolver needs a cache for the run.
- **Writing** is `IPortableDeviceContent::CreateObjectWithPropertiesAndData` (name, size, parent id,
  content type `WPD_CONTENT_TYPE_AUDIO`, format) → an `IStream` to write into → `Commit`. The size must
  be given up front. Folders are created with `CreateObjectWithPropertiesOnly` (`WPD_CONTENT_TYPE_FOLDER`).
- **No reliable rename.** Changing `WPD_OBJECT_ORIGINAL_FILE_NAME` through `IPortableDeviceProperties::SetValues`
  is refused by many devices (Android among them, unverified here). The mass-storage sync writes to
  `name.miu-part`, checks the size and renames; over MTP it has to write to the final name, check the
  size (`WPD_OBJECT_SIZE`), and delete the object if it is wrong.
- **Deleting** is `IPortableDeviceContent::Delete` with `PORTABLE_DEVICE_DELETE_NO_RECURSION`; empty
  folders are deleted the same way once their children are gone.
- **Free space** comes from the storage object (`WPD_STORAGE_FREE_SPACE_IN_BYTES`) — the probe already reads it.
- **Identity.** The PnP id changes with the USB port. A stable player id would be
  `mtp:<WPD_DEVICE_SERIAL_NUMBER>:<storage WPD_OBJECT_PERSISTENT_UNIQUE_ID>`, one player per storage
  (a phone with an SD card has two).
- **Android specifics.** The phone must be unlocked and set to *File transfer* (Android defaults to
  *Charging only*), so the UI needs that hint. Files written over MTP go through Android's MediaProvider,
  so music apps see them at once without a rescan.
- **Speed and exclusivity.** MTP is slower than mass storage (typically 10–30 MB/s) and one transfer at a
  time; Explorer having the device open is fine, but another sync tool may not be.

## What implementing it would take

The sync engine is already written against a `PlayerFs` trait (`player/sync.rs`): `len`, `copy_in`,
`rename`, `remove`, `make_dirs`, `remove_empty_dir`, `free_bytes`. An MTP player needs:

1. `WpdFs` implementing `PlayerFs` over one storage, with a per-run path → object-id cache.
2. A `PlayerFs::can_rename()` (or similar) so `put` writes straight to the final name on MTP, with the
   size check and delete-on-mismatch it already does.
3. Player discovery: `mtp::probe` filtered to `Kind::Player`, one `Player` per storage, merged into the
   `players` list the sidebar shows (same `players` setting, same `player_files` table, same plan).
4. Tests: a fake object store behind `PlayerFs` for the logic; one `#[ignore]` machine test against a real
   phone that copies two songs into a `MiuTest-…` folder and deletes them again.

Estimate: one to two days **with a device on the desk**; most of the risk is in how real devices
answer (rename, folder creation, `WPD_OBJECT_SIZE` after `Commit`), and every finding goes into this file.

## Open questions (fill in when a device is tested)

| Question | Android | Walkman (NW-A) |
|---|---|---|
| Does `SetValues(WPD_OBJECT_ORIGINAL_FILE_NAME)` rename? | ? | ? |
| Is `WPD_OBJECT_SIZE` correct straight after `Commit`? | ? | ? |
| Does deleting a folder with children fail without recursion? | ? | ? |
| Which formats does the stock music app play? | MP3, AAC, FLAC, Opus, Vorbis, WAV | MP3, AAC, FLAC, ALAC, WAV (per Sony) |
