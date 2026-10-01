# Memiro AI — «память ваших встреч»

Локальный десктоп-стенографист: записывает встречи/звонки, расшифровывает,
разделяет голоса (диаризация) и делает ИИ-отчёты. **Приватность прежде всего** —
запись, расшифровка и диаризация выполняются локально; ИИ-функции опциональны и
идут через личный API-ключ пользователя.

> Это главный контекстный файл проекта. Подробный runbook релиза — в
> [`docs/RELEASE.md`](docs/RELEASE.md). Дизайн-кит бренда — в
> `<local-design-directory>/auris-rebrand\design_handoff_auris\` (README = токены/
> экраны, IMPLEMENTATION = как внедрять). ТЗ на ИИ-модуль —
> `<local-design-directory>/Техническое задание для ИИ-модуля.docx`.

---

## 1. Бренд и технические идентификаторы (НЕ переименовывать)

Продукт — **Memiro AI** («память ваших встреч»; до 0.10 продукт назывался иначе). `productName` — `Memiro AI`, exe — `Memiro.exe` (`mainBinaryName`), на Mac —
`Memiro AI.app`, установщики — `Memiro.AI_x.y.z_…`. В тексте интерфейса — «Memiro»,
формальное имя (окно, «О программе», установщик, заголовки) — «Memiro AI». Репозиторий
пока `github.com/bglglzd/auris`. Несколько внутренних идентификаторов сохранили
историческое значение и **менять их нельзя** — это сотрёт данные и настройки у
пользователей:

| Артефакт | Значение | Почему не менять |
|---|---|---|
| `identifier` (tauri.conf.json) | `com.3uxo.app` | папка данных `%APPDATA%\com.3uxo.app` и идентичность апдейтера |
| база встреч | `3uxo.db` | переименование = потеря всех записей |
| бэкенд-лог | `3uxo.log` | диагностика, «Копировать лог» |
| ключи localStorage | `3uxo.settings/theme/labels.*/speakers.*/solo.*/titleEdited.*/autotitle.*` | слёт настроек/темы/подписей |
| `bundle.windows.wix.upgradeCode` | `1e80fb85-5177-5635-8650-55fc18e718a0` (из прежнего `productName`) | MSI Memiro AI обновляет прежнюю MSI-установку |
| NSIS-хук `src-tauri/windows/hooks.nsh` | удаляет программу под прежним именем после установки | иначе Memiro AI встаёт рядом со старой установкой (NSIS привязывает папку/ярлыки/«Программы» к `productName`) |

Эти значения — только в коде; в пользовательской документации их не упоминать.
Updater endpoint — `github.com/bglglzd/auris/releases/latest/download/latest.json`
(старые установки ходят на прежний адрес репозитория, GitHub перенаправляет).

---

## 2. Стек и структура

- **Tauri 2** (Rust) + **React 19 + TypeScript + Vite** (фронтенд).
- Cargo-workspace, 2 крейта:
  - **`core/`** (`uxo-core`) — доменная логика без GUI/Tauri, собирается и тестится
    на любой ОС.
  - **`src-tauri/`** (`memiro`, бинарь → `Memiro.exe`; lib `memiro_lib`) — тонкий
    Tauri-слой: команды (`commands.rs`), запуск/трей/хоткей/монитор (`lib.rs`).
- **`src/`** — React-фронтенд. **`docs/`** — релиз (`RELEASE.md`), статус
  (`STATUS.md`), скриншоты для README. История версий — `CHANGELOG.md` в корне.

### Модули `core/src/`
`ai` (ИИ-бэкенд + промпты-пресеты), `audio` (WAV-хелперы, `quiet_chunks`),
`call_detector` (детект звонка по аудио-сессиям WASAPI, Windows), `cluster`
(кластеризация голосов: AHC + отсев мелких кластеров, чистый Rust, тесты на любой
ОС), `cli_transcriber` (внешний whisper-CLI),
`decode` (symphonia+rubato → 16кГц/моно/i16; +opus за фичей), `diarize`
(pyannote segmentation-3.0 + wespeaker на ONNX Runtime за фичей), `edit` (карта громкости + вырезание
фрагментов + пересчёт расшифровки), `error`, `model`, `models` (каталог/статус/загрузка моделей), `recorder` (трейт +
MockRecorder), `service` (сервис-слой: запись/импорт/расшифровка/правка/файлы), `storage`
(rusqlite, миграции), `transcript` (модель + merge/assign_speakers), `transcriber`
(трейт), `whisper` (whisper-rs, за фичей), `wasapi_recorder` (реальный захват на
Windows, `#[cfg(windows)]`), `mac_recorder` (macOS: микрофон через cpal/CoreAudio +
системный звук через ScreenCaptureKit, `#[cfg(target_os = "macos")]`); общие для
захвата `audio::StreamResampler`/`TrackSink` (любая частота → 16 кГц моно WAV).

