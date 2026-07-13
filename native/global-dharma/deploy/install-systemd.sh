#!/usr/bin/env bash
set -euo pipefail

# Run after the verified release binaries have been unpacked. This script never
# downloads arbitrary code and requires an administrator-owned configuration.
prefix="${PREFIX:-/usr/local}"
config_dir="${CONFIG_DIR:-/etc/global-dharma}"
state_dir="${STATE_DIR:-/var/lib/global-dharma}"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

test -x "${prefix}/bin/global-dharma-daemon" || { echo "missing verified daemon in ${prefix}/bin" >&2; exit 2; }
test -f "${config_dir}/global-dharma.toml" || { echo "missing ${config_dir}/global-dharma.toml" >&2; exit 2; }
id -u global-dharma >/dev/null 2>&1 || useradd --system --home "${state_dir}" --shell /usr/sbin/nologin global-dharma
install -d -o global-dharma -g global-dharma -m 0750 "${state_dir}"
install -d -o root -g global-dharma -m 0750 "${config_dir}"
chmod 0640 "${config_dir}/global-dharma.toml"
install -m 0644 "${script_dir}/global-dharma.service" /etc/systemd/system/global-dharma.service
systemctl daemon-reload
systemctl enable --now global-dharma.service
systemctl is-active --quiet global-dharma.service
