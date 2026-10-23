<?php
// Tests for the relay (lib.php), run on a machine with PHP and Redis:
//   php relay/tests/test.php
// Uses its own Redis prefix and removes everything it made.
declare(strict_types=1);

putenv('MIUPARTY_PREFIX=miuparty-test-' . bin2hex(random_bytes(4)) . ':');
require __DIR__ . '/../lib.php';

$failed = 0;
function check(string $what, bool $ok, mixed $got = null): void
{
    global $failed;
    echo ($ok ? 'ok   ' : 'FAIL ') . $what . ($ok ? '' : '  got: ' . json_encode($got)) . "\n";
    if (!$ok) {
        $failed++;
    }
}

function host(string $op, string $room, string $secret, ?array $body = null, array $query = []): array
{
    return handle('POST', $op, $room, $query, $body, $secret, '10.0.0.1');
}

function guest(string $method, string $op, string $room, array $query = [], ?array $body = null): array
{
    return handle($method, $op, $room, $query, $body, '', '10.0.0.2');
}

try {
    // Miu opens a room.
    [$s, $made] = host('create', '', '');
    check('a room opens', $s === 200 && valid_room($made['room'] ?? '') && strlen($made['secret'] ?? '') === 48, $made);
    $room = $made['room'];
    $key = $made['secret'];

    // Before the first push guests see an empty queue.
    [$s, $v] = guest('GET', 'state', $room, ['guest' => 'g1']);
    check('empty queue before the first push', $s === 200 && $v['queue'] === [] && $v['open'] === true, $v);

    // Only Miu may push.
    $state = [
        'version' => 3,
        'now' => null,
        'queue' => [
            ['id' => 1, 'kind' => 'song', 'songId' => 2, 'title' => 'Đi Về Nhà', 'artist' => 'Đen', 'addedBy' => 'g1', 'addedName' => 'Ann', 'voters' => ['g1', 'g2']],
            ['id' => 2, 'kind' => 'video', 'videoId' => 'dQw4w9WgXcQ', 'title' => 'Karaoke', 'addedBy' => 'host', 'addedName' => 'DJ', 'voters' => ['host']],
        ],
        'waiting' => ['g2'],
        'approval' => true,
        'open' => true,
    ];
    $library = [['songId' => 1, 'title' => 'Love Yourself', 'artist' => 'Justin Bieber'], ['songId' => 2, 'title' => 'Đi Về Nhà', 'artist' => 'Đen, JustaTee']];
    [$s] = host('push', $room, 'wrong', ['state' => $state]);
    check('a wrong secret cannot push', $s === 403, $s);
    [$s] = host('push', $room, $key, ['state' => $state, 'library' => $library]);
    check('Miu pushes the queue and library', $s === 200, $s);

    // Each guest sees their own view.
    [$s, $v] = guest('GET', 'state', $room, ['guest' => 'g2']);
    $e = $v['queue'][0] ?? [];
    check('the queue in order', count($v['queue']) === 2 && $e['title'] === 'Đi Về Nhà' && $v['version'] === 3, $v);
    check('votes counted, own vote shown', $e['votes'] === 2 && $e['voted'] === true && $e['mine'] === false, $e);
    check('voters are not shown to guests', !isset($e['voters']) && !isset($e['addedBy']), $e);
    check('their song waiting for the DJ', $v['waitingMine'] === 1 && $v['approval'] === true, $v);
    [, $v] = guest('GET', 'state', $room, ['guest' => 'g1']);
    check('a guest\'s own song is marked', $v['queue'][0]['mine'] === true && $v['waitingMine'] === 0, $v);

    // Search, accents or not.
    [, $found] = guest('GET', 'search', $room, ['q' => 'di ve']);
    check('search without accents', count($found) === 1 && $found[0]['songId'] === 2 && !isset($found[0]['f']), $found);
    [, $found] = guest('GET', 'search', $room, ['q' => 'justin']);
    check('search by artist', count($found) === 1 && $found[0]['songId'] === 1, $found);

    // Guests' adds and votes wait for Miu.
    [$s, $a] = guest('POST', 'add', $room, [], ['guest' => 'g3', 'name' => ' Cat ', 'songId' => 1]);
    check('an add waits for the DJ', $s === 200 && isset($a['pending']), $a);
    [$s, $b] = guest('POST', 'vote', $room, [], ['guest' => 'g3', 'id' => 1, 'on' => true]);
    check('a vote waits for the DJ', $s === 200 && $b['pending'] === $a['pending'] + 1, $b);
    [$s] = guest('POST', 'add', $room, [], ['guest' => 'host', 'songId' => 1]);
    check('guests cannot pretend to be the DJ', $s === 400, $s);
    [$s] = guest('POST', 'add', $room, [], ['guest' => 'g3']);
    check('an add needs a song or a link', $s === 400, $s);

    [$s, $got] = host('actions', $room, $key, null, ['after' => 0]);
    check('Miu fetches the actions', $s === 200 && count($got['actions']) === 2 && $got['actions'][0]['body'] === ['guest' => 'g3', 'name' => 'Cat', 'songId' => 1], $got);
    [, $got] = host('actions', $room, $key, null, ['after' => $a['pending']]);
    check('handled actions are dropped', count($got['actions']) === 1 && $got['actions'][0]['op'] === 'vote', $got);

    // A refusal reaches the guest once.
    host('results', $room, $key, ['results' => [(string) $a['pending'] => ['status' => 409, 'error' => 'limit'], (string) $b['pending'] => ['status' => 200]]]);
    [, $v] = guest('GET', 'state', $room, ['guest' => 'g3']);
    check('the guest hears why', $v['notices'] === [['id' => $a['pending'], 'error' => 'limit']], $v['notices']);
    [, $v] = guest('GET', 'state', $room, ['guest' => 'g3']);
    check('only once', $v['notices'] === [], $v['notices']);

    // The DJ closed the queue: adds are refused at once.
    $state['open'] = false;
    host('push', $room, $key, ['state' => $state]);
    [$s, $err] = guest('POST', 'add', $room, [], ['guest' => 'g3', 'songId' => 1]);
    check('closed queue', $s === 409 && $err['error'] === 'closed', $err);

    // The admin turns party links off.
    set_config(['enabled' => false]);
    [$s, $err] = guest('GET', 'state', $room, ['guest' => 'g1']);
    check('off means off', $s === 503 && $err['error'] === 'relayOff', $err);
    set_config(['enabled' => true, 'max_rooms' => 1]);
    [$s, $err] = host('create', '', '');
    check('no more rooms than allowed', $s === 503 && $err['error'] === 'full', $err);
    set_config(['max_rooms' => 50]);

    // Miu ends the party.
    [$s] = host('close', $room, $key);
    [$s2, $err] = guest('GET', 'state', $room, ['guest' => 'g1']);
    check('a closed room is gone', $s === 200 && $s2 === 404 && $err['error'] === 'roomGone', $err);
    check('unknown rooms are gone too', guest('GET', 'state', 'ZZZZZZ')[0] === 404);
    check('the counter counts', array_sum(array_column(requests_per_minute(2), 'count')) > 10);
} finally {
    $keys = redis()->keys(prefix() . '*') ?: [];
    if ($keys) {
        redis()->del($keys);
    }
}

echo $failed ? "\n$failed failed\n" : "\nall passed\n";
exit($failed ? 1 : 0);
