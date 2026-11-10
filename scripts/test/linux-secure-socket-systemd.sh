#!/bin/sh
set -eu

test "$(id -u)" = 0 || { echo 'Run this restricted systemd test via sudo.' >&2; exit 1; }
command -v systemd-run >/dev/null 2>&1 || { echo 'systemd-run is required.' >&2; exit 1; }
command -v getent >/dev/null 2>&1 || { echo 'getent is required.' >&2; exit 1; }

project_root=$(CDPATH= cd -P "$(dirname "$0")/../.." && pwd)
fixture=${1:-$project_root/target/debug/examples/secure_socket_systemd_fixture}
case "$fixture" in
    /*) ;;
    *) fixture=$project_root/$fixture;;
esac
test -x "$fixture" || { echo "Secure socket fixture is not executable: $fixture" >&2; exit 1; }

owner_uid=${2:-${SUDO_UID:-}}
case "$owner_uid" in ''|*[!0-9]*|0) owner_uid=;; esac
if test -n "$owner_uid" && ! getent passwd "$owner_uid" >/dev/null 2>&1; then
    owner_uid=
fi
if test -z "$owner_uid"; then
    owner_uid=$(getent passwd | while IFS=: read -r _ _ uid _; do
        case "$uid" in
            ''|*[!0-9]*|0) ;;
            *) printf '%s\n' "$uid"; break;;
        esac
    done)
fi
test -n "$owner_uid" || { echo 'No non-root local UID is available for the systemd test.' >&2; exit 1; }

unit=tinc-client-secure-socket-test-$$
test_root=/run/$unit
case "$test_root" in /run/tinc-client-secure-socket-test-[0-9]*) ;; *) echo 'Unsafe test path.' >&2; exit 1;; esac

cleanup() {
    systemctl stop "$unit.service" >/dev/null 2>&1 || true
    systemctl reset-failed "$unit.service" >/dev/null 2>&1 || true
    case "$test_root" in /run/tinc-client-secure-socket-test-[0-9]*) rm -rf "$test_root";; esac
}
trap cleanup EXIT HUP INT TERM

install -d -m 755 "$test_root"
install -m 755 "$fixture" "$test_root/secure-socket-systemd-fixture"

if ! output=$(systemd-run --quiet --wait --pipe --collect \
    --unit "$unit" \
    --property Type=oneshot \
    --property User=root \
    --property Group=root \
    --property RuntimeDirectory="$unit" \
    --property RuntimeDirectoryMode=0755 \
    --property UMask=0077 \
    --property NoNewPrivileges=yes \
    --property ProtectHome=yes \
    --property ProtectSystem=strict \
    --property ReadWritePaths="$test_root" \
    --property PrivateTmp=yes \
    --property 'RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK' \
    --property 'CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_CHOWN CAP_DAC_OVERRIDE' \
    "$test_root/secure-socket-systemd-fixture" "$test_root" "$owner_uid" 2>&1); then
    printf '%s\n' "$output" >&2
    exit 1
fi
printf '%s\n' "$output"
case "$output" in *'LEGACY_CHMOD_ERRNO=1'*) ;; *) echo 'Legacy syscall did not prove chmod EPERM.' >&2; exit 1;; esac
case "$output" in *"SECURE_SOCKET_OK uid=$owner_uid mode=600"*) ;; *) echo 'Fixed socket ownership or mode was not verified.' >&2; exit 1;; esac

echo 'PASS restricted systemd service reproduces legacy EPERM and accepts chmod-before-chown'
