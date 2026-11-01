# Third-party notices

The bootstrap data-center addresses and ports in `telegram-protocol/src/dc.rs`
are transcribed from `td/telegram/net/ConnectionCreator.cpp` at TDLib commit
`a17f87c4cff7b90b278d12b91ba0614383aaee82`.

Upstream: <https://github.com/tdlib/td>

TDLib is distributed under the Boost Software License 1.0. The license text is
included at `../telegram-protocol/LICENSE-TDLIB-BOOST-1.0.txt`. No GPL Telegram
client implementation source is copied into this crate.


## Direct HTTPS Web-PKI dependencies

The first-party Telegram network owner uses the following general-purpose Rust
TLS/PKI crates for direct HTTPS certificate validation. They are dependencies,
not Telegram product owners, and no source from another Fabushi product crate
is reused for Telegram lifecycle policy.

- rustls 0.23.41 — upstream: https://github.com/rustls/rustls — license:
  Apache-2.0 OR ISC OR MIT.
- rustls-webpki 0.103.13 — distributed by the rustls project — license:
  ISC-style terms as declared by the upstream crate.
- webpki-roots 0.26.11 / 1.0.8 — upstream:
  https://github.com/rustls/webpki-roots — Mozilla CA-derived trust anchors;
  crate license MPL-2.0.
- ring 0.17.14 — upstream: https://github.com/briansmith/ring — license terms
  are declared by the upstream distribution and transitive files.

Exact package versions/checksums were source-transcribed from the already
pinned `native/global-dharma/Cargo.lock` during Stage A; no dependency resolver
or lock generator was executed.
