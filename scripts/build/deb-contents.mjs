export const REQUIRED_LINUX_PAYLOAD_PATHS = Object.freeze([
  'usr/share/tinc-connect/payload/vpn-agent',
  'usr/share/tinc-connect/payload/setup-agent.sh',
  'usr/share/tinc-connect/payload/install-agent.sh',
  'usr/share/tinc-connect/payload/upgrade-agent.sh',
  'usr/share/tinc-connect/payload/uninstall-agent.sh',
  'usr/share/tinc-connect/payload/tinc-client-agent.service',
]);

export function normalizeDebArchivePath(path) {
  return path.startsWith('./') ? path.slice(2) : path;
}

export function debArchivePaths(contents) {
  return new Set(
    contents
      .split(/\r?\n/)
      .map((line) => line.trim().split(/\s+/).at(-1))
      .filter(Boolean)
      .map(normalizeDebArchivePath),
  );
}

export function missingDebArchivePaths(contents, requiredPaths) {
  const archived = debArchivePaths(contents);
  return requiredPaths.filter((path) => !archived.has(normalizeDebArchivePath(path)));
}
