#!/bin/sh
set -eu
test "$(id -u)" = 0 || { echo 'Run via sudo.' >&2; exit 1; }
systemctl disable --now tinc-client-agent.service
echo 'Service stopped and disabled. Binary, unit, configuration and keys retained for explicit administrator removal.'
