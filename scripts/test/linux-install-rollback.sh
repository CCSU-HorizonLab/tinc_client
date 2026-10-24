#!/bin/sh
set -eu

test "$(id -u)" = 0 || { echo 'Run this isolated install test via sudo.' >&2; exit 1; }
project_root=$(CDPATH= cd -P "$(dirname "$0")/../.." && pwd)
source_script="$project_root/packaging/linux/install-agent.sh"
test_root=$(mktemp -d /tmp/tinc-install-test.XXXXXX)

cleanup() {
    case "$test_root" in
        /tmp/tinc-install-test.*) rm -rf "$test_root";;
        *) echo "Refusing to clean unexpected test path: $test_root" >&2;;
    esac
}
trap cleanup EXIT HUP INT TERM

prepare_case() {
    name=$1
    case_root="$test_root/$name"
    mkdir -p "$case_root/bin" "$case_root/payload" "$case_root/install" "$case_root/config" "$case_root/systemd" "$case_root/state"
    printf '#!/bin/sh\necho tinc version 1.0.36\n' > "$case_root/tincd"
    printf '#!/bin/sh\necho ip utility, iproute2\n' > "$case_root/ip"
    printf '#!/bin/sh\nexit 0\n' > "$case_root/ping"
    printf 'new-agent-%s\n' "$name" > "$case_root/payload/vpn-agent"
    printf 'new-unit-%s\n' "$name" > "$case_root/payload/tinc-client-agent.service"
    chmod 755 "$case_root/tincd" "$case_root/ip" "$case_root/ping" "$case_root/payload/vpn-agent"
    chmod 644 "$case_root/payload/tinc-client-agent.service"

    sed \
        -e "s|^agent_root=.*|agent_root=$case_root/install|" \
        -e "s|^config_root=.*|config_root=$case_root/config|" \
        -e "s|^unit_root=.*|unit_root=$case_root/systemd|" \
        -e "s|^state_root=.*|state_root=$case_root/state|" \
        -e "s|/usr/sbin/tincd|$case_root/tincd|g" \
        -e "s|/usr/sbin/ip|$case_root/ip|g" \
        -e "s|/usr/bin/ping|$case_root/ping|g" \
        -e "s|test -c /dev/net/tun && test -r /dev/net/tun && test -w /dev/net/tun|test -f $case_root/tun|" \
        "$source_script" > "$case_root/payload/install-agent.sh"
    touch "$case_root/tun"

    cat > "$case_root/bin/getent" <<'MOCK_GETENT'
#!/bin/sh
test "$1" = passwd && test "$2" = 1000
MOCK_GETENT
    cat > "$case_root/bin/systemctl" <<'MOCK_SYSTEMCTL'
#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$MOCK_SYSTEMCTL_LOG"
case "$1" in
    daemon-reload|disable) exit 0;;
    enable) test ! -f "$MOCK_FAIL_ENABLE";;
    is-active) exit 0;;
    *) echo "Unexpected mocked systemctl command: $*" >&2; exit 1;;
esac
MOCK_SYSTEMCTL
    chmod 755 "$case_root/bin/getent" "$case_root/bin/systemctl"
}

run_installer() {
    PATH="$case_root/bin:$PATH" \
    MOCK_SYSTEMCTL_LOG="$case_root/systemctl.log" \
    MOCK_FAIL_ENABLE="$case_root/fail-enable" \
    /bin/sh "$case_root/payload/install-agent.sh" "$case_root/payload/vpn-agent" 1000 tinc
}

prepare_case success
run_installer
grep -q 'new-agent-success' "$case_root/install/vpn-agent" || { echo 'Successful install did not install the Agent.' >&2; exit 1; }
grep -q '"data_dir":".*/state"' "$case_root/config/settings.json" || { echo 'Successful install wrote invalid settings.' >&2; exit 1; }
grep -q 'new-unit-success' "$case_root/systemd/tinc-client-agent.service" || { echo 'Successful install did not install the unit.' >&2; exit 1; }

prepare_case rollback
touch "$case_root/fail-enable"
if run_installer; then
    echo 'Expected the simulated service enable to fail.' >&2
    exit 1
fi
for created in "$case_root/install/vpn-agent" "$case_root/config/settings.json" "$case_root/systemd/tinc-client-agent.service"; do
    test ! -e "$created" || { echo "Install rollback left a managed file: $created" >&2; exit 1; }
done
grep -q 'disable --now tinc-client-agent.service' "$case_root/systemctl.log" || { echo 'Install rollback did not disable the partial service.' >&2; exit 1; }
test "$(grep -c '^daemon-reload$' "$case_root/systemctl.log")" = 2 || { echo 'Install rollback did not refresh systemd after cleanup.' >&2; exit 1; }

echo 'PASS Linux Agent install removes partial managed files after service failure'
