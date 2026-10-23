<?php
// The relay's admin page: turn party links on or off, cap the rooms, see the load, end parties.
// The password hash lives outside the web folder in /etc/miuparty/config.php (see README.md).
declare(strict_types=1);

require __DIR__ . '/lib.php';

const ADMIN_CONFIG = '/etc/miuparty/config.php';

header('Cache-Control: no-store');
header('X-Content-Type-Options: nosniff');
header('X-Frame-Options: DENY');
header("Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'");

session_name('miuparty_admin');
session_set_cookie_params(['lifetime' => 0, 'path' => '/party/', 'secure' => true, 'httponly' => true, 'samesite' => 'Strict']);
session_start();

$ip = (string) ($_SERVER['HTTP_CF_CONNECTING_IP'] ?? $_SERVER['REMOTE_ADDR'] ?? '');
$h = fn ($s) => htmlspecialchars((string) $s, ENT_QUOTES, 'UTF-8');
$adminConfig = is_readable(ADMIN_CONFIG) ? (require ADMIN_CONFIG) : [];
$hash = (string) ($adminConfig['password_hash'] ?? '');
$error = '';

if (($_POST['do'] ?? '') === 'login') {
    if (!within('login:' . $ip, 10, 600)) {
        $error = 'Too many tries. Wait ten minutes.';
    } elseif ($hash !== '' && password_verify((string) ($_POST['password'] ?? ''), $hash)) {
        session_regenerate_id(true);
        $_SESSION['admin'] = true;
        $_SESSION['csrf'] = bin2hex(random_bytes(16));
        header('Location: admin.php', true, 303);
        exit;
    } else {
        $error = 'Wrong password.';
    }
}

$signedIn = !empty($_SESSION['admin']);

if ($signedIn && $_SERVER['REQUEST_METHOD'] === 'POST') {
    if (!hash_equals((string) ($_SESSION['csrf'] ?? ''), (string) ($_POST['csrf'] ?? ''))) {
        http_response_code(400);
        exit('Bad request.');
    }
    switch ($_POST['do'] ?? '') {
        case 'enable':
            set_config(['enabled' => true]);
            break;
        case 'disable':
            set_config(['enabled' => false]);
            break;
        case 'limits':
            set_config(['max_rooms' => $_POST['max_rooms'] ?? 50, 'rate' => $_POST['rate'] ?? 1500]);
            break;
        case 'end':
            if (valid_room((string) ($_POST['room'] ?? ''))) {
                close_room((string) $_POST['room']);
            }
            break;
        case 'end_all':
            foreach (rooms() as $code) {
                close_room($code);
            }
            break;
        case 'logout':
            $_SESSION = [];
            session_destroy();
            break;
    }
    header('Location: admin.php', true, 303);
    exit;
}

