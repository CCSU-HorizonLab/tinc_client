#!/bin/sh
set -eu

agent_target=/usr/local/lib/tinc-client/vpn-agent
unit_target=/etc/systemd/system/tinc-client-agent.service

test "$(id -u)" = 0 || { echo 'Run via sudo.' >&2; exit 1; }
command -v systemctl >/dev/null 2>&1 || { echo 'systemd is required to remove the Agent service.' >&2; exit 1; }
command -v stat >/dev/null 2>&1 || { echo 'stat is required to validate the Agent service.' >&2; exit 1; }
test -f "$unit_target" && test ! -L "$unit_target" || { echo 'Refusing to disable a missing or linked Agent service unit.' >&2; exit 1; }
test "$(stat -c %u "$unit_target")" = 0 || { echo 'Refusing to disable an Agent service unit not owned by root.' >&2; exit 1; }
if find "$unit_target" -maxdepth 0 -perm /022 -print | grep -q .; then
    echo 'Refusing to disable a group- or world-writable Agent service unit.' >&2
    exit 1
fi
test "$(grep -Fxc "ExecStart=$agent_target" "$unit_target")" = 1 || {
    echo 'Refusing to disable an Agent service with an unexpected executable path.' >&2
    exit 1
}
systemctl disable --now tinc-client-agent.service
echo 'Service stopped and disabled. Binary, unit, configuration and keys retained for explicit administrator removal.'
