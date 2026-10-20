// First-run wizard (F6.2): Apple drivers → iPhone → music folders. Each step is done, skipped,
// current or still to do; the wizard closes when nothing is left or the user dismisses it.

/** `s`: { driversReady, phone: 'none' | 'untrusted' | 'ready', folders, skippedPhone, dismissed }. */
export function wizard(s) {
  const done = {
    drivers: Boolean(s.driversReady),
    phone: s.phone === 'ready',
    folders: s.folders > 0,
  };
  const skipped = { phone: Boolean(s.skippedPhone) && !done.phone };
  const order = ['drivers', 'phone', 'folders'];
  const current = order.find((id) => !done[id] && !skipped[id]) ?? null;
  const steps = order.map((id) => ({
    id,
    state: done[id] ? 'done' : skipped[id] ? 'skipped' : id === current ? 'current' : 'todo',
  }));
  return {
    steps,
    current,
    hint: current === 'phone' && s.phone === 'untrusted' ? 'trust' : null,
    show: !s.dismissed && current !== null,
  };
}
