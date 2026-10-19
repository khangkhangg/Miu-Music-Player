// Device card for the sidebar (F1.3, F1.4).
import { t } from './i18n.js';

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
    label: t('device.storage', { free: formatBytes(free), total: formatBytes(total) }),
  };
}

export function deviceCard(devices) {
  const list = devices ?? [];
  if (list.length === 0) {
    return { state: 'empty', title: t('device.none'), detail: t('device.plugIn'), storage: null };
  }
  const [first, ...rest] = list;
  if (!first.trusted) {
    return { state: 'untrusted', title: t('device.connected'), detail: t('device.trust'), storage: null };
  }
  const more = rest.length ? ` · ${t('device.more', { n: rest.length })}` : '';
  const info = first.info;
  return {
    state: 'ready',
    title: info?.name || t('device.connected'),
    detail: (info ? t('device.model', { model: info.model, version: info.iosVersion }) : t('device.usb')) + more,
    storage: storage(info),
  };
}
