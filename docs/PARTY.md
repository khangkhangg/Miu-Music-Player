# Party mode

Guests scan a QR code on the party screen, see the queue on their phone, add songs from the DJ's
library or paste a link, and vote; the most votes play first. Auto DJ plays the queue: songs are
mixed in, YouTube videos (karaoke) play in YouTube's own embedded player on the party screen and
the music comes back when the video ends. Nothing is downloaded.

- **Queue** (`src-tauri/src/party/queue.rs`): Miu holds the one true queue. Guests may add (per-guest
  limit), vote once per entry; the DJ can require approval, remove anything and close the queue.
- **Links** (`links.rs`): YouTube → a video; Spotify and Apple Music → matched to a song in the DJ's
  library by title (they cannot be streamed).
- **On the Wi-Fi** (`web.rs`, `server.rs`): a small HTTP server on the PC at `http://<pc>:8765/p/<random
  token>/`; the guest page long-polls the queue.
- **Anywhere** (`relay.rs`, `relay/`): for guests not on the same Wi-Fi, Miu opens a room on
  `https://musicplayer.vietnamcos.com/party/?r=ROOM`. The site only passes messages: Miu pushes the
  queue when it changes and fetches guests' adds and votes every 2 s, handles them like the Wi-Fi page
  and sends back refusals. Guests poll every 3 s. Rooms run out 12 h after Miu last spoke. An admin page
  (`/party/admin.php`) turns party links off, caps parties and requests, shows the load and ends parties.
  See `relay/README.md` for the server side.
- **The DJ's screen** (`src/js/partyscreen.js`): Guests panel (both links, approval, limit, waiting
  and queued songs), the video now playing with Skip.
- **The party screen** (`src/party.html`, `src/js/partywindow.js`): its own window, so the DJ screen
  stays usable. Drag it to the TV, full screen with F11, a double-click or its button (the pointer
  hides when the mouse rests). It follows the DJ by itself: cover, song on air, progress, up next, the
  QR code (the relay's link when it works, else the Wi-Fi one) and the top of the queue. YouTube videos
  play there full screen with the QR code in a corner; it opens by itself when a guest's video comes up.
