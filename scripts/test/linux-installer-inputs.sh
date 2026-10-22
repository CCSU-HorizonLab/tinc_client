#!/bin/sh
set -eu

installer=$(CDPATH= cd -P "$(dirname "$0")/../.." && pwd)/packaging/linux/install-agent.sh
setup=$(CDPATH= cd -P "$(dirname "$0")/../.." && pwd)/packaging/linux/setup-agent.sh

test "$(id -u)" = 0 || { echo 'Run this non-mutating input test via sudo.' >&2; exit 1; }
command -v getent >/dev/null 2>&1 || { echo 'getent is required to select a test owner UID.' >&2; exit 1; }

owner_uid=${SUDO_UID:-}
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
test -n "$owner_uid" || { echo 'No non-root local UID is available for installer input tests.' >&2; exit 1; }

expect_failure() {
    expected=$1
    shift
    if output=$("$installer" "$@" 2>&1); then
        echo "Expected installer rejection containing: $expected" >&2
        exit 1
    fi
    case "$output" in
        *"$expected"*) ;;
        *)
            echo "Unexpected installer rejection: $output" >&2
            exit 1
            ;;
    esac
}

expect_failure 'AGENT_BINARY must be an absolute path' relative-agent "$owner_uid" tinc
expect_failure 'Invalid UID' /does/not/exist not-a-uid tinc
expect_failure 'OWNER_UID must identify a non-root desktop user' /does/not/exist 0 tinc
expect_failure 'OWNER_UID does not identify a local user' /does/not/exist 4294967294 tinc
expect_failure 'Invalid interface name' /does/not/exist "$owner_uid" 'bad/name'
expect_failure 'Linux interface names must not exceed 15 characters' /does/not/exist "$owner_uid" tinc_interface_too_long

if output=$(env -u SUDO_UID -u PKEXEC_UID /bin/sh "$setup" 2>&1); then
    echo 'Expected setup wrapper to reject a missing desktop user UID' >&2
    exit 1
fi
case "$output" in
    *'Unable to identify the non-root desktop user.'*) ;;
    *) echo "Unexpected setup wrapper rejection: $output" >&2; exit 1;;
esac

echo 'PASS Linux installer rejects unsafe inputs before making system changes'
