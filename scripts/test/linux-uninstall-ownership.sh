#!/bin/sh
set -eu

test "$(id -u)" = 0 || { echo 'Run this isolated uninstall test via sudo.' >&2; exit 1; }
project_root=$(CDPATH= cd -P "$(dirname "$0")/../.." && pwd)
source_script="$project_root/packaging/linux/uninstall-agent.sh"
test_root=$(mktemp -d /tmp/tinc-uninstall-test.XXXXXX)

cleanup() {
    case "$test_root" in
        /tmp/tinc-uninstall-test.*) rm -rf "$test_root";;
        *) echo "Refusing to clean unexpected test path: $test_root" >&2;;
    esac
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$test_root/bin" "$test_root/install" "$test_root/systemd"
cat > "$test_root/bin/systemctl" <<'MOCK_SYSTEMCTL'
#!/bin/sh
printf '%s\n' "$*" >> "$MOCK_SYSTEMCTL_LOG"
MOCK_SYSTEMCTL
chmod 755 "$test_root/bin/systemctl"

prepare_script() {
    sed \
        -e "s|^agent_target=.*|agent_target=$test_root/install/vpn-agent|" \
        -e "s|^unit_target=.*|unit_target=$test_root/systemd/tinc-client-agent.service|" \
        "$source_script" > "$test_root/uninstall-agent.sh"
}

run_uninstaller() {
    PATH="$test_root/bin:$PATH" \
    MOCK_SYSTEMCTL_LOG="$test_root/systemctl.log" \
    /bin/sh "$test_root/uninstall-agent.sh"
}

assert_rejected_without_systemctl() {
    label=$1
    : > "$test_root/systemctl.log"
    if run_uninstaller >/dev/null 2>&1; then
        echo "$label was accepted." >&2
        exit 1
    fi
    test ! -s "$test_root/systemctl.log" || { echo 'Untrusted Agent service reached systemctl.' >&2; exit 1; }
}

prepare_script
printf '[Service]\nExecStart=%s\n' "$test_root/install/vpn-agent" > "$test_root/systemd/tinc-client-agent.service"
chmod 644 "$test_root/systemd/tinc-client-agent.service"
run_uninstaller
grep -q '^disable --now tinc-client-agent.service$' "$test_root/systemctl.log" || { echo 'Trusted Agent service was not disabled.' >&2; exit 1; }

printf '[Service]\nExecStart=/tmp/not-tinc-connect\n' > "$test_root/systemd/tinc-client-agent.service"
assert_rejected_without_systemctl 'Unexpected executable path'

printf '[Service]\nExecStart=%s\n' "$test_root/install/vpn-agent" > "$test_root/systemd/tinc-client-agent.service"
chmod 666 "$test_root/systemd/tinc-client-agent.service"
assert_rejected_without_systemctl 'Writable service unit'

rm -f "$test_root/systemd/tinc-client-agent.service"
printf '[Service]\nExecStart=%s\n' "$test_root/install/vpn-agent" > "$test_root/linked-unit"
ln -s "$test_root/linked-unit" "$test_root/systemd/tinc-client-agent.service"
assert_rejected_without_systemctl 'Linked service unit'

echo 'PASS Linux Agent uninstall validates service ownership before disabling it'
