<?php
// Miu Party relay: passes messages between a DJ's Miu and the guests' phones when they are not on
// the same Wi-Fi. Miu holds the one true queue; the site keeps a copy for guests to read, a list of
// what guests asked for (Miu fetches it and answers), and nothing else. Everything lives in Redis
// and runs out after 12 hours. The whole API is `handle()` so tests/test.php can call it directly.

declare(strict_types=1);

const ROOM_TTL = 43200;          // a party room lives 12 h after its last push
const MAX_ACTIONS = 500;         // waiting guest actions per room
const MAX_BODY = 4_000_000;      // a push carries the library
const SEARCH_MAX = 30;
const CODE_CHARS = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789';

function prefix(): string
{
    return getenv('MIUPARTY_PREFIX') ?: 'miuparty:';
}

function redis(): Redis
{
    static $r = null;
    if ($r === null) {
        $r = new Redis();
        $r->connect('127.0.0.1', 6379, 1.0);
    }
    return $r;
}

/** Admin settings: on/off, how many rooms at once, guest requests per IP per minute. */
function config(): array
{
    $c = redis()->hGetAll(prefix() . 'config') ?: [];
    return [
        'enabled' => ($c['enabled'] ?? '1') === '1',
        'max_rooms' => (int) ($c['max_rooms'] ?? 50),
        'rate' => (int) ($c['rate'] ?? 1500),
    ];
}

function set_config(array $c): void
{
    $out = [];
    if (array_key_exists('enabled', $c)) {
        $out['enabled'] = $c['enabled'] ? '1' : '0';
    }
    foreach (['max_rooms', 'rate'] as $k) {
        if (array_key_exists($k, $c)) {
            $out[$k] = (string) max(0, (int) $c[$k]);
        }
    }
    if ($out) {
        redis()->hMSet(prefix() . 'config', $out);
    }
}

/** Counts requests per minute (the admin page shows the last hour). */
function count_request(): void
{
    $k = prefix() . 'stats:' . intdiv(time(), 60);
    redis()->incr($k);
    redis()->expire($k, 3700);
}

function requests_per_minute(int $minutes = 10): array
{
    $now = intdiv(time(), 60);
    $out = [];
    for ($m = $now - $minutes + 1; $m <= $now; $m++) {
        $out[] = ['minute' => $m * 60, 'count' => (int) (redis()->get(prefix() . 'stats:' . $m) ?: 0)];
    }
    return $out;
}

/** True while `$key` (an IP and what it does) stays under `$limit` per `$window` seconds. */
function within(string $key, int $limit, int $window): bool
{
    $k = prefix() . 'rate:' . $key . ':' . intdiv(time(), $window);
    $n = redis()->incr($k);
    if ($n === 1) {
        redis()->expire($k, $window + 5);
    }
    return $n <= $limit;
}

/** Lower case, no accents (đ is d), words separated by single spaces: like Miu's own search. */
function fold(string $s): string
{
    $s = mb_strtolower($s, 'UTF-8');
    $s = strtr($s, ['đ' => 'd']);
    if (class_exists('Normalizer')) {
        $s = Normalizer::normalize($s, Normalizer::FORM_D) ?: $s;
        $s = preg_replace('/\p{Mn}+/u', '', $s) ?? $s;
    }
    return trim(preg_replace('/[^\p{L}\p{N}]+/u', ' ', $s) ?? '');
}

function room_key(string $room, string $what = ''): string
{
    return prefix() . 'room:' . $room . ($what === '' ? '' : ':' . $what);
}

function valid_room(string $room): bool
{
    return (bool) preg_match('/^[' . CODE_CHARS . ']{6}$/', $room);
}

function valid_guest(mixed $g): bool
{
    return is_string($g) && $g !== '' && strlen($g) <= 64 && $g !== 'host' && preg_match('/^[A-Za-z0-9_-]+$/', $g);
}

function new_code(): string
{
    $code = '';
    for ($i = 0; $i < 6; $i++) {
        $code .= CODE_CHARS[random_int(0, strlen(CODE_CHARS) - 1)];
    }
    return $code;
}

/** The rooms still alive (forgets those that ran out). */
function rooms(): array
{
    $r = redis();
    $alive = [];
    foreach ($r->sMembers(prefix() . 'rooms') ?: [] as $code) {
        if ($r->exists(room_key($code))) {
            $alive[] = $code;
        } else {
            $r->sRem(prefix() . 'rooms', $code);
        }
    }
    sort($alive);
    return $alive;
}

