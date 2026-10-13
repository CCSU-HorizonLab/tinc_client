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
} else {
  const result = spawnSync('npm', ['run', 'tauri', '--workspace', 'apps/desktop', '--', 'build'], {
    stdio: 'inherit',
    cwd: root,
  });
  if (result.error) console.error(`Unable to start the desktop build: ${result.error.message}`);
  process.exit(result.status ?? 1);
}