### Cargo-фичи (`core` зеркалит в `src-tauri`)
- `whisper` — встроенный whisper.cpp (whisper-rs).
- `gpu` — whisper с Vulkan (включает `whisper`).
- `metal` — whisper с Metal (Apple Silicon; включает `whisper`).
- `diarize` — диаризация (ONNX Runtime через `ort`, статически; бинарники ORT
  качаются при сборке с cdn.pyke.io).
- `opus` — декод Ogg/Opus (libopus через audiopus/cmake).
- `parakeet` — распознавание NVIDIA Parakeet TDT 0.6B v3 (ONNX Runtime).
- **Релиз собирает `--features gpu,diarize,opus,parakeet`** (Windows),
  **`metal,diarize,opus,parakeet`** (Mac arm64), **`whisper,diarize,opus,parakeet`**
  (Mac Intel); **CI check-app — `cargo build` с `whisper,diarize,opus,parakeet`**
  (без GPU); job `onnx-windows` — e2e диаризации и Parakeet на реальных моделях;
  job `macos` (arm64 + x86_64) — тесты ядра + e2e диаризации + `tauri build`.
- **ONNX Runtime на macOS** — `ort` с `load-dynamic` + `api-23` (у pyke нет сборки
  под Intel-Mac, у Microsoft Intel есть только до 1.23.2). Dylib 1.23.2 качает
  `scripts/fetch-onnxruntime-macos.sh` в `src-tauri/macos/` (в git не хранится),
  бандл кладёт её в `Contents/Frameworks`, `lib.rs` при старте ставит
  `ORT_DYLIB_PATH`. На Windows/Linux — прежняя статическая сборка (`api-27`).

---

## 3. Как это работает (потоки данных)

### Запись (Windows, WASAPI)
`wasapi_recorder.rs` пишет ДВЕ дорожки (16кГц/моно/i16 WAV) в
`<app_data>/meetings/<id>/`:
- `mic.wav` — микрофон (Capture-устройство, **event-режим** — события приходят).
- `system.wav` — системный звук через loopback (Render-устройство). **Loopback в
  shared-режиме НЕ шлёт WASAPI-события** (`events_ok=0`), поэтому опрашивается по
  таймеру (polling, 8мс); микрофон — на event-режиме.
- `capture_loop` логирует в `3uxo.log`: `reads / events_ok / samples / peak`
  (peak≈0 → захвачена тишина).
- Старт/стоп — кнопка, глобальный хоткей или трей; событие `recording-changed`.

### Расшифровка
- Команда `transcribe` (commands.rs), движок — `load_asr` по `models::pick_model`:
  **Parakeet** (по умолчанию) или Whisper. Модель скачивается один раз (фаза
  `download`), путь `<app_data>/models`. Прогресс — событие `transcribe-progress`.
- **Записанные дорожки нормализуются** через `decode::decode_to_wav_16k_mono`
  (тот же декодер, что и импорт) ПЕРЕД whisper — выравнивает «сырой» WASAPI-WAV с
  рабочим путём импорта. Лог: `transcribed: mic=N segs, system=N segs`.
- Записанная встреча: `mic` (=«Я») + `system` (=«Собеседник») → `merge_tracks`.
  При выборе ≥2 собеседников системная дорожка диаризуется (`assign_speakers`).
- Импортированная встреча: одна дорожка `audio.wav` → whisper → (с фичей diarize)
  диаризация на N голосов.

