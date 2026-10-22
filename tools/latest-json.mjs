// Write latest.json for the in-app updater (tauri-plugin-updater) from a signed build.
// Usage: node tools/latest-json.mjs <version> <path to setup.exe.sig> <notes file> <out file>
// The app reads it from https://github.com/khangkhangg/Miu-Music-Player/releases/latest/download/latest.json,
// so upload it to every release next to the installer (renamed to assetName(version)).
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const REPO = 'khangkhangg/Miu-Music-Player';

/** Name of the installer on the GitHub release. */
export function assetName(version) {
  return `MiuMusic_${version}_x64-setup.exe`;
}

/** The manifest the updater checks: version, notes, date and the signed Windows installer. */
export function buildManifest({ version, notes, pubDate, signature }) {
  const v = String(version).replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+$/.test(v)) throw new Error(`version must be x.y.z, got ${version}`);
  const sig = String(signature ?? '').trim();
  if (!sig) throw new Error('the signature is empty: build with TAURI_SIGNING_PRIVATE_KEY set');
  return {
    version: v,
    notes,
    pub_date: pubDate.toISOString(),
    platforms: {
      'windows-x86_64': { signature: sig, url: `https://github.com/${REPO}/releases/download/v${v}/${assetName(v)}` },
    },
  };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [version, sigPath, notesPath, out] = process.argv.slice(2);
  if (!out) {
    console.error('usage: node tools/latest-json.mjs <version> <setup.exe.sig> <notes file> <out file>');
    process.exit(2);
  }
  const manifest = buildManifest({
    version,
    notes: fs.readFileSync(notesPath, 'utf8').trim(),
    pubDate: new Date(),
    signature: fs.readFileSync(sigPath, 'utf8'),
  });
  fs.writeFileSync(out, JSON.stringify(manifest, null, 2) + '\n');
  console.log(`wrote ${out}`);
}
