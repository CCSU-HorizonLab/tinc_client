#!/bin/sh
set -eu
# sudo ./install-agent.sh /absolute/path/vpn-agent INTERACTIVE_UID INTERFACE
agent_root=/usr/local/lib/tinc-client
config_root=/etc/tinc-client
unit_root=/etc/systemd/system
state_root=/var/lib/tinc-client
agent_target="$agent_root/vpn-agent"
settings_target="$config_root/settings.json"
unit_target="$unit_root/tinc-client-agent.service"

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
test ! -L "$agent_root" && test ! -L "$config_root" && test ! -L "$state_root" || { echo 'Refusing symlinked installation directories.' >&2; exit 1; }
test ! -e "$settings_target" && test ! -L "$settings_target" || { echo 'Existing settings: refusing overwrite.' >&2; exit 1; }
test ! -e "$agent_target" && test ! -L "$agent_target" || { echo 'Existing binary: refusing overwrite.' >&2; exit 1; }
test ! -e "$unit_target" && test ! -L "$unit_target" || { echo 'Existing systemd unit: refusing overwrite.' >&2; exit 1; }

install_started=0
install_succeeded=0
rollback_install() {
    status=$?
    trap - EXIT HUP INT TERM
    if test "$install_started" = 1 && test "$install_succeeded" = 0; then
        systemctl disable --now tinc-client-agent.service >/dev/null 2>&1 || true
        rm -f "$agent_target" "$settings_target" "$unit_target"
        systemctl daemon-reload >/dev/null 2>&1 || true
    fi
    exit "$status"
}
trap rollback_install EXIT HUP INT TERM

install_started=1
install -d -m 755 "$agent_root"
install -d -m 700 "$config_root" "$state_root"
install -m 755 "$1" "$agent_target"
printf '{"data_dir":"%s","tinc_binary":"/usr/sbin/tincd","owner":"%s","interface":"%s","interface_prefix":32}\n' "$state_root" "$2" "$3" > "$settings_target"
chmod 600 "$settings_target"
install -m 644 "$unit_source" "$unit_target"
systemctl daemon-reload
systemctl enable --now tinc-client-agent.service
systemctl is-active --quiet tinc-client-agent.service
install_succeeded=1
trap - EXIT HUP INT TERM
