#!/bin/sh
set -eu

agent_target=/usr/local/lib/tinc-client/vpn-agent
settings_target=/etc/tinc-client/settings.json
unit_target=/etc/systemd/system/tinc-client-agent.service
state_root=/var/lib/tinc-client

test "$(id -u)" = 0 || { echo 'Run via sudo or pkexec.' >&2; exit 1; }
test "$#" = 1 || { echo 'Usage: upgrade-agent.sh AGENT_BINARY' >&2; exit 1; }
case "$1" in /*) ;; *) echo 'AGENT_BINARY must be an absolute path' >&2; exit 1;; esac
test -f "$1" && test -x "$1" || { echo 'The packaged Agent binary is missing or not executable.' >&2; exit 1; }

script_dir=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
unit_source="$script_dir/tinc-client-agent.service"
test -f "$unit_source" || { echo 'The systemd service unit is missing beside the upgrader.' >&2; exit 1; }
command -v systemctl >/dev/null 2>&1 || { echo 'systemd is required to upgrade the Agent service.' >&2; exit 1; }
command -v stat >/dev/null 2>&1 || { echo 'stat is required to validate the installed Agent.' >&2; exit 1; }

for directory in /usr/local/lib/tinc-client /etc/tinc-client "$state_root"; do
    test -d "$directory" && test ! -L "$directory" || { echo "Unsafe or missing installation directory: $directory" >&2; exit 1; }
done
for installed in "$agent_target" "$settings_target" "$unit_target"; do
    test -f "$installed" && test ! -L "$installed" || { echo "Incomplete or unsafe Agent installation: $installed" >&2; exit 1; }
    test "$(stat -c %u "$installed")" = 0 || { echo "Installed Agent file is not owned by root: $installed" >&2; exit 1; }
    mode=$(stat -c %a "$installed")
    case "$mode" in
        *[2367][0-7]|*[0-7][2367]) echo "Installed Agent file is writable outside root: $installed" >&2; exit 1;;
    esac
done

backup_dir=$(mktemp -d "$state_root/upgrade.XXXXXX")
case "$backup_dir" in "$state_root"/upgrade.*) ;; *) echo 'Unexpected upgrade backup path.' >&2; exit 1;; esac
chmod 700 "$backup_dir"

was_active=0
if systemctl is-active --quiet tinc-client-agent.service; then
    was_active=1
fi
upgrade_started=0
upgrade_succeeded=0

restore_previous() {
    install -m 755 "$backup_dir/vpn-agent" "$agent_target" || return 1
    install -m 644 "$backup_dir/tinc-client-agent.service" "$unit_target" || return 1
    systemctl daemon-reload || return 1
    if test "$was_active" = 1; then
        systemctl start tinc-client-agent.service || return 1
        systemctl is-active --quiet tinc-client-agent.service || return 1
    fi
}

cleanup_backup() {
    case "$backup_dir" in
        "$state_root"/upgrade.*)
            rm -f "$backup_dir/vpn-agent" "$backup_dir/tinc-client-agent.service"
            rmdir "$backup_dir" 2>/dev/null || true
            ;;
    esac
}

finish() {
    code=$?
    trap - EXIT HUP INT TERM
    set +e
    if test "$upgrade_started" = 1 && test "$upgrade_succeeded" != 1; then
        if ! restore_previous; then
            echo 'Agent upgrade failed and rollback could not restore the running service.' >&2
            code=1
        fi
    fi
    cleanup_backup
    exit "$code"
}
trap finish EXIT
trap 'exit 1' HUP INT TERM

install -m 755 "$agent_target" "$backup_dir/vpn-agent"
install -m 644 "$unit_target" "$backup_dir/tinc-client-agent.service"
upgrade_started=1
if test "$was_active" = 1; then
    systemctl stop tinc-client-agent.service
fi
install -m 755 "$1" "$agent_target"
install -m 644 "$unit_source" "$unit_target"
systemctl daemon-reload
if test "$was_active" = 1; then
    systemctl start tinc-client-agent.service
    systemctl is-active --quiet tinc-client-agent.service
fi
upgrade_succeeded=1
echo 'Tinc Client Agent upgrade completed. Existing settings, profiles and keys were retained.'