### Запись (macOS, v0.9)
`mac_recorder.rs`: `mic.wav` — cpal (CoreAudio, поток в своём треде, любая частота
→ `TrackSink`); `system.wav` — **macOS 14.2+: Core Audio process tap**
(`mac_audiotap.rs`: `CATapDescription` моно без своего процесса → приватное
агрегатное устройство → IOProc; функции tap через `dlsym` — на 13 не падает;
разрешение «Только запись системного звука», `NSAudioCaptureUsageDescription`),
иначе/при ошибке — ScreenCaptureKit («Запись экрана и системного звука»).
`TrackSink`/`StreamResampler` — оконный sinc (Блэкман) → 16 кГц, без алиасинга.
Статусы: микрофон — `AVCaptureDevice`, системный звук — `TCCAccessPreflight`
(TCC.framework через dlopen; нет — `unknown`). Команды `mac_permissions`,
`request_mic_access`, `request_system_audio_access`, `open_privacy_settings`.
Фронт: `MacSetup` («Подготовка Mac» при первом запуске, флаг
`3uxo.macsetup.done`, событие `memiro-mac-setup`) и `MacPermissions` в
настройках. Уведомления на Mac по умолчанию выкл (`settings.notifications`,
команды `set_notifications`/`test_notification`). Без доступа — запись с
микрофона + `recording-warning`. Авто-запись на Mac скрыта (детектор — WASAPI).
**Подпись**: секреты `MACOS_CERT_P12`/`MACOS_CERT_PASSWORD` (самоподписанный,
`scripts/make-macos-signing-cert.sh`) → `scripts/import-macos-signing-cert.sh` →
`APPLE_SIGNING_IDENTITY`; стабильная подпись = разрешения не сбрасываются.
Проверка mac-кода без Mac: `rustup target add aarch64-apple-darwin` + отдельный
крейт с модулем (cpal/screencapturekit требуют Xcode).

### Импорт
`service::import_to_meeting` → `decode_to_wav_16k_mono(src, audio.wav)` (symphonia
+ rubato; opus отдельной фичей). Поддерживает m4a/mp3/wav/flac/ogg/opus и т.п.

### Диаризация (v0.8.0)
`diarize.rs` (фича `diarize`): `OnnxDiarizer` — pyannote **segmentation-3.0**
(ONNX, окна 10 с с шагом 5 с, powerset → до 3 локальных голосов на окно) +
**wespeaker ResNet34-LM** (эмбеддинг голоса по чистой речи локального говорящего,
Kaldi fbank 80). Модели (~33 МБ) — GitHub-релизы (не HF), качаются ОДИН раз в
`<app_data>/models/diarize`. Число голосов и разметку считает `cluster.rs`:
AHC (центроид, косинус, порог `AUTO_THRESHOLD`=0.45, подобран на эталонах) →
мелкие кластеры (<2% речи, 3…25 с) — не люди → k-means-уточнение. Явное число
голосов соблюдается. Эмбеддинги кешируются в `<id>/diarization.json` →
`recluster_speakers` меняет число голосов мгновенно, текст (и правки) не
трогает. Записанная встреча: «Я» = микрофон, системная дорожка делится
автоматически (1 голос → «Собеседник»). Отладка точности:
`cargo run --release -p uxo-core --features diarize --example diar_eval`;
e2e-тест `core/tests/diarize_e2e.rs` (`-- --ignored`, в CI на Windows).
Прежний движок native-pyannote-rs (Burn) на эталоне pyannote вообще не находил
речь — отсюда «неверное число спикеров» до v0.8.

### Правка аудио (отдельный экран, v0.7.0)
`edit.rs`: `waveform` (пик+RMS по корзинам, 0..1000 — как `recording_levels`),
`apply_cuts` (PCM i16 → i16 без перекодирования), `remap_transcript` (сдвиг
времён реплик). `service::apply_audio_edit_files` режет ВСЕ дорожки встречи
одним набором вырезов (иначе «Я»/«Собеседник» разъедутся), перед первой правкой
кладёт бэкап `mic.orig.wav`/`system.orig.wav`/`audio.orig.wav` +
`transcript.orig.json`; `revert_audio_edit_files` возвращает оригинал. Команды:
`waveform`, `audio_edit_state`, `apply_audio_edit`, `revert_audio_edit`.
Фронт — `AudioEditor`/`WaveLane` + чистая логика `audioedit.ts`.

### Parakeet (v0.8.0, движок по умолчанию)
`parakeet.rs` (фича `parakeet`): NVIDIA Parakeet TDT 0.6B v3 int8 (экспорт
sherpa-onnx, GitHub-релиз `.tar.bz2` ~490 МБ, распаковка tar+bzip2 на чистом Rust в
`<app_data>/models/parakeet-tdt-0.6b-v3`). Признаки — `nemo_mel.rs` (log-mel NeMo,
сверено с librosa до 1e-4), encoder → жадное TDT (joiner: токен + пропуск
кадров 0..4), кадр 80 мс, окна ~15 с с разрезом в паузе (на 30 с TDT терял
хвосты). 25 европейских языков, пунктуация; `models::pick_model` берёт Whisper,
если язык вне списка. Отладка: `--example asr_eval`, e2e `core/tests/parakeet_e2e.rs`.