function room_info(string $code): array
{
    $h = redis()->hMGet(room_key($code), ['created', 'seen', 'state']);
    $state = json_decode((string) ($h['state'] ?? ''), true);
    return [
        'room' => $code,
        'created' => (int) ($h['created'] ?? 0),
        'seen' => (int) ($h['seen'] ?? 0),
        'queue' => is_array($state) ? count($state['queue'] ?? []) : 0,
        'actions' => (int) redis()->lLen(room_key($code, 'actions')),
    ];
}

function close_room(string $code): void
{
    $r = redis();
    $keys = [room_key($code), room_key($code, 'actions'), room_key($code, 'seq'), room_key($code, 'owners')];
    foreach ($r->keys(room_key($code, 'notices') . ':*') ?: [] as $k) {
        $keys[] = $k;
    }
    $r->del($keys);
    $r->sRem(prefix() . 'rooms', $code);
}

function touch_room(string $code): void
{
    foreach (['', 'actions', 'seq', 'owners'] as $what) {
        redis()->expire(room_key($code, $what), ROOM_TTL);
    }
}

/** A guest's view from the queue Miu pushed: the same shape as the Wi-Fi page's /api/state. */
function guest_view(array $s, string $guest): array
{
    $entry = function (array $x) use ($guest): array {
        $voters = $x['voters'] ?? [];
        $v = $x;
        unset($v['addedBy'], $v['voters']);
        $v['votes'] = count($voters);
        $v['voted'] = in_array($guest, $voters, true);
        $v['mine'] = ($x['addedBy'] ?? '') === $guest;
        return $v;
    };
    return [
        'version' => (int) ($s['version'] ?? 0),
        'now' => isset($s['now']) && is_array($s['now']) ? $entry($s['now']) : null,
        'queue' => array_map($entry, $s['queue'] ?? []),
        'waitingMine' => count(array_filter($s['waiting'] ?? [], fn ($g) => $g === $guest)),
        'approval' => (bool) ($s['approval'] ?? false),
        'open' => (bool) ($s['open'] ?? true),
    ];
}

/**
 * Answers one API call: [status, body]. `$secret` is the X-Party-Secret header (Miu's ops),
 * `$ip` the caller (for limits).
 */
