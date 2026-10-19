import { resolveTheme } from './theme.js';

const media = window.matchMedia('(prefers-color-scheme: dark)');
const preference = 'system'; // becomes a setting in F5.5

function applyTheme() {
  document.documentElement.dataset.theme = resolveTheme(preference, media.matches);
}
applyTheme();
media.addEventListener('change', applyTheme);

const invoke = window.__TAURI__?.core?.invoke;
if (invoke) {
  invoke('app_info').then(info => {
    document.getElementById('app-version').textContent = `${info.name} ${info.version}`;
  });
}
