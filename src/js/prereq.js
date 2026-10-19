// User-facing message for the Apple driver check (F0.3). Strings move to src/i18n in F8.6.
export function prereqMessage(report) {
  if (!report || report.ready) return null;
  return {
    title: 'Apple drivers needed',
    body: 'Miu Music talks to your iPhone through Apple’s drivers. Install the free Apple Devices app (or iTunes) from the Microsoft Store, then restart Miu Music.',
    detail: `Missing: ${report.missing.join(', ')}`,
  };
}