function handle(string $method, string $op, string $room, array $query, ?array $body, string $secret, string $ip): array
{
    $r = redis();
    $cfg = config();
    count_request();
    if (!$cfg['enabled']) {
        return [503, ['error' => 'relayOff']];
    }

    if ($op === 'create') {
        if ($method !== 'POST') {
            return [405, ['error' => 'badRequest']];
        }
        if (count(rooms()) >= $cfg['max_rooms']) {
            return [503, ['error' => 'full']];
        }
        if (!within('create:' . $ip, 10, 3600)) {
            return [429, ['error' => 'slowDown']];
        }
        do {
            $code = new_code();
        } while ($r->exists(room_key($code)));
        $key = bin2hex(random_bytes(24));
        $r->hMSet(room_key($code), ['secret' => hash('sha256', $key), 'created' => (string) time(), 'seen' => (string) time()]);
        $r->sAdd(prefix() . 'rooms', $code);
        touch_room($code);
        return [200, ['room' => $code, 'secret' => $key]];
    }

    if (!valid_room($room) || !$r->exists(room_key($room))) {
        return [404, ['error' => 'roomGone']];
    }

    // ---- Miu (the host) ----
    if (in_array($op, ['push', 'actions', 'results', 'close'], true)) {
        $stored = (string) $r->hGet(room_key($room), 'secret');
        if ($secret === '' || !hash_equals($stored, hash('sha256', $secret))) {
            return [403, ['error' => 'notHost']];
        }
        $r->hSet(room_key($room), 'seen', (string) time());
        touch_room($room);
        switch ($op) {
            case 'push':
                if (!is_array($body['state'] ?? null)) {
                    return [400, ['error' => 'badRequest']];
                }
                $set = ['state' => json_encode($body['state'], JSON_UNESCAPED_UNICODE)];
                if (is_array($body['library'] ?? null)) {
                    $lib = [];
                    foreach ($body['library'] as $s) {
                        if (!is_array($s) || !isset($s['songId'])) {
                            continue;
                        }
                        $title = (string) ($s['title'] ?? '');
                        $artist = (string) ($s['artist'] ?? '');
                        $lib[] = ['songId' => (int) $s['songId'], 'title' => $title, 'artist' => $artist, 'f' => fold("$title $artist")];
                    }
                    $set['library'] = json_encode($lib, JSON_UNESCAPED_UNICODE);
                }
                $r->hMSet(room_key($room), $set);
                return [200, ['ok' => true]];
            case 'actions':
                // Everything up to `after` has been handled: drop it, send the rest.
                $after = (int) ($query['after'] ?? 0);
                $list = $r->lRange(room_key($room, 'actions'), 0, -1) ?: [];
                $out = [];
                $done = 0;
                foreach ($list as $raw) {
                    $a = json_decode($raw, true);
                    if (!is_array($a) || ($a['id'] ?? 0) <= $after) {
                        $done++;
                        continue;
                    }
                    $out[] = $a;
                }
                if ($done > 0) {
                    $r->lTrim(room_key($room, 'actions'), $done, -1);
                }
                return [200, ['actions' => $out]];
            case 'results':
                // Refusals reach the guest who asked on their next look at the queue.
                foreach ((array) ($body['results'] ?? []) as $id => $res) {
                    $guest = $r->hGet(room_key($room, 'owners'), (string) $id);
                    $r->hDel(room_key($room, 'owners'), (string) $id);
                    if (!is_string($guest) || !is_array($res) || empty($res['error'])) {
                        continue;
                    }
                    $k = room_key($room, 'notices') . ':' . $guest;
                    $r->rPush($k, json_encode(['id' => (int) $id, 'error' => (string) $res['error']]));
                    $r->lTrim($k, -10, -1);
                    $r->expire($k, 3600);
                }
                return [200, ['ok' => true]];
            case 'close':
                close_room($room);
                return [200, ['ok' => true]];
        }
    }

    // ---- Guests ----
    if (!within('guest:' . $ip, $cfg['rate'], 60)) {
        return [429, ['error' => 'slowDown']];
    }
    $state = json_decode((string) $r->hGet(room_key($room), 'state'), true);
    if (!is_array($state)) {
        $state = ['version' => 0, 'now' => null, 'queue' => [], 'waiting' => [], 'approval' => false, 'open' => true];
    }
    switch ($op) {
        case 'state':
            $guest = (string) ($query['guest'] ?? '');
            $view = guest_view($state, $guest);
            $view['notices'] = [];
            if (valid_guest($guest)) {
                $k = room_key($room, 'notices') . ':' . $guest;
                foreach ($r->lRange($k, 0, -1) ?: [] as $raw) {
                    $view['notices'][] = json_decode($raw, true);
                }
                $r->del($k);
            }
            return [200, $view];
        case 'search':
            $words = array_values(array_filter(explode(' ', fold((string) ($query['q'] ?? '')))));
            if (!$words) {
                return [200, []];
            }
            $lib = json_decode((string) $r->hGet(room_key($room), 'library'), true) ?: [];
            $out = [];
            foreach ($lib as $s) {
                $hay = $s['f'] ?? '';
                foreach ($words as $w) {
                    if (!str_contains($hay, $w)) {
                        continue 2;
                    }
                }
                $out[] = ['songId' => $s['songId'], 'title' => $s['title'], 'artist' => $s['artist']];
                if (count($out) >= SEARCH_MAX) {
                    break;
                }
            }
            return [200, $out];
        case 'add':
        case 'vote':
            if ($method !== 'POST' || !is_array($body) || !valid_guest($body['guest'] ?? null)) {
                return [400, ['error' => 'badRequest']];
            }
            if ($op === 'add' && !($state['open'] ?? true)) {
                return [409, ['error' => 'closed']];
            }
            if ($r->lLen(room_key($room, 'actions')) >= MAX_ACTIONS) {
                return [503, ['error' => 'busy']];
            }
            $guest = $body['guest'];
            if ($op === 'add') {
                $clean = ['guest' => $guest, 'name' => mb_substr(trim((string) ($body['name'] ?? '')), 0, 40)];
                if (isset($body['songId']) && is_numeric($body['songId'])) {
                    $clean['songId'] = (int) $body['songId'];
                } elseif (isset($body['url']) && is_string($body['url']) && strlen($body['url']) <= 500) {
                    $clean['url'] = $body['url'];
                } else {
                    return [400, ['error' => 'badRequest']];
                }
            } else {
                if (!isset($body['id']) || !is_numeric($body['id'])) {
                    return [400, ['error' => 'badRequest']];
                }
                $clean = ['guest' => $guest, 'id' => (int) $body['id'], 'on' => (bool) ($body['on'] ?? true)];
            }
            $id = (int) $r->incr(room_key($room, 'seq'));
            $r->rPush(room_key($room, 'actions'), json_encode(['id' => $id, 'op' => $op, 'body' => $clean], JSON_UNESCAPED_UNICODE));
            $r->hSet(room_key($room, 'owners'), (string) $id, $guest);
            touch_room($room);
            return [200, ['pending' => $id]];
    }
    return [404, ['error' => 'notFound']];
}
