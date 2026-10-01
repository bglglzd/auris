# Releasing Memiro AI

A release is a signed, published GitHub release with a tag `vX.Y.Z` plus a generated
`latest.json`. Installed copies of Memiro find it through the updater, show the release
notes and install it after the user agrees.

## Checklist

1. **Branch** from the latest `main` (direct pushes to `main` are not used — everything goes through a PR).
2. **Bump the version** in both `package.json` and `src-tauri/tauri.conf.json` (keep them equal), then run `npm install --package-lock-only`.
3. **Update `CHANGELOG.md`** — a new section at the top.
4. **Check locally**:
   ```bash
   npx tsc --noEmit && npm test && npm run build
   cargo test -p uxo-core
   ```
5. **Open a PR** and wait until every CI job has `conclusion: success`
   (check the conclusion, not the exit code of `gh run watch`):
   ```bash
   gh run view <run-id> --json conclusion --jq .conclusion   # → success
   ```
6. **Squash-merge** the PR.
7. **Start the release build** — either way works:
   - push a tag on `main`:
     ```bash
     git checkout main && git pull
     git tag vX.Y.Z && git push origin vX.Y.Z
     ```
   - or run **Actions → release → Run workflow** on `main` with `tag = vX.Y.Z` and
     `notes` = the changelog section (Markdown). The workflow creates the tag itself.
8. **Verify the publication**:
   ```bash
   gh run view <release-run-id> --json conclusion --jq .conclusion    # success
   gh release view vX.Y.Z --json isDraft,assets                        # setup.exe, .msi, .sig, latest.json
   curl -sL https://github.com/bglglzd/auris/releases/latest/download/latest.json | grep version
   ```

The release notes (`notes` input or the release body) are what users see in the
«Update available» dialog, so write them for users.

## Workflows

### `ci.yml` — every push to `main` and every PR
| Job | Runner | What it checks |
|---|---|---|
| `frontend` | ubuntu | `npm ci`, `npm test`, `tsc`, `vite build` |
| `core` | ubuntu | `cargo test -p uxo-core` |
| `check-app` | windows | full `cargo build` of the app with `whisper,diarize,opus,parakeet` (catches link errors) |
| `onnx-windows` | windows | ONNX tests + end-to-end diarization and Parakeet tests on real models |
| `macos (arm64)` / `macos (x86_64)` | macos-15 / macos-15-intel | core + ONNX tests and diarization e2e on ONNX Runtime 1.23.2, `tauri build --debug --bundles app` (Metal on arm64), checks the bundled dylib, Info.plist and architecture |

### `release.yml` — tag `v*` or manual run
1. `build-windows` — LLVM + Vulkan SDK, `tauri-action` with
   `--features gpu,diarize,opus,parakeet`; creates the non-draft release «Memiro AI vX.Y.Z»
   with `latest.json`.
2. `build-macos` (after Windows, one architecture at a time) — Apple Silicon on
   `macos-15` with `metal,diarize,opus,parakeet`, Intel on `macos-15-intel` with
   `whisper,diarize,opus,parakeet`. Downloads ONNX Runtime 1.23.2
   (`scripts/fetch-onnxruntime-macos.sh`), uploads `.dmg` and `.app.tar.gz` (+ `.sig`) to
   the same release and adds `darwin-aarch64` / `darwin-x86_64` to `latest.json`.

All update artifacts are signed with `TAURI_SIGNING_PRIVATE_KEY`. macOS apps are ad-hoc
signed; for Developer ID signing and notarization add the `APPLE_*` secrets listed in
`release.yml` and pass them to the macOS step.

After a release check that `latest.json` lists `windows-x86_64`, `darwin-aarch64` and
`darwin-x86_64`.

## Bug reports from the app

Users send reports from the app (🐞 / Settings / tray / Help menu). They arrive as GitHub
issues created from `.github/ISSUE_TEMPLATE/app_report.yml`, titled `[app] …` and labelled
`bug`, with version, OS and a sanitized diagnostic log. A weekly routine triages them:
deduplicate and answer, ask for details where needed, fix confirmed bugs in a PR with
tests, release a patch version and close the fixed issues with a link to the release.

## Updater

- The endpoint is set in `src-tauri/tauri.conf.json → plugins.updater.endpoints`
  and points at the latest GitHub release of this repository.
- Signatures are verified with the public key in `plugins.updater.pubkey`; the private
  key exists only in repository secrets.
- `releases/latest` is the newest non-draft, non-prerelease release, so users always
  jump straight to the latest version.

## Pitfalls

- **Never tag before CI is green** — a tag on a broken commit produces a failed release build.
- **Keep versions in sync** in `package.json` and `tauri.conf.json`.
- **Do not change** `bundle.windows.wix.upgradeCode` (derived from the previous product name:
  it lets the MSI upgrade old installs) or remove `src-tauri/windows/hooks.nsh` (removes
  the app installed under the previous name after installing Memiro AI). Changing `productName` again needs
  the same kind of migration: NSIS keys the install folder, shortcuts and the «Apps» entry
  by the product name.
- **Do not change** the app `identifier` in `tauri.conf.json`: it defines the data folder
  and updater identity of existing installations.
- **Icons**: regenerate from a 1024×1024 PNG/SVG with `npx tauri icon <file> -o src-tauri/icons`
  and delete the generated `android/` and `ios/` folders (desktop-only app).

## Fixing a broken release

If the release build fails, nothing is published and users are unaffected. Fix it in a
new PR and release the next patch version (`vX.Y.Z+1`). An unpublished tag can be
deleted with `git push origin :refs/tags/vX.Y.Z`.
