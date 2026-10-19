import { resolveTheme } from './theme.js';
import { prereqMessage } from './prereq.js';

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
  invoke('check_prereqs').then(showPrereqNotice);
}

function showPrereqNotice(report) {
  const message = prereqMessage(report);
  const notice = document.getElementById('prereq-notice');
  notice.hidden = !message;
  if (!message) return;
  notice.querySelector('.notice-title').textContent = message.title;
  notice.querySelector('.notice-body').textContent = message.body;
  notice.querySelector('.notice-detail').textContent = message.detail;
}
