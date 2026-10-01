# Memiro AI — current status

> Release history lives in [CHANGELOG.md](../CHANGELOG.md). This page tracks what is
> verified, what still needs real-world testing and what is planned.

**Latest release:** see [GitHub Releases](https://github.com/bglglzd/auris/releases/latest).

## Verified automatically (CI)

- Frontend unit tests, type check and build; core tests (`uxo-core`).
- Full Windows build of the app with `whisper,diarize,opus,parakeet`.
- On Windows, with real models: speaker separation of the pyannote reference recording
  (2 voices, ~99% of speech time labelled correctly) and Parakeet transcription.
- macOS builds of `Memiro AI.app` for Apple Silicon (Metal) and Intel, with ONNX Runtime
  1.23.2 bundled in `Contents/Frameworks`, and the speaker-separation e2e test on both.

## Needs testing on real calls

- End-to-end on Windows: record → Parakeet transcription → voices on a real meeting,
  including group calls (3+ people on the system track).
- Auto-recording of calls (the call detector and monitor are built but not field-tested).
- The update dialog (first visible when a version newer than 0.8.x is published).
- **macOS (new in 0.9):** microphone capture (CoreAudio) and system audio
  (ScreenCaptureKit) on a real call, the screen-recording permission flow, the menu bar,
  the translucent window and updates — compiled and bundled by CI, not yet run by users.

- **New in 0.11:** «Report a bug» on real Windows/macOS (browser opens the prefilled GitHub
  form, OS version via `system_info`), the macOS window following the app theme (sidebar
  material) and the restored primary-button colors — checked in the browser preview with a
  contrast audit of every button in both themes, not yet on a real Mac.

## Known limitations

- macOS builds are ad-hoc signed, not notarized: the first launch needs right-click → Open.
  Developer ID signing and notarization are wired in `release.yml` and need Apple secrets.
- Auto-recording of calls is Windows-only (the call detector uses WASAPI sessions).
- Parakeet covers 25 European languages; other languages fall back to Whisper.
- Speaker count tuning is based on reference recordings; the Voices panel lets users
  correct it (number of voices, merge) without re-transcribing.

## Ideas / roadmap

- Word-level timestamps to split a transcript line at a speaker change.
- Profanity filtering options and style variants for the clean-text preset.
- PDF export.
- Auto-recording of calls on macOS.
- Apple Developer ID signing and notarization.
