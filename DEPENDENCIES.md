# Dependency maintenance

Run the locked installation and audits from the repository root:

```sh
npm ci
npm run check:tauri-versions
npm audit --audit-level=low
cargo install cargo-audit --locked --version 0.22.2
cargo audit --file Cargo.lock
cargo test --locked -p uxo-core
```

CI runs both audits. Update npm and Cargo locks together when refreshing Tauri
or a paired plugin; the build checks installed JavaScript and native major/minor
versions. Use Node.js 24 LTS for the tested development and CI environment.

The security refresh moves bundled SQLite to the patched `libsqlite3-sys` 0.39
series through rusqlite 0.41, updates TLS trust/verification and XML parsers, and
uses the updater's supported ZIP 9 dependency. Existing meeting schemas and data
remain unchanged. Meeting durations are checked against SQLite's signed integer
range before insert or update.

## Remaining upstream warnings

The November 2026 audit has no known vulnerabilities and still reports these
informational warnings. No advisory is ignored in CI:

- `audiopus_sys` 0.1.8 is unmaintained ([RUSTSEC-2026-0150](https://rustsec.org/advisories/RUSTSEC-2026-0150.html)).
  It supports the optional `opus` audio decoder used in desktop builds. There is
  no patched compatible release in this dependency line; replacing the decoder
  needs separate format and platform validation.
- `proc-macro-error` and the older `zbus_names`/`zvariant`/`zvariant_derive`
  dependency lines have maintenance warnings. They are transitive desktop
  dependencies; compatible updates are retained in the lock.
- GTK3's `glib` 0.18.5 has an unsoundness warning
  ([RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html)).
  It belongs to the Linux GTK dependency graph and is absent from the Windows
  target graph. The GTK3 line has no compatible fix. This refresh does not claim
  to fix that upstream issue or establish Linux application support.

Repeat the audits when updating: the advisory database can change independently
of this source. Model/runtime downloads and native acceptance are separate from
package auditing; a clean audit does not prove every platform or feature works.
