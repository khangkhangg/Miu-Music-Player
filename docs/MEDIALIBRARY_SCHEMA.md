# MediaLibrary.sqlitedb — findings (iPhone 5s, iOS 12.5.8)

Analyzed from a real device database containing 69 MP3s synced by 3uTools.
Device path (AFC media root): `/iTunes_Control/iTunes/MediaLibrary.sqlitedb` (+ `-wal`, `-shm`).
Always read with the WAL present — most recent writes live in `-wal`.

## Database facts
- `PRAGMA user_version` = **1610030**, journal mode **WAL**.
- **No custom collations or SQL functions** in tables/indexes → stock SQLite can open and write it.
- 65 indexes, 5 triggers (all on `item_store`, they derive `item.in_my_library` and `item.remote_location_id`).
- `_MLDatabaseProperties` holds sync anchors, e.g. `MLSyncClientLastSyncedRevision` (= latest `entity_revision.revision`),
  `MPDateLastSynced`, `MLSyncClientGenerationID`. It also contains the Apple account ID → treat file as private.

## What one synced song writes
| Table | Key fields observed (non-zero) |
|---|---|
| `item` | `item_pid` (random int64), `media_type=8` (music), `title_order`/`_section`, `item_artist_pid`, `item_artist_order`, `album_pid`, `album_order`, `album_artist_pid`, `album_artist_order`, `genre_id`, `genre_order`, `disc_number`, `track_number`, `base_location_id`, `keep_local=1`, `in_my_library=1` (trigger), `date_added` |
| `item_extra` | `title`, `sort_title`, `total_time_ms` (float), `location` (`ABCD.mp3`), `file_size`, **`integrity`** (57-byte blob), `date_modified`, `media_kind=1`, `location_kind_id=42` |
| `item_store` | `sync_id` (random int64), `sync_in_my_library=1` |
| `item_playback` | `audio_format=301` (MP3), `bit_rate`, `sample_rate` |
| `item_stats` | `date_accessed` (written by device) |
| `item_search` | `search_title`, `search_album`, `search_artist`, `search_album_artist` = the matching `*_order` values |
| `item_video`, `lyrics`, `chapter` | one empty row each (just `item_pid`) |
| `album` | `album`, `sort_album`, `album_artist_pid`, `representative_item_pid`, `grouping_key`, `keep_local_status=2` |
| `item_artist` / `album_artist` | name, sort name, `grouping_key`, `representative_item_pid` |
| `genre` | `genre`, `grouping_key`, `representative_item_pid` |
| `container_item` | row linking `item_pid` → the device's main container ("<name>'s iPhone", `distinguished_kind=0`), with `position` |
| `entity_revision` | one row per new entity, incrementing `revision`; `class`: 0=item, 2=item_artist, 4=album, 7=album_artist |
| `sort_map` | `(name, name_order, name_section, sort_key)` for every title/artist/album/genre string |

## Encodings
- **Dates:** seconds since 2001-01-01 UTC (Apple/Cocoa epoch). `unix = apple + 978307200`.
- **File location:** `base_location.path = iTunes_Control/Music/Fnn`, `base_location_id = 3840 + nn`
  (e.g. 3884 → F44). File name = 4 random uppercase letters + extension.
- **`*_order`** = `sort_map.name_order` for that string (sparse uint32-range values preserving alphabetical order).
- **`*_order_section`** = index-bar letter bucket (0 = A … 25 = Z, 26 = #?, 27 = empty). Verify.
- **`sort_key` / `grouping_key`:** look like **ICU collation keys** (levels separated by `0x01`).
  `grouping_key` ≈ primary-strength key (case/diacritic-insensitive), e.g. `Bolero` → `,F@2LF`.
- **`integrity`:** 57 bytes, always starts `04 00`, unique per track. Likely a signature over file/metadata
  produced during the official sync handshake. **Do not forge.** Approach A lets Apple's stack produce it.

## Open questions (resolve in Phase 0/1)
- Does the Music app reject items with NULL / copied `integrity`? (decides whether Approach B is viable)
- Exact ICU version/locale for `sort_key` reproduction (Vietnamese diacritics must sort correctly).
- Section numbering for `#` and non-Latin titles.
