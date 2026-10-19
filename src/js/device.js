// Device card text for the sidebar (F1.3). Device name and storage arrive with F1.4.
// Strings move to src/i18n in F8.6.
export function deviceCard(devices) {
  const list = devices ?? [];
  if (list.length === 0) {
    return { state: 'empty', title: 'No device connected', detail: 'Plug in your iPhone with a USB cable' };
  }
  const [first, ...rest] = list;
  if (!first.trusted) {
    return { state: 'untrusted', title: 'iPhone connected', detail: 'Unlock your iPhone and tap Trust' };
  }
  const more = rest.length ? ` · ${rest.length} more device${rest.length > 1 ? 's' : ''}` : '';
  return { state: 'ready', title: 'iPhone connected', detail: `Connected over USB${more}` };
}
