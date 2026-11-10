# Windows native acceptance in CI

The `check-app` job builds an unsigned debug NSIS installer with the native
`whisper,diarize,opus,parakeet` features. On the same fresh GitHub-hosted Windows
runner, `scripts/native-acceptance.mjs` verifies the build manifest and installer
digest, installs into a new temporary directory, and starts the installed app.
It refuses to run on a user machine or over an existing Memiro data folder.

`prepare-native-acceptance.mjs` creates a CI-only build overlay with a fresh
WebView2 data directory and explicit loopback CDP arguments. WebView2 150 and
newer [ignore environment overrides in elevated hosts](https://github.com/MicrosoftEdge/WebView2Feedback/issues/5645),
so the overlay passes these options through Tauri's API configuration. It keeps
the normal window settings and Wry defaults. Release workflows do not use it.
The generated launch metadata and configuration digest are bound to the build
manifest. Startup failures retain owned-child output, backend logs when present,
and process/version diagnostics limited to the owned process tree.

Playwright connects to the real WebView2 using its documented
[CDP integration](https://playwright.dev/docs/webview2). The harness supplies
controlled paths for OS open/save dialogs. Every other intercepted invocation
continues to the actual Tauri backend; the renderer is the built application.

The journey checks:

- Invalid audio produces an error without a library row; a valid import then
  succeeds using the native decoder and SQLite.
- The UI runs the default local speech model on an explicitly synthetic Russian
  WAV. The resulting text and segment times are checked, saved and shown in a
  screenshot. No microphone or configured AI server is used.
- The export dialog creates DOCX, Markdown, plain text and SRT files. The harness
  opens each file, validates the DOCX ZIP/XML and SRT time format, and checks that
  every recognized segment is present.
- A failed native file write stays visible and can be retried successfully.

`memiro-windows-native-acceptance` contains `result.json`, the actual screenshot,
transcript and exported files. The result binds its source SHA and installer
SHA-256 to `BUILD-MANIFEST.json` in the installer artifact. A failed run retains
its failure report/screenshot when the app reached that stage.

Model downloads require access to the public model hosts on the first run.
This is a Windows installation/import/transcription/export check. It does not
exercise microphone capture, real call detection, paid AI, macOS GUI, upgrades,
uninstall, signing or updater delivery.
