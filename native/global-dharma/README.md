# Global Dharma Rust runtime

This runtime delivers only to administrator-configured, HTTPS-authorized nodes. It does not scan networks, discover targets, broadcast UDP, control hotspots, alter system time, or run self-propagating updates.

After verifying the release archive checksum and Cosign bundle, unpack it at
`/usr/local` so the binaries live in `/usr/local/bin`. Copy
`config/global-dharma.toml.example` to `/etc/global-dharma/global-dharma.toml`,
replace the node identity, then run `global-dharmactl validate-config` followed
by `global-dharmactl install-systemd`. The daemon listens only on loopback.

For containers: `docker compose -f deploy/compose.yaml up --build`.
