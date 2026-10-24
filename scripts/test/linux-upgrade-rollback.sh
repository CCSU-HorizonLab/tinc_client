#!/bin/sh
set -eu

test "$(id -u)" = 0 || { echo 'Run this isolated upgrade test via sudo.' >&2; exit 1; }
project_root=$(CDPATH= cd -P "$(dirname "$0")/../.." && pwd)
source_script="$project_root/packaging/linux/upgrade-agent.sh"
test_root=$(mktemp -d /tmp/tinc-upgrade-test.XXXXXX)

cleanup() {
    case "$test_root" in
        /tmp/tinc-upgrade-test.*) rm -rf "$test_root";;
        *) echo "Refusing to clean unexpected test path: $test_root" >&2;;
    esac
}
trap cleanup EXIT HUP INT TERM

prepare_case() {
    name=$1
    case_root="$test_root/$name"
    mkdir -p "$case_root/bin" "$case_root/install" "$case_root/config" "$case_root/systemd" "$case_root/state"
    printf 'old-agent-%s\n' "$name" > "$case_root/install/vpn-agent"
    printf '{"owner":"1000","sentinel":"keep-%s"}\n' "$name" > "$case_root/config/settings.json"
    printf 'old-unit-%s\n' "$name" > "$case_root/systemd/tinc-client-agent.service"
    printf 'new-agent-%s\n' "$name" > "$case_root/new-agent"
    printf 'new-unit-%s\n' "$name" > "$case_root/tinc-client-agent.service"
    chmod 755 "$case_root/install/vpn-agent" "$case_root/new-agent"
    chmod 600 "$case_root/config/settings.json"
    chmod 644 "$case_root/systemd/tinc-client-agent.service" "$case_root/tinc-client-agent.service"

    sed \
        -e "s|^agent_root=.*|agent_root=$case_root/install|" \
        -e "s|^config_root=.*|config_root=$case_root/config|" \
        -e "s|^unit_root=.*|unit_root=$case_root/systemd|" \
        -e "s|^state_root=.*|state_root=$case_root/state|" \
        "$source_script" > "$case_root/upgrade-agent.sh"

    cat > "$case_root/bin/systemctl" <<'MOCK_SYSTEMCTL'
#!/bin/sh
set -eu
case "$1" in
    is-active|stop|daemon-reload) exit 0;;
    start)
        count=0
        test ! -f "$MOCK_START_COUNT" || count=$(cat "$MOCK_START_COUNT")
        count=$((count + 1))
        printf '%s\n' "$count" > "$MOCK_START_COUNT"
        if test -f "$MOCK_FAIL_FIRST_START" && test "$count" = 1; then
            exit 1
        fi
        exit 0
        ;;
    *) echo "Unexpected mocked systemctl command: $*" >&2; exit 1;;
esac
MOCK_SYSTEMCTL
    chmod 755 "$case_root/bin/systemctl"
}

assert_no_backup() {
    if find "$case_root/state" -maxdepth 1 -name 'upgrade.*' -print | grep -q .; then
        echo "Upgrade backup was not cleaned: $case_root" >&2
        exit 1
    fi
}

prepare_case success
PATH="$case_root/bin:$PATH" \
MOCK_START_COUNT="$case_root/start-count" \
MOCK_FAIL_FIRST_START="$case_root/no-failure" \
/bin/sh "$case_root/upgrade-agent.sh" "$case_root/new-agent"
cmp -s "$case_root/new-agent" "$case_root/install/vpn-agent" || { echo 'Successful upgrade did not install the new Agent.' >&2; exit 1; }
grep -q 'keep-success' "$case_root/config/settings.json" || { echo 'Successful upgrade changed settings.' >&2; exit 1; }
grep -q 'new-unit-success' "$case_root/systemd/tinc-client-agent.service" || { echo 'Successful upgrade did not install the new unit.' >&2; exit 1; }
assert_no_backup

prepare_case rollback
touch "$case_root/fail-first-start"
if PATH="$case_root/bin:$PATH" \
   MOCK_START_COUNT="$case_root/start-count" \
   MOCK_FAIL_FIRST_START="$case_root/fail-first-start" \
   /bin/sh "$case_root/upgrade-agent.sh" "$case_root/new-agent"; then
    echo 'Expected the simulated first service start to fail.' >&2
    exit 1
fi
grep -q 'old-agent-rollback' "$case_root/install/vpn-agent" || { echo 'Rollback did not restore the old Agent.' >&2; exit 1; }
grep -q 'old-unit-rollback' "$case_root/systemd/tinc-client-agent.service" || { echo 'Rollback did not restore the old unit.' >&2; exit 1; }
grep -q 'keep-rollback' "$case_root/config/settings.json" || { echo 'Rollback changed settings.' >&2; exit 1; }
test "$(cat "$case_root/start-count")" = 2 || { echo 'Rollback did not attempt to restart the restored service.' >&2; exit 1; }
assert_no_backup

echo 'PASS Linux Agent upgrade preserves settings and rolls back failed service starts'
