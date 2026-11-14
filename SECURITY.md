# Security policy

## Supported versions

Only the latest release receives fixes. Memiro updates itself (with your consent),
so please update before reporting.

| Version | Supported |
|---|---|
| latest release | ✅ |
| older versions | ❌ |

## Reporting a vulnerability

Please do not disclose vulnerabilities in a public issue. Use
[GitHub private vulnerability reporting](https://github.com/bglglzd/Memiro/security/advisories/new)
with a description, reproduction steps and affected versions. Do not include real API
keys, private audio or unredacted transcripts.

We acknowledge valid reports and coordinate a fix and release before public disclosure.

## Scope notes

- Audio, transcripts and settings (including the AI API key) are stored locally in the
  user's `%APPDATA%` profile.
- Network access: model downloads, the signed update check, and — only when the user
  uses AI features — requests to the user-configured AI endpoint.
- Updates are signed; the app verifies signatures with the public key embedded in the build.
