// Device card for the sidebar (F1.3, F1.4). Strings move to src/i18n in F8.6.

export function formatBytes(bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return unit === 0 ? `${value} B` : `${value.toFixed(1)} ${units[unit]}`;
}

function storage(info) {
  const { capacityBytes: total, freeBytes: free } = info ?? {};
  if (!total || free == null) return null;
  return {
    usedPercent: Math.round(((total - free) / total) * 100),
    label: `${formatBytes(free)} free of ${formatBytes(total)}`,
  };
}

export function deviceCard(devices) {
  const list = devices ?? [];
  if (list.length === 0) {
    return { state: 'empty', title: 'No device connected', detail: 'Plug in your iPhone with a USB cable', storage: null };
  }
  const [first, ...rest] = list;
  if (!first.trusted) {
    return { state: 'untrusted', title: 'iPhone connected', detail: 'Unlock your iPhone and tap Trust', storage: null };
  }
  const more = rest.length ? ` · ${rest.length} more device${rest.length > 1 ? 's' : ''}` : '';
  const info = first.info;
  return {
    state: 'ready',
    title: info?.name || 'iPhone connected',
    detail: (info ? `${info.model} · iOS ${info.iosVersion}` : 'Connected over USB') + more,
    storage: storage(info),
  };
}
