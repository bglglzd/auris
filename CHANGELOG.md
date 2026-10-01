# Changelog

All notable changes to Memiro AI. Versions follow [Semantic Versioning](https://semver.org/);
installers for every version are on [GitHub Releases](https://github.com/bglglzd/auris/releases).

## [0.11.0] — 2026-10-01

### Added
- **Report a bug** on Windows and macOS: describe the problem, choose how much it gets in the way and how often, and send — Memiro opens a ready-to-submit GitHub issue with the version, the system and a diagnostic log (personal data removed: API keys, tokens, e-mails, user names in paths; no meeting text). Copy or save the report as a file if you have no GitHub account. «My reports» keeps the history. Available from the sidebar (🐞), Settings → Errors and diagnostics, the tray menu, Help → Report a bug on macOS, and from error messages (pre-filled).
- Bug reports are triaged and fixed on a weekly schedule; fixes ship as regular updates.

### Fixed
- macOS: primary buttons (Save, Ask, Save file…) were invisible — white text on a white background in the light theme and dark-on-dark in the dark theme.
- macOS: the sidebar took the system's light/dark material instead of the app theme, so in the dark theme it looked grey and different from the rest of the window. The window now follows the app theme, and the sidebar uses the Memiro navy/light tint.
- Notifications no longer cover dialog buttons (they now sit under open dialogs).
- Dark theme: better contrast on the selected «Auto» voice-count button.
- Windows: the title bar follows the app's light/dark theme.

## [0.10.0] — 2026-09-26

### Changed
- **New name — Memiro AI** «память ваших встреч»: in the app, window, menus, notifications, installers and documentation; the listening mark, colors and design stay.
- Updating keeps everything: meetings, transcripts, reports, settings, voice names and downloaded models live in the same data folder.
- Windows: the installer removes the app installed under the previous name (files, shortcuts, the «Apps» entry), so there are no duplicates; the MSI upgrades a previous MSI install.
- Installers are now named `Memiro.AI_x.y.z_…`.

## [0.9.0] — 2026-09-26

### Added
- **Memiro for macOS** — Apple Silicon and Intel Macs (macOS 13 Ventura or newer), `.dmg` in every release and in-app updates like on Windows.
  - Two-track recording: your microphone through CoreAudio and the other side through ScreenCaptureKit (only sound, Memiro's own audio excluded). Without the «Screen & System Audio Recording» permission Memiro records the microphone and says so; Settings → Recording shows the status and opens the right System Settings page.
  - Native look following Apple's Human Interface Guidelines while staying recognizable: translucent sidebar with window vibrancy, traffic lights in the sidebar, SF system font, compact 13 pt controls, macOS-style lists, sheets and focus rings; the Memiro mark, gradients and speaker colors stay.
  - macOS menu bar: About, Settings ⌘,, Check for updates, Record, Note · just me, Import ⌘O, Edit (copy/paste), Search ⌘F, Light/dark ⌘⇧L, Window. Monochrome menu-bar icon.
  - Global hotkey ⌘⇧R; shortcuts shown with Mac symbols (⌘ ⇧ ⌥ ⌃).
  - Speech recognition and voices work as on Windows: Parakeet and speaker separation on ONNX Runtime (bundled), Whisper with Metal on Apple Silicon.

### Changed
- Auto-recording of calls is shown only on Windows (the call detector is Windows-specific).

### Fixed
- The web icon (`auris.svg`) was not valid SVG.

## [0.8.2] — 2026-09-26

### Added
- **Follows your AI server's model.** Memiro asks the server which models it serves (`GET /models`) on launch and every 6 hours; when you roll out a new model and the old name disappears, it switches to the current one automatically and tells you. As a safety net, if the server answers “model not found” during a request, Memiro picks the current model and retries. Leave the model field empty to always use what the server serves.
- **Check connection** in Settings → Artificial intelligence: server status, latency and the list of models to pick from; «Follow the server model» switch.
- **Meeting menu «⋯»** in the list: rename, notes and details, delete.
- **Meeting notes** — free-form notes per meeting, editable in the meeting and from the menu; the first line is shown in the list and search covers notes.

### Changed
- «Note · just me» and «Import recording» are now uniform sidebar buttons with icons (a person and a download arrow).
- The AI badge shows the model in use.
- Reasoning blocks (`<think>…</think>`) of reasoning models are removed from AI answers.

### Fixed
- Dialogs opened from the meeting list (delete confirmation, edit) are centered on the window instead of being clipped to the sidebar.

## [0.8.1] — 2026-09-25

### Added
- **AI agent brief** («Инструкция для ИИ») — a new AI preset that turns the conversation into a ready-to-paste prompt for Claude, Cursor, ChatGPT and other agents: task, context, requirements, constraints, exact details, acceptance criteria and open questions. Copied with Markdown intact.
- Release notes are shown in the in-app update dialog.

### Changed
- **Compact interface.** Everything fits on screen right after launch: the window opens centered at 1120×720 (minimum 720×520); denser sidebar, cards and buttons; transcript and voice actions are icon buttons with tooltips; audio tools live in the player row; theme and settings share one row.
- **New app icon** built from the app's mark — a dark tile with the blue-teal ear (installer, taskbar, tray, favicon).
- Settings: the auto-record section is collapsed while auto-record is off; the sticky footer no longer lets content show through.

### Fixed
- Wide tables in AI reports no longer overflow their card.

## [0.8.0] — 2026-09-25

### Added
- **NVIDIA Parakeet TDT 0.6B v3** speech recognition (ONNX Runtime) — the new default engine: accurate Russian with punctuation, 25 European languages, ~10× faster than real time on a CPU, no hallucinations on silence. Whisper is used automatically for other languages.
- **Voices panel**: talk-time share, voice sample playback, names, merge two voices, and change the number of voices instantly without re-transcribing.
- **Model manager** in Settings: status of every model, download in advance, delete unused ones.
- **AI presets without overlap**: Meeting summary, Tasks, Conversation review, Clean text, Follow-up e-mail. Speaker names are passed to the model. After transcription the AI sets the title and prepares the summary automatically (configurable).
- **Export dialog**: Word (.docx), Markdown, plain text and SRT subtitles — transcript plus selected AI reports in one document.
- **Updates with consent**: Memiro checks on launch and every 6 hours, shows what's new and installs + restarts only after you agree (never during a recording).

### Changed
- **Speaker separation rebuilt** on pyannote segmentation-3.0 + WeSpeaker ResNet34 (ONNX Runtime) with a new clustering algorithm; the speaker count is detected automatically, including group calls. About 25× faster than before.
- Whisper: large-v3-turbo is the default Whisper model; beam search; audio is split at pauses instead of mid-word.
- The old «Brief» and «Digest» reports are replaced by «Meeting summary» (saved ones are still shown and exported).

### Fixed
- «Downloading model» no longer appears on every transcription — models download exactly once.
- The previous diarization engine found no speech on some recordings, which produced a wrong number of speakers.

## [0.7.1] — 2026-08-18
### Fixed
- Audio editor: tracks are released before rewriting on Windows, zoom keeps the selection in view, Space no longer hijacks focused buttons, edit errors stay visible.

## [0.7.0] — 2026-08
### Added
- **Audio editor**: loudness timeline per track, cut ranges, “keep only this”, zoom, preview with cuts skipped, precise boundaries; applies to all tracks and shifts the transcript; one-click revert to the original.

## [0.6.3]
### Fixed
- More resilient system-audio (loopback) capture; live level meters while recording.

## [0.6.2]
### Fixed
- Centered play/pause icon in the player.

## [0.6.1]
### Added
- Edit transcripts (text, speaker, delete lines) and AI reports after generation.

## [0.6.0]
### Added
- Pause/resume with seamless segment merge and crash recovery.
- Auto-record debounce (ignores notification sounds, drops very short recordings).
- Solo mode — a note with a single voice.
- Copy as plain text.

## [0.5.x]
- Configurable global hotkey and auto-recording of calls; system-audio capture via polling; recorded tracks normalized before transcription; capture diagnostics; version shown in Settings; app binary renamed to `Memiro.exe`.

## [0.4.0]
- New name, logo and design system.

## [0.3.x]
- Liquid-glass redesign with light and dark themes and a responsive layout; drag-and-drop import; Ogg/Opus voice messages; stenogram export; AI brief, analysis and literary text.

## Earlier
- Two-track recording (microphone + system audio), local Whisper transcription, meeting library, import of external recordings, speaker diarization, AI summaries and questions via an OpenAI-compatible endpoint.

[0.11.0]: https://github.com/bglglzd/auris/releases/tag/v0.11.0
[0.10.0]: https://github.com/bglglzd/auris/releases/tag/v0.10.0
[0.9.0]: https://github.com/bglglzd/auris/releases/tag/v0.9.0
[0.8.2]: https://github.com/bglglzd/auris/releases/tag/v0.8.2
[0.8.1]: https://github.com/bglglzd/auris/releases/tag/v0.8.1
[0.8.0]: https://github.com/bglglzd/auris/releases/tag/v0.8.0
[0.7.1]: https://github.com/bglglzd/auris/releases/tag/v0.7.1
