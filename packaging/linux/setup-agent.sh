#!/bin/sh
set -eu

test "$(id -u)" = 0 || { echo 'Run via sudo or pkexec.' >&2; exit 1; }

owner_uid=${PKEXEC_UID:-${SUDO_UID:-}}
case "$owner_uid" in
    ''|*[!0-9]*|0) echo 'Unable to identify the non-root desktop user.' >&2; exit 1;;
esac

payload_dir=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
agent="$payload_dir/vpn-agent"
installer="$payload_dir/install-agent.sh"
upgrader="$payload_dir/upgrade-agent.sh"
test -f "$installer" || { echo 'The Agent installer is missing from the package payload.' >&2; exit 1; }
test -f "$upgrader" || { echo 'The Agent upgrader is missing from the package payload.' >&2; exit 1; }
test -f "$agent" && test -x "$agent" || { echo 'The packaged Agent binary is missing or not executable.' >&2; exit 1; }

installed=0
for path in /usr/local/lib/tinc-client/vpn-agent /etc/tinc-client/settings.json /etc/systemd/system/tinc-client-agent.service; do
    if test -e "$path" || test -L "$path"; then
        installed=$((installed + 1))
    fi
done
if test "$installed" = 3; then
    exec /bin/sh "$upgrader" "$agent"
fi
test "$installed" = 0 || { echo 'The existing Agent installation is incomplete; refusing automatic repair.' >&2; exit 1; }
exec /bin/sh "$installer" "$agent" "$owner_uid" tinc
