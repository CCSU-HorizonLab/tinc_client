import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..', '..');
if (process.platform === 'win32') {
  const result = spawnSync('powershell.exe', ['-NoProfile', '-File', resolve(root, 'scripts/build/windows.ps1')], {
    stdio: 'inherit',
    cwd: root,
  });
  if (result.error) console.error(`Unable to start the Windows build: ${result.error.message}`);
  process.exit(result.status ?? 1);
} else if (process.platform === 'linux') {
  const result = spawnSync(process.execPath, [resolve(root, 'scripts/build/linux.mjs')], {
    stdio: 'inherit',
    cwd: root,
  });
  if (result.error) console.error(`Unable to start the Linux build: ${result.error.message}`);
  process.exit(result.status ?? 1);
}

console.error(`Desktop packaging is not configured for ${process.platform}.`);
process.exit(1);