$ago = function (int $t): string {
    if ($t === 0) {
        return '-';
    }
    $s = time() - $t;
    return $s < 60 ? "{$s} s ago" : ($s < 3600 ? intdiv($s, 60) . ' min ago' : intdiv($s, 3600) . ' h ago');
};
?>
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>Miu Party relay</title>
<style>
  :root { --bg: #f6f6f8; --card: #fff; --text: #1c1c1e; --muted: #6e6e73; --line: #d9d9de; --accent: #e5397f; --ok: #1f8f4e; --bad: #c4302b; color-scheme: light dark; }
  @media (prefers-color-scheme: dark) { :root { --bg: #111113; --card: #1c1c1f; --text: #f2f2f4; --muted: #9a9aa1; --line: #333338; } }
  body { margin: 0; background: var(--bg); color: var(--text); font: 15px/1.45 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif; }
  main { max-width: 760px; margin: 0 auto; padding: 24px 16px 48px; }
  h1 { font-size: 22px; margin: 0 0 16px; }
  h2 { font-size: 16px; margin: 0 0 12px; }
  section { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 16px; margin-bottom: 16px; }
  .row { display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
  .state { font-weight: 600; }
  .on { color: var(--ok); } .off { color: var(--bad); }
  button { font: inherit; padding: 7px 14px; border-radius: 8px; border: 1px solid var(--line); background: var(--card); color: var(--text); cursor: pointer; }
  button.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
  button.danger { color: var(--bad); }
  input { font: inherit; padding: 6px 8px; border-radius: 8px; border: 1px solid var(--line); background: var(--bg); color: var(--text); width: 90px; }
  input[type=password] { width: 220px; }
  table { width: 100%; border-collapse: collapse; }
  th, td { text-align: left; padding: 6px 4px; border-bottom: 1px solid var(--line); font-variant-numeric: tabular-nums; }
  th { color: var(--muted); font-weight: 500; font-size: 13px; }
  .muted { color: var(--muted); }
  .bars { display: flex; gap: 4px; align-items: flex-end; height: 80px; }
  .bars div { flex: 1; background: var(--accent); border-radius: 3px 3px 0 0; min-height: 2px; }
  .error { color: var(--bad); }
  form { margin: 0; }
</style>
</head>
<body>
<main>
<h1>Miu Party relay</h1>
<?php if (!$signedIn): ?>
  <section>
    <h2>Sign in</h2>
    <?php if ($hash === ''): ?>
      <p class="error">No admin password is set. See relay/README.md.</p>
    <?php endif; ?>
    <form method="post" class="row">
      <input type="hidden" name="do" value="login">
      <input type="password" name="password" autocomplete="current-password" aria-label="Password" required>
      <button class="primary" type="submit">Sign in</button>
    </form>
    <?php if ($error): ?><p class="error"><?= $h($error) ?></p><?php endif; ?>
  </section>
<?php else:
    $cfg = config();
    $rooms = array_map('room_info', rooms());
    $stats = requests_per_minute(10);
    $peak = max(1, ...array_column($stats, 'count'));
    $csrf = $h($_SESSION['csrf']);
    ?>
  <section>
    <h2>Party links</h2>
    <div class="row">
      <span class="state <?= $cfg['enabled'] ? 'on' : 'off' ?>"><?= $cfg['enabled'] ? 'On' : 'Off' ?></span>
      <span class="muted"><?= $cfg['enabled'] ? 'Guests can use party links.' : 'Party links answer that they are off. Wi-Fi parties still work.' ?></span>
      <form method="post">
        <input type="hidden" name="csrf" value="<?= $csrf ?>">
        <input type="hidden" name="do" value="<?= $cfg['enabled'] ? 'disable' : 'enable' ?>">
        <button class="<?= $cfg['enabled'] ? 'danger' : 'primary' ?>" type="submit"><?= $cfg['enabled'] ? 'Turn off' : 'Turn on' ?></button>
      </form>
    </div>
  </section>

  <section>
    <h2>Load</h2>
    <p class="muted">Requests per minute, last 10 minutes (now: <?= $h(end($stats)['count']) ?>).</p>
    <div class="bars" role="img" aria-label="Requests per minute">
      <?php foreach ($stats as $s): ?>
        <div style="height: <?= (int) round(100 * $s['count'] / $peak) ?>%" title="<?= $h(date('H:i', $s['minute'])) ?>: <?= $h($s['count']) ?>"></div>
      <?php endforeach; ?>
    </div>
    <p class="muted">Server load: <?= $h(implode(' / ', array_map(fn ($l) => number_format($l, 2), sys_getloadavg() ?: []))) ?></p>
  </section>

  <section>
    <h2>Limits</h2>
    <form method="post" class="row">
      <input type="hidden" name="csrf" value="<?= $csrf ?>">
      <input type="hidden" name="do" value="limits">
      <label>Parties at once <input type="number" name="max_rooms" min="0" value="<?= $h($cfg['max_rooms']) ?>"></label>
      <label>Guest requests per IP per minute <input type="number" name="rate" min="0" value="<?= $h($cfg['rate']) ?>"></label>
      <button type="submit">Save</button>
    </form>
  </section>

  <section>
    <div class="row" style="justify-content: space-between">
      <h2>Parties (<?= count($rooms) ?>)</h2>
      <?php if ($rooms): ?>
      <form method="post">
        <input type="hidden" name="csrf" value="<?= $csrf ?>">
        <input type="hidden" name="do" value="end_all">
        <button class="danger" type="submit">End all</button>
      </form>
      <?php endif; ?>
    </div>
    <?php if (!$rooms): ?>
      <p class="muted">No parties right now.</p>
    <?php else: ?>
    <table>
      <thead><tr><th>Room</th><th>Started</th><th>DJ last seen</th><th>Queue</th><th>Waiting actions</th><th></th></tr></thead>
      <tbody>
      <?php foreach ($rooms as $r): ?>
        <tr>
          <td><?= $h($r['room']) ?></td>
          <td><?= $h($ago($r['created'])) ?></td>
          <td><?= $h($ago($r['seen'])) ?></td>
          <td><?= $h($r['queue']) ?></td>
          <td><?= $h($r['actions']) ?></td>
          <td>
            <form method="post">
              <input type="hidden" name="csrf" value="<?= $csrf ?>">
              <input type="hidden" name="do" value="end">
              <input type="hidden" name="room" value="<?= $h($r['room']) ?>">
              <button class="danger" type="submit">End</button>
            </form>
          </td>
        </tr>
      <?php endforeach; ?>
      </tbody>
    </table>
    <?php endif; ?>
  </section>

  <form method="post">
    <input type="hidden" name="csrf" value="<?= $csrf ?>">
    <input type="hidden" name="do" value="logout">
    <button type="submit">Sign out</button>
  </form>
<?php endif; ?>
</main>
</body>
</html>