### Обновления (v0.8.0)
`src/updater.ts` + `UpdateDialog`: проверка при запуске и каждые 6 ч (и кнопкой в
настройках) → диалог «Доступно обновление» с заметками релиза → по согласию
скачивание с прогрессом, установка, `relaunch()`. Во время записи кнопка
неактивна. «Позже» откладывает версию до следующего запуска. Заметки —
только для своей платформы (`notesForPlatform`: разделы `### macOS` /
`### Windows`, метки строк `[mac]` / `[win]`; формат — docs/RELEASE.md).

### Модели (v0.8.0)
`models.rs`: каталог Whisper (по умолчанию **large-v3-turbo-q8_0**), статус,
загрузка (атомарно через `.part`), удаление. Прогресс «Скачивание модели»
шлётся ТОЛЬКО при реальной загрузке (баг ≤0.7: диаризация всегда слала
download → «качает модель» при каждой расшифровке). Команды `models_status`,
`download_model`, `delete_model`; UI — `ModelsManager` в настройках. Whisper:
beam search 5, suppress_nst, окна режутся в паузах (`audio::quiet_chunks`).

### ИИ (опционально, через ключ пользователя)
`ai.rs`: OpenAI-совместимый HTTP-бэкенд (`base_url`/`api_key`/`model` из настроек).
С v0.8 — пресеты без пересечений (`ai::report_prompt`, команда `generate_report`):
**Итоги встречи** (`summary.md`), **Задачи** (`tasks.md`), **Разбор разговора**
(`analysis.md`), **Чистовой текст** (`literary.md`), **Письмо по итогам**
(`followup.md`), **Инструкция для ИИ** (`agent.md`, промпт для ИИ-агента; с v0.8.1);
старое «Краткое резюме» (`brief.md`) только показывается.
В промпт идут заголовок/участники и имена голосов из интерфейса
(`MeetingContext`). Длинные — заметки по частям → итог (literary — склейка).
После расшифровки `src/aiauto.ts` сам ставит заголовок (если не правился
вручную: `3uxo.titleEdited.<id>`) и строит «Итоги» (настройки `aiAuto`).
Старые команды (`summarize`, `brief_summary`…) оставлены для совместимости.
**Модель сервера (v0.8.2):** `ai::list_models` (`GET {base}/models`, форматы OpenAI и
Ollama), `ai::pick_model` (настроенная → ближайшая по префиксу → первая),
`ai::check` (команда `ai_check`). `HttpChatBackend`: пустая модель/`auto` → модель
сервера; ответ «model not found» (HTTP 400/404/422) → перечитать список и повторить
один раз; `<think>…</think>` вырезается. Фронт: `src/aimodel.ts::syncServerModel`
при запуске и каждые 6 ч (`aiAuto.followModel`) → тост «модель обновилась».

### Глобальный хоткей + авто-запись
- `update_hotkey(accelerator)` (lib.rs) — настраиваемый глобальный хоткей старт/
  стоп (дефолт Ctrl+Shift+R). `setup_global_shortcut` не паникует при «занято»
  (unregister_all + register, иначе старт-краш). Фронт перерегистрирует при старте.
- Авто-запись звонков: `call_detector::any_active_call(processes)` +
  фоновый поток-монитор (`spawn_autorecord_monitor`) + команда `set_autorecord`.
  Конфиг приложений — `src/autorecord.ts`. **Движок собран, рантайм НЕ проверен.**

---

## 4. Фронтенд (`src/`)

- **Дизайн-система Memiro** (`App.css`): CSS-токены на `:root`/`[data-theme]`
  (светлая/тёмная). Палитра: `--brand-grad` (сине-бирюзовый), `--teal`
  (приватность), `--record` (запись), `--violet` (ИИ/спикер-3), `--spk-0..5`
  (аватары спикеров). Шрифты: **Manrope** (заголовки/лого, `--font-display`) +
  **Golos Text** (UI, `--font-ui`) + JetBrains Mono. Keyframes: `ripple` (кольца-
  эхо), `recpulse`, `blink`, `wv` (эквалайзер).
- **`MemiroMark.tsx`** — фирменный знак (ухо), градиент по теме. Лок-ап в сайдбаре:
  знак + `memiro` с плашкой `AI` (`.brand-ai`) │ `память ваших встреч`.
