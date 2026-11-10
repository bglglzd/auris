# Contributing to Memiro AI

Thanks for helping improve Memiro!

## Before opening an issue

- Search existing issues and pull requests first.
- For a bug, include the Memiro version (Settings → footer), Windows version, exact
  steps, expected and actual result. The log is in **Settings → Diagnostics → Copy log**.
- Never post API keys, private audio or unredacted transcripts in a public issue.

## Development setup

The product platform is Windows; the core crate and the frontend build and test on any OS.

```bash
npm ci
npm test
npx tsc --noEmit
npm run build
cargo test -p uxo-core
```

Running the full app needs Windows with Rust, Node.js 24 LTS, CMake and LLVM
(`npm run tauri dev -- --features whisper,diarize,opus,parakeet`). See the
[README](README.md#build-from-source) for details.

Run the checks in [DEPENDENCIES.md](DEPENDENCIES.md) when changing dependency
locks. Native Tauri and JavaScript plugin versions must be updated together.

## Pull requests

1. Start from the current `main` and keep the change focused.
2. Explain the user-visible problem and link the related issue.
3. Add or update tests when behaviour changes.
4. Run the checks for the parts you touched and list them in the PR description.
5. Keep refactors and formatting-only changes out of bug fixes.
6. UI changes follow the Memiro design system in `src/App.css` (tokens, fonts, both themes).
7. Preserve local recognition by default. Optional AI sends meeting text/context
   and optional server recognition sends audio to the user's configured endpoint.
   Document those boundaries; never use private audio or credentials in tests.

The Windows CI also runs [native installation/import/ASR/export acceptance](docs/native-acceptance.md)
on a fresh hosted runner. Do not run that harness against a personal app profile.

By contributing you agree that your work is distributed under the repository's MIT license.
