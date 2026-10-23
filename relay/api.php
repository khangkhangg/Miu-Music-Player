<?php
// The relay's API (see lib.php): /party/api.php?op=...&r=ROOM
declare(strict_types=1);

require __DIR__ . '/lib.php';

header('Content-Type: application/json; charset=utf-8');
header('Cache-Control: no-store');
header('X-Content-Type-Options: nosniff');

$raw = file_get_contents('php://input', false, null, 0, MAX_BODY + 1);
if ($raw !== false && strlen($raw) > MAX_BODY) {
    http_response_code(413);
    echo json_encode(['error' => 'tooBig']);
    exit;
}
$body = $raw ? json_decode($raw, true) : null;

try {
    [$status, $out] = handle(
        $_SERVER['REQUEST_METHOD'] ?? 'GET',
        (string) ($_GET['op'] ?? ''),
        (string) ($_GET['r'] ?? ''),
        $_GET,
        is_array($body) ? $body : null,
        (string) ($_SERVER['HTTP_X_PARTY_SECRET'] ?? ''),
        (string) ($_SERVER['HTTP_CF_CONNECTING_IP'] ?? $_SERVER['REMOTE_ADDR'] ?? ''),
    );
} catch (RedisException $e) {
    [$status, $out] = [503, ['error' => 'relayDown']];
}
http_response_code($status);
echo json_encode($out, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