- **Аудио-редактор**: `AudioEditor` (отдельный полноэкранный режим из плеера
  встречи — волна громкости по дорожкам, выделение протяжкой, вырезы, зум,
  предпрослушивание без вырезов, «Применить»/«Вернуть оригинал»), `WaveLane`
  (одна дорожка), `audioedit.ts` (вырезы/линейка/путь волны — чистые функции).
- Компоненты: `Sidebar`, `RecordButton`, `MeetingList`, `MeetingView`,
  `TranscriptView` (лента с аватарами-инициалами), `AiPanel` (pill «ИИ · ваш
  ключ»), `SettingsModal` (раскрывающиеся секции: Запись/хоткей, Авто-запись,
  Распознавание, ИИ, Диагностика; версия в футере), `ImportModal`, `HotkeyCapture`,
  `CopyLogButton`, `Markdown`.
- Состояние: `settings.ts` (localStorage `3uxo.settings`), `theme.ts`, `labels.ts`,
  `api.ts` (обёртки `invoke`). Тема применяется до рендера (`initTheme`).
- **macOS-вид (v0.9)**: `platform.ts` ставит `data-platform="macos"` до рендера;
  в конце `App.css` — слой `:root[data-platform="macos"]` (Apple HIG: шрифт SF,
  прозрачное окно + нативная вибрация `windowEffects: sidebar` под сайдбаром,
  «светофор» в сайдбаре, полосы `.mac-drag` с `data-tauri-drag-region`, контролы
  13 pt). Знак, градиенты и цвета спикеров — те же. Подписи сочетаний —
  `formatAccel`/`accelKeys` (⌘⇧R), хоткей по умолчанию — `defaultHotkey()`.
  Строка меню macOS (`setup_mac_menu` в lib.rs) шлёт `app-menu` → `appmenu.ts`
  (`useAppMenu`). Окно/бандл Mac — `src-tauri/tauri.macos.conf.json`
  (`macOSPrivateApi`, Overlay-заголовок, frameworks, entitlements, мин. 13.0,
  ad-hoc подпись), `Info.plist`, `Entitlements.plist`. Собирать Mac только через
  `npm run tauri` — CLI сам включает фичу `macos-private-api`. Тема окна
  синхронизируется с темой приложения (`theme.ts` → `setTheme`, право
  `core:window:allow-set-theme`): иначе вибрация берёт системную тему. Варианты
  `.btn.primary/.danger` в mac-слое переопределяются явно (общий mac-фон кнопки
  специфичнее). Тосты — под модальными окнами (z-index 45 < 50).
- Экспорт (v0.8): одна кнопка «⬇ Экспорт» → `ExportModal`: Word (.docx — свой
  генератор `docx.ts`, без зависимостей), Markdown, TXT, субтитры SRT; в документ
  складываются стенограмма (опц. таймкоды) + выбранные ИИ-отчёты.
- Встречи (v0.8.2): меню «⋯» в `MeetingList` (переименовать / заметки / удалить) →
  `MeetingEditDialog` (порталом в body: у сайдбара `backdrop-filter`); заметки —
  столбец `notes` в БД (миграция), команда `update_meeting_notes`, поле в
  `MeetingView`, первая строка — в списке, поиск по заметкам.
- **Отчёты об ошибках (v0.11)**: `BugReportModal` + чистая логика `bugreport.ts`
  (`sanitize` — убирает пути с именем, ключи, токены, e-mail; `issueUrl` —
  заполненная форма `.github/ISSUE_TEMPLATE/app_report.yml`, заголовок `[app] …`,
  ссылка ≤ 7600 символов; «Мои отчёты» — localStorage `3uxo.bugreports`). Открыть
  из любого места — `openBugReport(prefill)` (событие `memiro-bug-report`): кнопка 🐞
  в футере сайдбара, Настройки → «Ошибки и диагностика», трей и Справка на Mac
  (`app-menu` `bug`), баннеры ошибок в `MeetingView`/`AiPanel`. Версия ОС —
  команда `system_info`. Разбор: еженедельный routine в сессии Claude — issues
  `[app]`/`bug` → дубли/уточнения → исправления PR → релиз патча (docs/RELEASE.md).
- Голоса (v0.8): `SpeakersPanel` — доля речи, «▶ Образец», имена, «Объединить»,
  число голосов Авто/1…6 (мгновенный пересчёт); чистая логика — `speakers.ts`.

---

## 5. Сборка и запуск

