import { spawnSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..', '..');
if (process.platform !== 'linux') {
  console.error('The Linux package build must run on Linux.');
  process.exit(1);
}

const cargo = spawnSync(
  'cargo',
  ['build', '--locked', '--release', '-p', 'vpn-agent', '--message-format=json-render-diagnostics'],
  { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
);
if (cargo.stderr) process.stderr.write(cargo.stderr);
if (cargo.error || cargo.status !== 0) {
  console.error(cargo.error ? `Unable to start Cargo: ${cargo.error.message}` : 'Linux Agent release build failed.');
  process.exit(cargo.status ?? 1);
}

const agent = cargo.stdout
  .split(/\r?\n/)
  .filter(Boolean)
  .map((line) => {
    try {
      return JSON.parse(line);
    } catch {
      return null;
    }
  })
  .filter((entry) => entry?.reason === 'compiler-artifact' && entry.target?.name === 'vpn-agent' && entry.executable)
  .at(-1);
if (!agent) {
  console.error('Cargo did not report the Linux Agent executable.');
  process.exit(1);
}

const payloadRoot = resolve(root, '.runtime', 'build', 'linux-payload');
const stagedAgent = resolve(payloadRoot, 'vpn-agent');
mkdirSync(payloadRoot, { recursive: true, mode: 0o755 });
copyFileSync(agent.executable, stagedAgent);
chmodSync(stagedAgent, 0o755);

const config = resolve(root, 'apps', 'desktop', 'src-tauri', 'tauri.linux.conf.json');
const bundle = spawnSync(
  'npm',
  ['run', 'tauri', '--workspace', 'apps/desktop', '--', 'build', '--bundles', 'deb', '--config', config],
  { cwd: root, stdio: 'inherit' },
);
if (bundle.error) console.error(`Unable to start the Linux desktop build: ${bundle.error.message}`);
if (bundle.error || bundle.status !== 0) process.exit(bundle.status ?? 1);

const configuredTarget = process.env.CARGO_TARGET_DIR;
const targetRoot = configuredTarget
  ? resolve(root, configuredTarget)
  : resolve(root, 'target');
const debRoot = resolve(targetRoot, 'release', 'bundle', 'deb');
const packages = readdirSync(debRoot)
  .filter((name) => name.endsWith('.deb'))
  .map((name) => resolve(debRoot, name))
  .sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs);
if (!packages.length) {
  console.error(`No Debian package was created in ${debRoot}.`);
  process.exit(1);
}

const contents = spawnSync('dpkg-deb', ['--contents', packages[0]], { encoding: 'utf8' });
if (contents.error || contents.status !== 0) {
  if (contents.stderr) process.stderr.write(contents.stderr);
  console.error(contents.error ? `Unable to inspect Debian package: ${contents.error.message}` : 'Unable to inspect Debian package contents.');
  process.exit(contents.status ?? 1);
}
for (const expected of [
  './usr/share/tinc-connect/payload/vpn-agent',
  './usr/share/tinc-connect/payload/setup-agent.sh',
  './usr/share/tinc-connect/payload/install-agent.sh',
  './usr/share/tinc-connect/payload/uninstall-agent.sh',
  './usr/share/tinc-connect/payload/tinc-client-agent.service',
]) {
  if (!contents.stdout.includes(expected)) {
    console.error(`Debian package is missing required payload: ${expected}`);
    process.exit(1);
  }
}
console.log(`Verified Linux desktop and Agent payload: ${packages[0]}`);
