#!/bin/sh
set -eu
# sudo ./install-agent.sh /absolute/path/vpn-agent INTERACTIVE_UID INTERFACE
test "$(id -u)" = 0 || { echo 'Run via sudo.' >&2; exit 1; }
test "$#" = 3 || { echo 'Usage: install-agent.sh AGENT_BINARY OWNER_UID INTERFACE' >&2; exit 1; }
case "$1" in /*) ;; *) echo 'AGENT_BINARY must be an absolute path' >&2; exit 1;; esac
case "$2" in ''|*[!0-9]*) echo 'Invalid UID' >&2; exit 1;; esac
case "$3" in ''|*[!a-zA-Z0-9_-]*) echo 'Invalid interface name' >&2; exit 1;; esac
test "$2" -ne 0 || { echo 'OWNER_UID must identify a non-root desktop user.' >&2; exit 1; }
command -v getent >/dev/null 2>&1 || { echo 'getent is required to validate OWNER_UID.' >&2; exit 1; }
getent passwd "$2" >/dev/null 2>&1 || { echo 'OWNER_UID does not identify a local user.' >&2; exit 1; }
test "${#3}" -le 15 || { echo 'Linux interface names must not exceed 15 characters.' >&2; exit 1; }
test -f "$1" && test -x "$1"
script_dir=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
unit_source="$script_dir/tinc-client-agent.service"
test -f "$unit_source" || { echo 'The systemd service unit is missing beside the installer.' >&2; exit 1; }
test -x /usr/sbin/tincd || { echo 'Install reviewed tinc 1.0.36 first.' >&2; exit 1; }
version=$(/usr/sbin/tincd --version 2>&1) || { echo 'Cannot query /usr/sbin/tincd version.' >&2; exit 1; }
printf '%s\n' "$version" | grep -Eq '^tinc version 1\.0\.36([[:space:]]|$)' || { echo 'This installer requires reviewed tinc version 1.0.36.' >&2; exit 1; }
test -x /usr/sbin/ip || { echo 'iproute2 is required at /usr/sbin/ip.' >&2; exit 1; }
ip_version=$(/usr/sbin/ip -Version 2>&1) || { echo 'Cannot query /usr/sbin/ip.' >&2; exit 1; }
printf '%s\n' "$ip_version" | grep -qi 'iproute2' || { echo '/usr/sbin/ip is not the expected iproute2 utility.' >&2; exit 1; }
test -x /usr/bin/ping || { echo 'iputils-ping is required at /usr/bin/ping.' >&2; exit 1; }
command -v systemctl >/dev/null 2>&1 || { echo 'systemd is required to install the Agent service.' >&2; exit 1; }
test -c /dev/net/tun && test -r /dev/net/tun && test -w /dev/net/tun || { echo 'A readable and writable /dev/net/tun device is required.' >&2; exit 1; }
test ! -L /usr/local/lib/tinc-client && test ! -L /etc/tinc-client && test ! -L /var/lib/tinc-client || { echo 'Refusing symlinked installation directories.' >&2; exit 1; }
test ! -e /etc/tinc-client/settings.json && test ! -L /etc/tinc-client/settings.json || { echo 'Existing settings: refusing overwrite.' >&2; exit 1; }
test ! -e /usr/local/lib/tinc-client/vpn-agent && test ! -L /usr/local/lib/tinc-client/vpn-agent || { echo 'Existing binary: refusing overwrite.' >&2; exit 1; }
test ! -e /etc/systemd/system/tinc-client-agent.service && test ! -L /etc/systemd/system/tinc-client-agent.service || { echo 'Existing systemd unit: refusing overwrite.' >&2; exit 1; }
install -d -m 755 /usr/local/lib/tinc-client
install -d -m 700 /etc/tinc-client /var/lib/tinc-client
install -m 755 "$1" /usr/local/lib/tinc-client/vpn-agent
printf '{"data_dir":"/var/lib/tinc-client","tinc_binary":"/usr/sbin/tincd","owner":"%s","interface":"%s","interface_prefix":32}\n' "$2" "$3" > /etc/tinc-client/settings.json
chmod 600 /etc/tinc-client/settings.json
install -m 644 "$unit_source" /etc/systemd/system/tinc-client-agent.service
systemctl daemon-reload
systemctl enable --now tinc-client-agent.service
systemctl is-active --quiet tinc-client-agent.service
