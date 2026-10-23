# Miu Party relay (musicplayer.vietnamcos.com/party/)

Lets party guests who are not on the DJ's Wi-Fi use the queue page. The site only passes messages;
the DJ's Miu holds the queue (see `src-tauri/src/party/relay.rs` and `lib.php`).

| File | Where it goes on spo2x |
|---|---|
| `lib.php`, `api.php`, `admin.php` | `/var/www/musicplayer-party/` (outside the web folder) |
| `src-tauri/src/party/page/guest.html` | `/var/www/musicplayer/party/index.html` |
| `src-tauri/src/party/page/guest.js`, `guest.css` | `/var/www/musicplayer/party/` |
| `nginx-party.conf` | inside the `musicplayer.vietnamcos.com` server blocks |

Needs PHP 8.1+ with the `redis` and `intl` extensions, and Redis on 127.0.0.1:6379. Keys start with
`miuparty:` and run out on their own (rooms 12 h after the DJ's Miu last spoke).

## Admin page

`https://musicplayer.vietnamcos.com/party/admin.php`: turn party links off (guests see "the party
link is off", Wi-Fi parties carry on), cap parties at once and guest requests per IP per minute, see
requests per minute and the server load, end one party or all.

The password hash lives in `/etc/miuparty/config.php` (root-owned, readable by the PHP-FPM user):

```php
<?php return ['password_hash' => '$2y$...'];
```

Make one with `php -r 'echo password_hash(trim(fgets(STDIN)), PASSWORD_DEFAULT), "\n";'`.

## Tests

On the server (uses its own Redis prefix and cleans up): `php /var/www/musicplayer-party/tests/test.php`