```bash
npm install                       # зависимости фронта
npm run dev                       # vite dev (порт 1420) — UI без бэкенда
npm run build                     # tsc + vite build (проверка фронта)
npm test                          # vitest (юнит-тесты фронта)
npx tsc --noEmit                  # проверка типов
npm run tauri dev -- --features whisper,diarize,opus,parakeet   # полное приложение (Windows)
```

- Ядро (`cargo test -p uxo-core`) и фронт проверяются локально на любой ОС;
  Windows-сборку приложения и e2e-тесты моделей гоняет CI. ONNX-фичи локально на
  Linux без доступа к cdn.pyke.io: `ORT_LIB_PATH=<onnxruntime>/lib
  ORT_PREFER_DYNAMIC_LINK=1` (onnxruntime с GitHub-релизов Microsoft).
- **Версию держать синхронно** в `package.json` и `src-tauri/tauri.conf.json`.
- Превью UI: MCP `preview_*` (vite на localhost:1420). `MeetingView`/`AiPanel`
  без данных бэкенда не отрисовать; `SettingsModal` и старт-экран — можно.
  Скриншот-инструмент таймаутит на бесконечных анимациях (ripple) — для кадра
  заморозить (`*{animation:none}`) или убрать `.ripple-ring`.

---

## 6. Деплой (релиз) — кратко

Релиз = подписанный GitHub-релиз `vX.Y.Z` + `latest.json`. Полный порядок —
[`docs/RELEASE.md`](docs/RELEASE.md). Кратко: ветка от `main` → бамп версии в
`package.json` + `tauri.conf.json` → запись в `CHANGELOG.md` → PR → **все jobs CI
`conclusion: success`** → squash-merge → тег `vX.Y.Z` на main **или** ручной запуск
`release.yml` с входами `tag` и `notes` (notes = «что нового», их видит окно
обновления) → проверить `gh release view` и `latest.json`.

- **CI** (`ci.yml`): `frontend`, `core`, `check-app` (Windows, полный `cargo build`
  со всеми фичами кроме gpu), `onnx-windows` (e2e диаризации и Parakeet).
- **Release** (`release.yml`): Windows, LLVM + Vulkan SDK,
  `--features gpu,diarize,opus,parakeet`, подпись `TAURI_SIGNING_PRIVATE_KEY`.

---

## 7. Текущее состояние

История версий — [`CHANGELOG.md`](CHANGELOG.md); проверено/не проверено/планы —
[`docs/STATUS.md`](docs/STATUS.md). Обновлять там, не здесь.

---

## 8. Правила и уроки (соблюдать!)

- **Проверять `conclusion` CI, а не код `gh run watch`.** `gh run watch` мог
  завершиться 0, пока сборка падала → так v0.5.0 уехал сломанным. Перед тегом:
  `gh run view <id> --json conclusion --jq .conclusion` == `success`.
- **Прямой push в `main` запрещён** (авто-режим) → только PR + `gh pr merge`.
  Push тега `git push origin <tag>` — разрешён.
- **Платформенное (WASAPI, реальные звонки) проверяет CI-сборка + рантайм-тест
  пользователя** на Windows; модели — e2e-тестами в CI.
- **Единый стиль Memiro** для всего нового (токены, шрифты, знак, семантика цвета,
  3 обещания: приватность/спокойствие/живость). Не ломать дизайн-язык.
- **«Ничего не убираем — только добавляем»** — не удалять существующие функции
  (TXT/MD-экспорт, кнопки и т.п.) при доработках.
- **Превью UI** — скриншоты в 800×600 и 1120×720, обе темы (Playwright + мок
  `__TAURI_INTERNALS__`), до мержа.
- `gh pr edit --base` ломается (GraphQL projectCards) → ретаргет базы через REST:
  `gh api -X PATCH repos/bglglzd/auris/pulls/N -f base=main`.
- Git identity: `bglglzd <248948303+bglglzd@users.noreply.github.com>`; never use a personal email in public commits.

---

## 9. Частые команды (gh/git)

```bash
gh pr create --base main --head <branch> --title "..." --body "..."
gh pr merge <N> --squash --delete-branch
gh run list --workflow=ci.yml --branch <branch> --limit 1 --json databaseId --jq '.[0].databaseId'
gh run view <id> --json conclusion --jq .conclusion         # проверка результата
gh run list --workflow=release.yml --limit 3
gh release view vX.Y.Z --json tagName,isDraft,assets
gh api repos/bglglzd/auris/releases/latest --jq .tag_name
```
