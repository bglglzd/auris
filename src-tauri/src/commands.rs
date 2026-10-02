use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use uxo_core::ai::{AiConfig, HttpChatBackend, MetadataSuggestion};
use uxo_core::cli_transcriber::{CliTranscriber, TranscribeOptions};
use uxo_core::edit::{Range, Waveform};
use uxo_core::error::{AppError, AppResult};
use uxo_core::model::Meeting;
use uxo_core::recorder::Recorder;
use uxo_core::service::{self, ActiveRecording, AudioEditState};
use uxo_core::storage::Repo;
use uxo_core::transcript::Transcript;

/// Событие прогресса расшифровки для фронтенда.
#[derive(Clone, serde::Serialize)]
pub struct TranscribeProgress {
    pub id: String,
    /// "loading" | "download" (модель распознавания, один раз) |
    /// "download-voices" (модели голосов, один раз) | "mic" | "system" | "diarize".
    pub stage: String,
    pub percent: f32,
    /// Сколько фрагментов готово / всего (0 — неизвестно/не применимо).
    pub done: u32,
    pub total: u32,
}

/// Показывает нативное уведомление Windows (тихо игнорирует ошибки).
/// Системные уведомления (старт/стоп записи и т.п.). На macOS по умолчанию
/// выключены — не просим лишнее разрешение; включаются в настройках.
static NOTIFY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(!cfg!(target_os = "macos"));

pub(crate) fn notify<R: tauri::Runtime>(app: &tauri::AppHandle<R>, title: &str, body: &str) {
    if NOTIFY.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

/// Включает/выключает системные уведомления (из настроек).
#[tauri::command]
pub fn set_notifications(enabled: bool) {
    NOTIFY.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Пробное уведомление — заодно macOS спросит разрешение, когда пользователь
/// сам включил уведомления.
#[tauri::command]
pub fn test_notification(app: AppHandle) {
    let _ = app
        .notification()
        .builder()
        .title("Memiro AI")
        .body("Уведомления включены")
        .show();
}

/// Дописывает строку в файл лога `<data_root>/3uxo.log` (переживает краш).
pub(crate) fn flog(data_root: &Path, msg: &str) {
    use std::io::Write;
    let line = format!("[{}] {}\n", chrono::Utc::now().to_rfc3339(), msg);
    let path = data_root.join("3uxo.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Возвращает «хвост» бэкенд-лога (последние ~64 КБ) для диагностики.
#[tauri::command]
pub fn get_backend_log(state: tauri::State<AppState>) -> AppResult<String> {
    let path = state.data_root.join("3uxo.log");
    if !path.exists() {
        return Ok(String::new());
    }
    let data = std::fs::read(&path)?;
    let start = data.len().saturating_sub(64 * 1024);
    Ok(String::from_utf8_lossy(&data[start..]).to_string())
}

/// Текст расшифровки встречи для подачи в ИИ (ошибка, если ещё не расшифровано).
fn meeting_transcript_text(data_root: &Path, id: &str) -> AppResult<String> {
    let transcript = service::load_transcript(data_root, id)?.ok_or_else(|| {
        AppError::InvalidState("нет расшифровки — сначала расшифруйте встречу".into())
    })?;
    Ok(uxo_core::ai::transcript_to_text(&transcript))
}

/// Конфиг авто-записи звонков (читается фоновым монитором).
#[derive(Default, Clone)]
pub struct AutoRecordCfg {
    pub enabled: bool,
    /// Имена процессов (.exe) для слежения за аудио-сессиями.
    pub processes: Vec<String>,
    pub auto_stop: bool,
    /// Сколько секунд звонок должен держаться непрерывно до старта записи —
    /// отсекает короткие звуки уведомлений (Telegram «дзынь» ~2 с). 0 — старт
    /// сразу при первом детекте.
    pub start_delay_secs: u32,
    /// Авто-удалять записи короче этого порога (сек), если их начала авто-запись
    /// (защита от мусорных огрызков-уведомлений). 0 — ничего не удалять.
    pub min_keep_secs: u32,
}

/// Глобальное состояние приложения.
pub struct AppState {
    pub data_root: PathBuf,
    pub repo: Mutex<Repo>,
    pub recorder: Box<dyn Recorder>,
    pub active: Mutex<Option<ActiveRecording>>,
    /// Настройки авто-записи; фоновый монитор опрашивает их и стартует/стопит.
    pub autorecord: Arc<Mutex<AutoRecordCfg>>,
}

/// Обновляет конфиг авто-записи (вызывается фронтом при загрузке и сохранении
/// настроек). Сам мониторинг ведёт фоновый поток (см. lib.rs).
#[tauri::command]
pub fn set_autorecord(
    state: tauri::State<AppState>,
    enabled: bool,
    processes: Vec<String>,
    auto_stop: bool,
    start_delay_secs: u32,
    min_keep_secs: u32,
) {
    let mut cfg = state.autorecord.lock().unwrap();
    cfg.enabled = enabled;
    cfg.processes = processes;
    cfg.auto_stop = auto_stop;
    cfg.start_delay_secs = start_delay_secs;
    cfg.min_keep_secs = min_keep_secs;
}

#[tauri::command]
pub fn start_recording(app: AppHandle, state: tauri::State<AppState>) -> AppResult<String> {
    let mut active = state.active.lock().unwrap();
    if active.is_some() {
        return Err(AppError::InvalidState("already recording".into()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let rec = service::start_recording(state.recorder.as_ref(), &state.data_root, id.clone())?;
    *active = Some(rec);
    notify(&app, "🔴 Memiro — запись начата", "Идёт запись звонка");
    report_recorder_warning(&app, &state);
    Ok(id)
}

/// Если запись стартовала неполной (напр. на macOS без доступа к системному
/// звуку) — пишет в лог и шлёт фронтенду событие `recording-warning`.
pub fn report_recorder_warning<R: tauri::Runtime>(app: &tauri::AppHandle<R>, state: &AppState) {
    if let Some(w) = state.recorder.warning() {
        flog(&state.data_root, &format!("recorder warning: {w}"));
        let _ = app.emit("recording-warning", w);
    }
}

/// Какая платформа: фронтенд подстраивает вид и подписи (⌘ на macOS).
#[tauri::command]
pub fn platform() -> &'static str {
    std::env::consts::OS
}

/// Сведения о системе для отчёта об ошибке: ОС, её версия, архитектура.
#[derive(Clone, serde::Serialize)]
pub struct SystemInfo {
    pub os: String,
    pub os_version: String,
    pub arch: String,
}

#[tauri::command]
pub fn system_info() -> SystemInfo {
    SystemInfo {
        os: std::env::consts::OS.to_string(),
        os_version: os_version().unwrap_or_default(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

/// Версия ОС: на macOS — `sw_vers` (15.1), на Windows — `ver` (10.0.26100.x;
/// сборка ≥ 22000 — это Windows 11). Без консольного окна.
fn os_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("sw_vers").arg("-productVersion").output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        // «Microsoft Windows [Version 10.0.26100.1]» / «[Версия …]» — язык не важен.
        let inner = text.split('[').nth(1)?.split(']').next()?;
        let ver = inner.split_whitespace().last()?.to_string();
        let build: u32 = ver.split('.').nth(2).and_then(|b| b.parse().ok()).unwrap_or(0);
        let name = if build >= 22000 { "Windows 11" } else { "Windows 10" };
        Some(format!("{name} ({ver})"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        std::fs::read_to_string("/etc/os-release").ok().and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
    }
}

/// Разрешения macOS для записи: доступ к звуку собеседников. `request = true`
/// — показать системный запрос. На других ОС всегда `true`.
#[tauri::command]
pub fn system_audio_access(request: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        if request {
            uxo_core::mac_recorder::request_system_audio();
        }
        uxo_core::mac_recorder::system_audio_status() == "granted"
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        true
    }
}

/// Статус разрешений для «Подготовки Mac»: `granted` / `denied` /
/// `undetermined` / `unknown`. `system_audio_mode`: `audio` — «Только запись
/// системного звука» (macOS 14.2+), `screen` — «Запись экрана и системного
/// звука» (13–14.1). На других ОС — всё `granted`.
#[derive(Clone, serde::Serialize)]
pub struct MacPermissions {
    pub mic: &'static str,
    pub system_audio: &'static str,
    pub system_audio_mode: &'static str,
}

#[tauri::command]
pub fn mac_permissions() -> MacPermissions {
    #[cfg(target_os = "macos")]
    {
        MacPermissions {
            mic: uxo_core::mac_audiotap::mic_status(),
            system_audio: uxo_core::mac_recorder::system_audio_status(),
            system_audio_mode: uxo_core::mac_recorder::system_audio_mode(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        MacPermissions { mic: "granted", system_audio: "granted", system_audio_mode: "audio" }
    }
}

/// Системный запрос: микрофон.
#[tauri::command]
pub fn request_mic_access() {
    #[cfg(target_os = "macos")]
    uxo_core::mac_audiotap::request_mic();
}

/// Системный запрос: звук собеседников.
#[tauri::command]
pub fn request_system_audio_access() {
    #[cfg(target_os = "macos")]
    uxo_core::mac_recorder::request_system_audio();
}

/// Открывает нужный раздел «Конфиденциальность и безопасность» (macOS):
/// `kind` = "screen" (запись экрана и системного звука) или "mic".
#[tauri::command]
pub fn open_privacy_settings(kind: String) -> AppResult<()> {
    #[cfg(target_os = "macos")]
    {
        let pane = if kind == "mic" { "Privacy_Microphone" } else { "Privacy_ScreenCapture" };
        std::process::Command::new("open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{pane}"
            ))
            .spawn()?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Err(AppError::InvalidState("only on macOS".into()))
    }
}

#[tauri::command]
pub async fn stop_recording(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
) -> AppResult<Meeting> {
    // Берём активную запись и сразу отпускаем lock, чтобы не держать его
    // во время блокирующего I/O в recorder.stop() (важно для Плана 2).
    let current = {
        let mut active = state.active.lock().unwrap();
        active
            .take()
            .ok_or_else(|| AppError::InvalidState("not recording".into()))?
    };
    let created_at = chrono::Utc::now().to_rfc3339();
    let meeting = {
        let repo = state.repo.lock().unwrap();
        service::stop_recording(state.recorder.as_ref(), &repo, &current, created_at)?
    };
    // Диагностика захвата: размеры записанных дорожек. Пустой WAV (~44 байт
    // заголовка, ~0 с) означает, что WASAPI ничего не захватил — это укажет на
    // причину «ничего не распознано».
    for tf in ["mic.wav", "system.wav"] {
        if let Ok(p) = service::track_path(&state.data_root, &meeting.id, tf) {
            let sz = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            let secs = sz.saturating_sub(44) / 32_000; // 16кГц * 2 байта моно
            flog(&state.data_root, &format!("recorded {tf} = {sz} bytes (~{secs}s)"));
        }
    }
    notify(&app, "✅ Memiro — запись сохранена", &meeting.title);
    Ok(meeting)
}

/// Состояние записи для фронтенда: идёт ли запись и стоит ли она на паузе.
#[derive(Clone, serde::Serialize)]
pub struct RecState {
    pub recording: bool,
    pub paused: bool,
}

#[tauri::command]
pub fn recording_state(state: tauri::State<AppState>) -> RecState {
    match state.active.lock().unwrap().as_ref() {
        Some(rec) => RecState {
            recording: true,
            paused: rec.current.is_none(),
        },
        None => RecState {
            recording: false,
            paused: false,
        },
    }
}

/// Текущий уровень дорожек (0..1000) для живых индикаторов записи.
/// Не идёт запись → нули. Читается-и-сбрасывается (peak с прошлого опроса).
#[tauri::command]
pub fn recording_levels(state: tauri::State<AppState>) -> uxo_core::recorder::TrackLevels {
    state.recorder.levels()
}

/// Ставит текущую запись на паузу (финализирует текущий сегмент). Сегменты
/// склеятся в один файл на стопе.
#[tauri::command]
pub fn pause_recording(app: AppHandle, state: tauri::State<AppState>) -> AppResult<()> {
    let current = {
        state
            .active
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| AppError::InvalidState("not recording".into()))?
    };
    let updated = service::pause_recording(state.recorder.as_ref(), current)?;
    *state.active.lock().unwrap() = Some(updated);
    notify(&app, "⏸ Memiro — пауза", "Запись на паузе");
    Ok(())
}

/// Возобновляет запись после паузы (открывает следующий сегмент).
#[tauri::command]
pub fn resume_recording(app: AppHandle, state: tauri::State<AppState>) -> AppResult<()> {
    let current = {
        state
            .active
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| AppError::InvalidState("not recording".into()))?
    };
    let updated = service::resume_recording(state.recorder.as_ref(), current)?;
    *state.active.lock().unwrap() = Some(updated);
    notify(&app, "🔴 Memiro — запись продолжена", "Идёт запись");
    Ok(())
}

/// Останавливает активную запись и возвращает встречу (или `None`, если записи
/// не было). Без уведомлений — для использования фоновым монитором авто-записи.
pub(crate) fn stop_active_recording(state: &AppState) -> AppResult<Option<Meeting>> {
    let current = { state.active.lock().unwrap().take() };
    let current = match current {
        Some(c) => c,
        None => return Ok(None),
    };
    let created_at = chrono::Utc::now().to_rfc3339();
    let repo = state.repo.lock().unwrap();
    let meeting = service::stop_recording(state.recorder.as_ref(), &repo, &current, created_at)?;
    Ok(Some(meeting))
}

/// Удаляет встречу (БД + папка) — для отбрасывания коротких авто-записей.
pub(crate) fn discard_meeting(state: &AppState, id: &str) -> AppResult<()> {
    let repo = state.repo.lock().unwrap();
    service::delete_meeting(&repo, &state.data_root, id)
}

/// Импортирует внешнюю аудиозапись (m4a/mp3/wav/flac/ogg…) как новую встречу.
/// Тяжёлый декод уходит в фон (`spawn_blocking`), поэтому окно не зависает.
#[tauri::command]
pub async fn import_recording(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> AppResult<Meeting> {
    let data_root = state.data_root.clone();
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    // Декод + ресемпл в audio.wav — вне async-потока (без обращения к БД).
    let meeting = tauri::async_runtime::spawn_blocking(move || {
        service::import_to_meeting(&data_root, id, &PathBuf::from(path), created_at)
    })
    .await
    .map_err(|e| AppError::Audio(format!("import join: {e}")))??;

    state.repo.lock().unwrap().insert(&meeting)?;
    notify(&app, "📥 Memiro — запись импортирована", &meeting.title);
    Ok(meeting)
}

#[tauri::command]
pub fn list_meetings(state: tauri::State<AppState>) -> AppResult<Vec<Meeting>> {
    state.repo.lock().unwrap().list()
}

#[tauri::command]
pub fn get_meeting(state: tauri::State<AppState>, id: String) -> AppResult<Meeting> {
    state.repo.lock().unwrap().get(&id)
}

#[tauri::command]
pub fn delete_meeting(state: tauri::State<AppState>, id: String) -> AppResult<()> {
    let repo = state.repo.lock().unwrap();
    service::delete_meeting(&repo, &state.data_root, &id)
}

/// Абсолютный путь к дорожке — фронтенд превратит его в asset-URL.
#[tauri::command]
pub fn track_path(
    state: tauri::State<AppState>,
    id: String,
    track_file: String,
) -> AppResult<String> {
    let p = service::track_path(&state.data_root, &id, &track_file)?;
    Ok(p.to_string_lossy().to_string())
}

#[tauri::command]
pub fn is_recording(state: tauri::State<AppState>) -> bool {
    state.active.lock().unwrap().is_some()
}

/// Переключает запись (для горячей клавиши и трея). Возвращает `true`, если
/// запись только что началась, `false` — если остановлена.
pub fn toggle_recording_state(state: &AppState) -> AppResult<bool> {
    let was_recording = state.active.lock().unwrap().is_some();
    if was_recording {
        let current = state.active.lock().unwrap().take();
        if let Some(current) = current {
            let created_at = chrono::Utc::now().to_rfc3339();
            let repo = state.repo.lock().unwrap();
            service::stop_recording(state.recorder.as_ref(), &repo, &current, created_at)?;
        }
        Ok(false)
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        let rec = service::start_recording(state.recorder.as_ref(), &state.data_root, id)?;
        *state.active.lock().unwrap() = Some(rec);
        Ok(true)
    }
}

// Асинхронная: тяжёлая работа (загрузка модели + whisper) уходит с главного
// потока, поэтому окно не зависает и можно ходить по другим встречам.
// Прогресс шлём событием `transcribe-progress`.
#[tauri::command]
pub async fn transcribe(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    options: TranscribeOptions,
    // Сколько голосов: для импорта — всего говорящих, для записи — число
    // собеседников на системной дорожке. None/0 — определить автоматически.
    speaker_count: Option<u32>,
    // Соло-режим «я один» (заметка на один голос): расшифровываем только
    // микрофон, всем сегментам говорящий «Я», без дорожки собеседника и
    // диаризации. Игнорируется для импортированных встреч.
    solo: Option<bool>,
    // Число — всех голосов записи (живая встреча, известная по прошлой
    // расшифровке), а не собеседников.
    total_voices: Option<bool>,
) -> AppResult<Transcript> {
    let _ = &total_voices; // нужен только с фичей diarize
    // Импортированная встреча — одна дорожка audio.wav; записанная — mic+system.
    let imported = state.repo.lock().unwrap().get(&id)?.source == "imported";
    let solo = !imported && solo.unwrap_or(false);

    // Явно указанный внешний whisper-CLI — используем его (без прогресса).
    if options
        .whisper_path
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false)
    {
        let transcriber = CliTranscriber::new(options);
        let transcript = if imported {
            service::transcribe_single_to_file(&transcriber, &state.data_root, &id)?
        } else if solo {
            service::transcribe_solo_to_file(&transcriber, &state.data_root, &id)?
        } else {
            service::transcribe_to_file(&transcriber, &state.data_root, &id)?
        };
        state.repo.lock().unwrap().update_status(&id, "transcribed")?;
        notify(&app, "📝 Memiro — расшифровка готова", "Текст разговора готов");
        return Ok(transcript);
    }

    // Иначе — встроенный whisper.cpp; модель скачивается ОДИН раз (фаза
    // download только при реальной загрузке). Прогресс шлём ПО ФАЗАМ из
    // безопасного потока (без FFI-колбэка внутри whisper.cpp — он ронял).
    #[cfg(any(feature = "whisper", feature = "parakeet"))]
    {
        use uxo_core::transcript::merge_tracks;

        // Сколько голосов задал пользователь: None/0 — авто.
        let wanted = speaker_count.filter(|&n| n > 0).map(|n| n as usize);
        flog(
            &state.data_root,
            &format!(
                "transcribe start id={id} imported={imported} solo={solo} speakers={}",
                wanted.map(|n| n.to_string()).unwrap_or_else(|| "auto".into())
            ),
        );

        // Прогресс из НАШЕГО цикла: фаза + процент + счётчик окон (done/total).
        let emit = |stage: &str, percent: f32, done: usize, total: usize| {
            let _ = app.emit(
                "transcribe-progress",
                TranscribeProgress {
                    id: id.clone(),
                    stage: stage.into(),
                    percent: percent.clamp(0.0, 100.0),
                    done: done as u32,
                    total: total as u32,
                },
            );
        };

        emit("loading", 0.0, 0, 0);
        let model =
            uxo_core::models::pick_model(options.model.as_deref(), options.language.as_deref())
                .to_string();
        flog(&state.data_root, &format!("transcribe: model {model}"));
        let vocab = uxo_core::vocab::Vocabulary::new(options.vocabulary.as_deref().unwrap_or(""), true);
        let transcriber = load_asr(
            &state.data_root,
            &model,
            options.language.clone(),
            &vocab,
            &|frac| emit("download", frac * 100.0, 0, 0),
        )?;

        let transcript = if imported {
            // Импорт — одна дорожка audio.wav: текст 0..75%, голоса 75..100%.
            let audio_path = service::track_path(&state.data_root, &id, "audio.wav")?;
            #[cfg(feature = "diarize")]
            let text_scale = 75.0f32;
            #[cfg(not(feature = "diarize"))]
            let text_scale = 100.0f32;

            emit("mic", 0.0, 0, 0);
            let segs = transcriber.run_staged(
                &audio_path,
                &|done, total| emit("mic", (done as f32 / total as f32) * text_scale * 0.8, done, total),
                &|done, total| emit("refine", text_scale * (0.8 + 0.2 * done as f32 / total.max(1) as f32), done, total),
            )?;
            flog(&state.data_root, &format!("transcribed: audio={} segs", segs.len()));

            #[cfg(feature = "diarize")]
            {
                use uxo_core::transcript::assign_speakers;
                if wanted == Some(1) {
                    uxo_core::transcript::single_speaker(segs, "spk0")
                } else {
                    let diar = diarize_track(
                        &state.data_root,
                        &id,
                        "audio.wav",
                        &audio_path,
                        wanted,
                        &|frac| emit("download-voices", frac * 100.0, 0, 0),
                        &|done, total| {
                            emit("diarize", 75.0 + done as f32 / total.max(1) as f32 * 25.0, done, total)
                        },
                    )?;
                    assign_speakers(segs, diar)
                }
            }
            #[cfg(not(feature = "diarize"))]
            {
                uxo_core::transcript::single_speaker(segs, "spk0")
            }
        } else if solo {
            // Соло-режим «я один»: только микрофон, всем сегментам «Я».
            let mic_path = normalize_track(&state.data_root, &id, "mic.wav")?;
            emit("mic", 0.0, 0, 0);
            flog(&state.data_root, "transcribe: solo mic track");
            let mic_segs = transcriber.run_staged(
                &mic_path,
                &|done, total| emit("mic", (done as f32 / total as f32) * 80.0, done, total),
                &|done, total| emit("refine", 80.0 + 20.0 * done as f32 / total.max(1) as f32, done, total),
            )?;
            flog(
                &state.data_root,
                &format!("transcribed solo: mic={} segs", mic_segs.len()),
            );
            uxo_core::transcript::single_speaker(mic_segs, uxo_core::transcript::ME)
        } else {
            // Записанные дорожки нормализуем тем же декодером, что и импорт
            // (сырой WASAPI-WAV whisper не всегда расшифровывал).
            let mic_path = normalize_track(&state.data_root, &id, "mic.wav")?;
            let system_path = normalize_track(&state.data_root, &id, "system.wav")?;

            // Собеседников может быть несколько (групповой звонок): системную
            // дорожку делим по голосам — автоматически, если число не задано.
            // «1 собеседник» — без разделения.
            #[cfg(feature = "diarize")]
            let will_diarize = wanted != Some(1);
            #[cfg(not(feature = "diarize"))]
            let will_diarize = false;
            let sys_end = if will_diarize { 80.0 } else { 100.0 };

            emit("mic", 0.0, 0, 0);
            flog(&state.data_root, "transcribe: mic track");
            let mic_segs = transcriber.run_staged(
                &mic_path,
                &|done, total| emit("mic", (done as f32 / total as f32) * 36.0, done, total),
                &|done, total| emit("refine", 36.0 + 9.0 * done as f32 / total.max(1) as f32, done, total),
            )?;

            emit("system", 45.0, 0, 0);
            flog(&state.data_root, "transcribe: system track");
            let sys_span = sys_end - 45.0;
            let system_segs = transcriber.run_staged(
                &system_path,
                &|done, total| emit("system", 45.0 + (done as f32 / total as f32) * sys_span * 0.8, done, total),
                &|done, total| {
                    emit("refine", 45.0 + sys_span * (0.8 + 0.2 * done as f32 / total.max(1) as f32), done, total)
                },
            )?;

            flog(
                &state.data_root,
                &format!(
                    "transcribed: mic={} segs, system={} segs",
                    mic_segs.len(),
                    system_segs.len()
                ),
            );

            #[cfg(feature = "diarize")]
            {
                use uxo_core::transcript::{
                    assign_speakers, collapse_single_speaker, is_in_person, merge_transcripts, single_speaker,
                    ME, THEM,
                };
                if is_in_person(&mic_segs, &system_segs) {
                    // Живая встреча: все голоса — в микрофоне; делим его.
                    // Число в интерфейсе — собеседники, значит голосов N + 1.
                    flog(&state.data_root, "in-person meeting: diarizing mic track");
                    let diar = diarize_track(
                        &state.data_root,
                        &id,
                        "mic.wav",
                        &mic_path,
                        if total_voices.unwrap_or(false) { wanted } else { wanted.map(|n| n + 1) },
                        &|frac| emit("download-voices", frac * 100.0, 0, 0),
                        &|done, total| {
                            emit("diarize", sys_end + done as f32 / total.max(1) as f32 * (100.0 - sys_end), done, total)
                        },
                    )?;
                    let room = collapse_single_speaker(assign_speakers(mic_segs, diar), ME);
                    merge_transcripts(room, single_speaker(system_segs, THEM))
                } else if will_diarize && !system_segs.is_empty() {
                    let diar = diarize_track(
                        &state.data_root,
                        &id,
                        "system.wav",
                        &system_path,
                        wanted,
                        &|frac| emit("download-voices", frac * 100.0, 0, 0),
                        &|done, total| {
                            emit("diarize", sys_end + done as f32 / total.max(1) as f32 * (100.0 - sys_end), done, total)
                        },
                    )?;
                    // Один голос у собеседников → привычный «Собеседник».
                    let them = collapse_single_speaker(assign_speakers(system_segs, diar), THEM);
                    let me = single_speaker(mic_segs, ME);
                    merge_transcripts(me, them)
                } else {
                    merge_tracks(mic_segs, system_segs)
                }
            }
            #[cfg(not(feature = "diarize"))]
            {
                merge_tracks(mic_segs, system_segs)
            }
        };

        service::save_transcript(&state.data_root, &id, &transcript)?;
        state.repo.lock().unwrap().update_status(&id, "transcribed")?;
        flog(&state.data_root, "transcribe done");
        notify(&app, "📝 Memiro — расшифровка готова", "Текст разговора готов");
        Ok(transcript)
    }
    #[cfg(not(any(feature = "whisper", feature = "parakeet")))]
    {
        let _ = (options, &app, speaker_count);
        Err(AppError::Audio(
            "встроенный Whisper недоступен в этой сборке; укажите путь к whisper в настройках".into(),
        ))
    }
}

/// Движок распознавания: Whisper (whisper.cpp) или Parakeet (ONNX).
#[cfg(any(feature = "whisper", feature = "parakeet"))]
trait Asr {
    /// Расшифровка файла по окнам; прогресс — (готово окон, всего).
    fn run(
        &self,
        wav: &Path,
        progress: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<uxo_core::transcript::Segment>>;

    /// То же, плюс прогресс второго прохода (уточнение трудных мест):
    /// `refine(готово, всего)`.
    fn run_staged(
        &self,
        wav: &Path,
        progress: &dyn Fn(usize, usize),
        refine: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<uxo_core::transcript::Segment>> {
        let _ = refine;
        self.run(wav, progress)
    }
}

#[cfg(feature = "whisper")]
impl Asr for uxo_core::whisper::WhisperTranscriber {
    fn run(
        &self,
        wav: &Path,
        progress: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<uxo_core::transcript::Segment>> {
        self.transcribe_windowed(wav, uxo_core::whisper::DEFAULT_WINDOW_SECS, progress)
    }
}

#[cfg(feature = "parakeet")]
impl Asr for uxo_core::parakeet::ParakeetTranscriber {
    fn run(
        &self,
        wav: &Path,
        progress: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<uxo_core::transcript::Segment>> {
        self.transcribe_windowed(wav, 15, progress)
    }
}

/// Parakeet + второй проход + контроль языка (всё локально, без вопросов
/// пользователю):
/// 1. Трудные окна (шум, перебивания: низкая уверенность или «речь есть, а
///    слов нет») распознаются заново — Parakeet по очищенному от шума звуку,
///    а если и так плохо — Whisper; остаётся лучший вариант (см. `rescue`).
/// 2. Реплики «не на том языке» (латиница вместо русского) перепроверяет
///    Whisper: язык из настроек — приоритетный, но английская фраза-термин,
///    если она действительно звучит, остаётся английской.
/// Whisper грузится, только если он понадобился.
#[cfg(all(feature = "parakeet", feature = "whisper"))]
struct QualityGuard {
    primary: uxo_core::parakeet::ParakeetTranscriber,
    language: Option<String>,
    prompt: Option<String>,
    data_root: PathBuf,
    whisper: std::cell::OnceCell<Option<uxo_core::whisper::WhisperTranscriber>>,
    /// Лимит Whisper на текущую дорожку: (вызовов осталось, крайний срок).
    budget: std::cell::Cell<(usize, std::time::Instant)>,
}

#[cfg(all(feature = "parakeet", feature = "whisper"))]
impl QualityGuard {
    /// Можно ли ещё звать Whisper на этой дорожке (и списать вызов).
    fn take_whisper(&self) -> bool {
        let (left, deadline) = self.budget.get();
        if left == 0 || std::time::Instant::now() > deadline {
            return false;
        }
        self.budget.set((left - 1, deadline));
        true
    }

    fn lang(&self) -> Option<&str> {
        self.language.as_deref().map(str::trim).filter(|l| !l.is_empty())
    }

    fn whisper(&self) -> Option<&uxo_core::whisper::WhisperTranscriber> {
        self.whisper
            .get_or_init(|| {
                match uxo_core::whisper::WhisperTranscriber::managed(&self.data_root, None, Some("auto".into()), &|_| {}) {
                    Ok(w) => Some(w.with_preferred_language(self.lang().map(str::to_string)).with_prompt(self.prompt.clone())),
                    Err(e) => {
                        flog(&self.data_root, &format!("quality: whisper unavailable: {e}"));
                        None
                    }
                }
            })
            .as_ref()
    }

    /// Второй проход по трудным окнам.
    fn rescue(
        &self,
        audio: &[f32],
        segs: Vec<uxo_core::transcript::Segment>,
        scores: &[uxo_core::parakeet::WindowScore],
        refine: &dyn Fn(usize, usize),
    ) -> Vec<uxo_core::transcript::Segment> {
        use uxo_core::rescue::{self as rs, Candidate};
        use uxo_core::transcript::Segment;
        const SR: f64 = 16_000.0;
        let shift = |v: Vec<Segment>, a: f64, b: f64| -> Vec<Segment> {
            v.into_iter()
                .map(|mut s| {
                    s.start_secs = (s.start_secs + a).min(b);
                    s.end_secs = (s.end_secs + a).min(b);
                    s
                })
                .collect()
        };
        // Трудные окна — от худших к лучшим (лимит Whisper — худшим).
        let mut hard: Vec<(rs::WindowStats, usize, usize)> = scores
            .iter()
            .filter_map(|w| {
                let (ia, ib) = ((w.start_secs * SR) as usize, ((w.end_secs * SR) as usize).min(audio.len()));
                if ib <= ia {
                    return None;
                }
                let window = &audio[ia..ib];
                let stats = rs::WindowStats {
                    start_secs: w.start_secs,
                    end_secs: w.end_secs,
                    snr_db: uxo_core::enhance::snr_db(window),
                    speech_secs: rs::speech_secs(window),
                    confidence: w.confidence,
                    tokens: w.tokens,
                };
                rs::is_hard(&stats).then_some((stats, ia, ib))
            })
            .collect();
        hard.sort_by(|a, b| rs::severity(&b.0).partial_cmp(&rs::severity(&a.0)).unwrap_or(std::cmp::Ordering::Equal));
        let total = hard.len();
        let (mut spans, mut fixed) = (Vec::new(), Vec::new());
        let (mut by_denoise, mut by_whisper, mut skipped) = (0usize, 0usize, 0usize);
        for (k, (stats, ia, ib)) in hard.iter().enumerate() {
            refine(k, total);
            let window = &audio[*ia..*ib];
            let mid = |s: &Segment| (s.start_secs + s.end_secs) / 2.0;
            let orig_words = rs::word_count(
                segs.iter().filter(|s| mid(s) >= stats.start_secs && mid(s) < stats.end_secs).map(|s| s.text.as_str()),
            );
            let orig = Candidate { words: orig_words, confidence: stats.confidence };
            let clean = uxo_core::enhance::denoise(window);
            let mut best = orig;
            let mut chosen: Option<(Vec<Segment>, bool)> = None;
            if let Ok((alt, conf)) = self.primary.transcribe_samples(&clean) {
                let cand = Candidate { words: rs::word_count(alt.iter().map(|s| s.text.as_str())), confidence: conf };
                if rs::better_same_engine(orig, cand) {
                    best = cand;
                    chosen = Some((alt, false));
                }
            }
            if rs::still_hard(best, rs::speech_secs(&clean).max(stats.speech_secs)) {
                if !self.take_whisper() {
                    skipped += 1;
                } else if let Some(wh) = self.whisper() {
                    if let Ok((alt, prob)) = wh.transcribe_samples(&clean) {
                        let cand = Candidate { words: rs::word_count(alt.iter().map(|s| s.text.as_str())), confidence: prob };
                        if rs::accept_whisper(best, cand) {
                            chosen = Some((alt, true));
                        }
                    }
                }
            }
            if let Some((alt, whisper)) = chosen {
                if whisper {
                    by_whisper += 1;
                } else {
                    by_denoise += 1;
                }
                spans.push((stats.start_secs, stats.end_secs));
                fixed.extend(shift(alt, stats.start_secs, stats.end_secs));
            }
        }
        refine(total, total);
        if total > 0 {
            flog(
                &self.data_root,
                &format!(
                    "second pass: {total} hard window(s), improved {} (denoise {by_denoise}, whisper {by_whisper}), whisper limit skipped {skipped}",
                    spans.len()
                ),
            );
        }
        if spans.is_empty() {
            return segs;
        }
        // Окна не пересекаются: правая граница — исключая (как у окон).
        let spans: Vec<(f64, f64)> = spans.into_iter().map(|(a, b)| (a, b - 1e-6)).collect();
        uxo_core::langguard::replace_spans(segs, &spans, fixed)
    }

    /// Реплики не той письменности → Whisper с приоритетным языком.
    fn language_fix(&self, wav: &Path, segs: Vec<uxo_core::transcript::Segment>) -> AppResult<Vec<uxo_core::transcript::Segment>> {
        use uxo_core::langguard as lg;
        let lang = self.lang();
        let Some(target) = lg::target_script(lang, &segs) else { return Ok(segs) };
        let duration = lg::wav_duration(wav).unwrap_or(0.0);
        let spans = lg::mismatch_spans(&segs, target, 0.4, duration);
        if spans.is_empty() {
            return Ok(segs);
        }
        let secs: f64 = spans.iter().map(|(a, b)| b - a).sum();
        flog(
            &self.data_root,
            &format!("language guard: {} span(s), {secs:.1}s not {target:?} → whisper ({})", spans.len(), lang.unwrap_or("auto")),
        );
        let Some(whisper) = self.whisper() else { return Ok(segs) };
        let mut fixed = Vec::new();
        for (i, &(a, b)) in spans.iter().enumerate() {
            if !self.take_whisper() {
                flog(&self.data_root, &format!("language guard: whisper limit, {} span(s) left as is", spans.len() - i));
                break;
            }
            let part = wav.with_extension(format!("span{i}.wav"));
            lg::cut_wav(wav, &part, a, b)?;
            let redo = whisper.run(&part, &|_, _| {});
            let _ = std::fs::remove_file(&part);
            match redo {
                Ok(redo) => fixed.extend(redo.into_iter().map(|mut s| {
                    s.start_secs += a;
                    s.end_secs = (s.end_secs + a).min(b);
                    s
                })),
                Err(e) => {
                    flog(&self.data_root, &format!("language guard: span {i} failed: {e}"));
                    // Оставляем исходные реплики этого участка.
                    return Ok(segs);
                }
            }
        }
        Ok(lg::replace_spans(segs, &spans, fixed))
    }
}

#[cfg(all(feature = "parakeet", feature = "whisper"))]
impl Asr for QualityGuard {
    fn run(
        &self,
        wav: &Path,
        progress: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<uxo_core::transcript::Segment>> {
        self.run_staged(wav, progress, &|_, _| {})
    }

    fn run_staged(
        &self,
        wav: &Path,
        progress: &dyn Fn(usize, usize),
        refine: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<uxo_core::transcript::Segment>> {
        let (segs, scores) = self.primary.transcribe_scored(wav, progress)?;
        let audio = uxo_core::enhance::read_wav_f32(wav)?;
        let (calls, time) = uxo_core::rescue::whisper_budget(scores.len(), audio.len() as f64 / 16_000.0);
        self.budget.set((calls, std::time::Instant::now() + time));
        let segs = self.rescue(&audio, segs, &scores, refine);
        self.language_fix(wav, segs)
    }
}

/// Загружает выбранный движок (модель качается только в первый раз).
#[cfg(any(feature = "whisper", feature = "parakeet"))]
fn load_asr(
    data_root: &Path,
    model: &str,
    language: Option<String>,
    vocab: &uxo_core::vocab::Vocabulary,
    on_download: &dyn Fn(f32),
) -> AppResult<Box<dyn Asr>> {
    let inner = load_engine(data_root, model, language, vocab, on_download)?;
    Ok(Box::new(VocabFix { inner, vocab: vocab.clone() }))
}

/// Исправляет написание терминов словаря в результате любого движка.
#[cfg(any(feature = "whisper", feature = "parakeet"))]
struct VocabFix {
    inner: Box<dyn Asr>,
    vocab: uxo_core::vocab::Vocabulary,
}

#[cfg(any(feature = "whisper", feature = "parakeet"))]
impl Asr for VocabFix {
    fn run(
        &self,
        wav: &Path,
        progress: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<uxo_core::transcript::Segment>> {
        self.run_staged(wav, progress, &|_, _| {})
    }

    fn run_staged(
        &self,
        wav: &Path,
        progress: &dyn Fn(usize, usize),
        refine: &dyn Fn(usize, usize),
    ) -> AppResult<Vec<uxo_core::transcript::Segment>> {
        let mut segs = self.inner.run_staged(wav, progress, refine)?;
        for s in &mut segs {
            s.text = self.vocab.correct(&s.text);
        }
        Ok(segs)
    }
}

#[cfg(any(feature = "whisper", feature = "parakeet"))]
fn load_engine(
    data_root: &Path,
    model: &str,
    language: Option<String>,
    vocab: &uxo_core::vocab::Vocabulary,
    on_download: &dyn Fn(f32),
) -> AppResult<Box<dyn Asr>> {
    if uxo_core::models::is_parakeet(model) {
        #[cfg(feature = "parakeet")]
        {
            let parakeet = uxo_core::parakeet::ParakeetTranscriber::managed(data_root, on_download)?;
            // Трудные окна и реплики «не на том языке» — второй проход (QualityGuard).
            #[cfg(feature = "whisper")]
            return Ok(Box::new(QualityGuard {
                primary: parakeet,
                language,
                prompt: vocab.prompt(),
                data_root: data_root.to_path_buf(),
                whisper: std::cell::OnceCell::new(),
                budget: std::cell::Cell::new((0, std::time::Instant::now())),
            }));
            #[cfg(not(feature = "whisper"))]
            {
                let _ = (language, vocab);
                return Ok(Box::new(parakeet));
            }
        }
        #[cfg(not(feature = "parakeet"))]
        return Err(AppError::InvalidState(
            "Parakeet недоступен в этой сборке — выберите Whisper в настройках".into(),
        ));
    }
    #[cfg(feature = "whisper")]
    {
        Ok(Box::new(
            uxo_core::whisper::WhisperTranscriber::managed(data_root, Some(model), language, on_download)?
                .with_prompt(vocab.prompt()),
        ))
    }
    #[cfg(not(feature = "whisper"))]
    {
        let _ = (data_root, language, vocab, on_download);
        Err(AppError::InvalidState(
            "Whisper недоступен в этой сборке — выберите Parakeet в настройках".into(),
        ))
    }
}

/// Прогоняет записанную дорожку через декодер импорта (16 кГц/моно/i16) в
/// `<stem>_norm.wav`. Best-effort: при ошибке (пустая дорожка) — исходный файл.
#[cfg(any(feature = "whisper", feature = "parakeet"))]
fn normalize_track(data_root: &Path, id: &str, track: &str) -> AppResult<PathBuf> {
    let src = service::track_path(data_root, id, track)?;
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("track").to_string();
    let dst = src.with_file_name(format!("{stem}_norm.wav"));
    match uxo_core::decode::decode_to_wav_16k_mono(&src, &dst) {
        Ok(()) => Ok(dst),
        Err(e) => {
            flog(data_root, &format!("normalize {stem} failed: {e}"));
            Ok(src)
        }
    }
}

/// Анализ голосов дорожки (модели качаются один раз) → разметка говорящих.
/// Эмбеддинги кешируются в `diarization.json`: потом число голосов можно
/// поменять мгновенно (`recluster_speakers`), без повторного анализа.
#[cfg(feature = "diarize")]
#[allow(clippy::too_many_arguments)]
fn diarize_track(
    data_root: &Path,
    id: &str,
    track: &str,
    wav: &Path,
    wanted: Option<usize>,
    on_download: &dyn Fn(f32),
    on_progress: &dyn Fn(usize, usize),
) -> AppResult<Vec<uxo_core::transcript::DiarSegment>> {
    use uxo_core::diarize::OnnxDiarizer;
    flog(data_root, &format!("diarize {track}: start"));
    let started = std::time::Instant::now();
    let diarizer = OnnxDiarizer::managed(data_root, wanted, on_download)?;
    on_progress(0, 1);
    let windows = diarizer.embed(wav, on_progress)?;
    let labels = uxo_core::cluster::cluster_windows(&windows, wanted);
    flog(
        data_root,
        &format!(
            "diarize {track}: {} fragments, {} voices, {:.1}s",
            windows.len(),
            uxo_core::cluster::count_speakers(&labels),
            started.elapsed().as_secs_f32()
        ),
    );
    let diar = uxo_core::cluster::windows_to_diar(&windows, &labels);
    let cache = service::DiarCache { track: track.to_string(), windows };
    if let Err(e) = service::save_diar_cache(data_root, id, &cache) {
        flog(data_root, &format!("diarize: cache save failed: {e}"));
    }
    Ok(diar)
}

/// Меняет число голосов в готовой расшифровке — мгновенно, по сохранённому
/// анализу голосов (без повторной расшифровки). `speaker_count`: None/0 — авто.
#[tauri::command]
pub async fn recluster_speakers(
    state: tauri::State<'_, AppState>,
    id: String,
    speaker_count: Option<u32>,
) -> AppResult<Transcript> {
    let data_root = state.data_root.clone();
    let wanted = speaker_count.filter(|&n| n > 0).map(|n| n as usize);
    let t = tauri::async_runtime::spawn_blocking(move || {
        service::recluster_transcript(&data_root, &id, wanted)
    })
    .await
    .map_err(|e| AppError::Audio(format!("recluster join: {e}")))??;
    Ok(t)
}

/// Какая дорожка разделена по голосам: `system.wav` (звонок), `mic.wav`
/// (живая встреча), `audio.wav` (импорт); `None` — анализа нет.
#[tauri::command]
pub fn voice_analysis_track(state: tauri::State<AppState>, id: String) -> AppResult<Option<String>> {
    Ok(service::load_diar_cache(&state.data_root, &id)?.map(|c| c.track))
}

/// Есть ли у встречи сохранённый анализ голосов (можно менять число голосов).
#[tauri::command]
pub fn has_voice_analysis(state: tauri::State<AppState>, id: String) -> AppResult<bool> {
    Ok(service::load_diar_cache(&state.data_root, &id)?.is_some())
}

// ── Модели ──────────────────────────────────────────────────────────────────

/// Событие прогресса загрузки модели из настроек.
#[derive(Clone, serde::Serialize)]
pub struct ModelProgress {
    pub id: String,
    pub percent: f32,
}

/// Статус локальных моделей (скачаны ли, сколько весят).
#[tauri::command]
pub fn models_status(state: tauri::State<AppState>) -> Vec<uxo_core::models::ModelInfo> {
    uxo_core::models::status(&state.data_root)
}

/// Скачивает модель заранее (из настроек): `id` — модель Whisper или "voices"
/// (разделение голосов). Прогресс — событием `model-download-progress`.
#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> AppResult<()> {
    let data_root = state.data_root.clone();
    let app2 = app.clone();
    let mid = id.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let report = |f: f32| {
            let _ = app2.emit(
                "model-download-progress",
                ModelProgress { id: mid.clone(), percent: (f * 100.0).clamp(0.0, 100.0) },
            );
        };
        if mid == "voices" {
            #[cfg(feature = "diarize")]
            {
                uxo_core::diarize::ensure_models(&data_root, &report)?;
                Ok(())
            }
            #[cfg(not(feature = "diarize"))]
            {
                let _ = &report;
                Err(AppError::InvalidState("разделение голосов недоступно в этой сборке".into()))
            }
        } else if uxo_core::models::is_parakeet(&mid) {
            #[cfg(feature = "parakeet")]
            {
                uxo_core::parakeet::ensure_model(&data_root, &report)
            }
            #[cfg(not(feature = "parakeet"))]
            {
                Err(AppError::InvalidState("Parakeet недоступен в этой сборке".into()))
            }
        } else {
            uxo_core::models::ensure_whisper(&data_root, &mid, &report).map(|_| ())
        }
    })
    .await
    .map_err(|e| AppError::Audio(format!("download join: {e}")))??;
    flog(&state.data_root, &format!("model downloaded: {id}"));
    Ok(())
}

/// Удаляет скачанную модель Whisper (освободить место на диске).
#[tauri::command]
pub fn delete_model(state: tauri::State<AppState>, id: String) -> AppResult<()> {
    uxo_core::models::delete_whisper(&state.data_root, &id)
}

#[tauri::command]
pub fn save_text_file(path: String, content: String) -> AppResult<()> {
    std::fs::write(&path, content)?;
    Ok(())
}

/// Копирует WAV-дорожку встречи в выбранный путь (скачивание аудио).
#[tauri::command]
pub fn export_audio(
    state: tauri::State<AppState>,
    id: String,
    track_file: String,
    dest: String,
) -> AppResult<()> {
    let src = service::track_path(&state.data_root, &id, &track_file)?;
    std::fs::copy(&src, &dest)?;
    Ok(())
}

// ── Аудио-редактор (вырезание фрагментов) ───────────────────────────────────

/// Карта громкости дорожки для таймлайна редактора: `buckets` корзин
/// (пик + RMS, 0..1000). Чтение файла уходит в фон — двухчасовая запись это
/// сотни мегабайт, окно не должно подвисать.
#[tauri::command]
pub async fn waveform(
    state: tauri::State<'_, AppState>,
    id: String,
    track_file: String,
    buckets: u32,
) -> AppResult<Waveform> {
    let path = service::track_path(&state.data_root, &id, &track_file)?;
    tauri::async_runtime::spawn_blocking(move || uxo_core::edit::waveform(&path, buckets as usize))
        .await
        .map_err(|e| AppError::Audio(format!("waveform join: {e}")))?
}

/// Какие дорожки есть у встречи и сохранён ли оригинал до правок.
#[tauri::command]
pub fn audio_edit_state(state: tauri::State<AppState>, id: String) -> AppResult<AudioEditState> {
    service::audio_edit_state(&state.data_root, &id)
}

/// Применяет правку аудио: вырезает интервалы `cuts` из всех дорожек встречи
/// (с однократным бэкапом оригинала), пересчитывает времена расшифровки и
/// обновляет длительность встречи. Возвращает обновлённую встречу.
#[tauri::command]
pub async fn apply_audio_edit(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
    cuts: Vec<Range>,
) -> AppResult<Meeting> {
    let data_root = state.data_root.clone();
    let cut_count = uxo_core::edit::merge_ranges(&cuts).len();
    let mid = id.clone();
    // Вырезание перезаписывает WAV-дорожки целиком — только вне async-потока.
    let secs = tauri::async_runtime::spawn_blocking(move || {
        service::apply_audio_edit_files(&data_root, &mid, &cuts)
    })
    .await
    .map_err(|e| AppError::Audio(format!("audio edit join: {e}")))??;

    let repo = state.repo.lock().unwrap();
    repo.update_duration(&id, secs)?;
    let meeting = repo.get(&id)?;
    drop(repo);
    flog(
        &state.data_root,
        &format!("audio edit applied: id={id} cuts={cut_count} duration={secs}s"),
    );
    notify(
        &app,
        "✂ Memiro — аудио обновлено",
        &format!("Вырезано фрагментов: {cut_count}"),
    );
    Ok(meeting)
}

/// Возвращает аудио встречи (и расшифровку) к оригиналу из бэкапа.
#[tauri::command]
pub async fn revert_audio_edit(
    state: tauri::State<'_, AppState>,
    id: String,
) -> AppResult<Meeting> {
    let data_root = state.data_root.clone();
    let mid = id.clone();
    let secs = tauri::async_runtime::spawn_blocking(move || {
        service::revert_audio_edit_files(&data_root, &mid)
    })
    .await
    .map_err(|e| AppError::Audio(format!("audio revert join: {e}")))??;

    let repo = state.repo.lock().unwrap();
    repo.update_duration(&id, secs)?;
    let meeting = repo.get(&id)?;
    drop(repo);
    flog(
        &state.data_root,
        &format!("audio edit reverted: id={id} duration={secs}s"),
    );
    Ok(meeting)
}

#[tauri::command]
pub fn get_transcript(state: tauri::State<AppState>, id: String) -> AppResult<Option<Transcript>> {
    service::load_transcript(&state.data_root, &id)
}

/// Сохраняет отредактированную пользователем расшифровку в `transcript.json`.
/// Статус встречи не меняется; ИИ-функции читают этот же файл, поэтому правки
/// подхватываются автоматически.
#[tauri::command]
pub fn save_transcript(
    state: tauri::State<AppState>,
    id: String,
    transcript: Transcript,
) -> AppResult<()> {
    service::save_transcript(&state.data_root, &id, &transcript)
}

/// Сохраняет отредактированный пользователем ИИ-отчёт. `kind` — какой именно:
/// "brief" | "summary" | "analysis" | "literary".
#[tauri::command]
pub fn save_report(
    state: tauri::State<AppState>,
    id: String,
    kind: String,
    content: String,
) -> AppResult<()> {
    service::save_report(&state.data_root, &id, &kind, &content)
}

/// Текст расшифровки для ИИ с именами говорящих, заданными пользователем.
fn named_transcript_text(
    data_root: &Path,
    id: &str,
    ctx: &uxo_core::ai::MeetingContext,
) -> AppResult<String> {
    let transcript = service::load_transcript(data_root, id)?.ok_or_else(|| {
        AppError::InvalidState("нет расшифровки — сначала расшифруйте встречу".into())
    })?;
    if transcript.segments.is_empty() {
        return Err(AppError::InvalidState("в расшифровке нет текста".into()));
    }
    let text = uxo_core::ai::transcript_to_named_text(&transcript, &ctx.names);
    Ok(if ctx.censor { uxo_core::profanity::censor(&text) } else { text })
}

/// Строит ИИ-отчёт вида `kind` ("summary" | "tasks" | "analysis" | "literary" |
/// "followup" | "agent") и сохраняет его. `context` — заголовок, участники и имена
/// говорящих (из интерфейса), чтобы отчёт говорил о людях по именам.
#[tauri::command]
pub async fn generate_report(
    state: tauri::State<'_, AppState>,
    id: String,
    kind: String,
    config: AiConfig,
    context: Option<uxo_core::ai::MeetingContext>,
) -> AppResult<String> {
    let ctx = context.unwrap_or_default();
    let text = named_transcript_text(&state.data_root, &id, &ctx)?;
    let data_root = state.data_root.clone();
    let (rid, rkind) = (id.clone(), kind.clone());
    let report = tauri::async_runtime::spawn_blocking(move || {
        let backend = HttpChatBackend::new(config);
        uxo_core::ai::generate_report(&backend, &rkind, &text, &ctx)
    })
    .await
    .map_err(|e| AppError::Http(format!("report join: {e}")))??;
    service::save_report(&data_root, &rid, &kind, &report)?;
    if kind == "summary" {
        state.repo.lock().unwrap().update_status(&id, "summarized")?;
    }
    Ok(report)
}

/// Все сохранённые ИИ-отчёты встречи: вид → текст.
#[tauri::command]
pub fn get_reports(
    state: tauri::State<AppState>,
    id: String,
) -> AppResult<std::collections::HashMap<String, String>> {
    Ok(service::load_reports(&state.data_root, &id)?.into_iter().collect())
}

/// Авто-заголовок/участники/тема с учётом имён говорящих.
#[tauri::command]
pub async fn suggest_meta(
    state: tauri::State<'_, AppState>,
    id: String,
    config: AiConfig,
    context: Option<uxo_core::ai::MeetingContext>,
) -> AppResult<MetadataSuggestion> {
    let ctx = context.unwrap_or_default();
    let text = named_transcript_text(&state.data_root, &id, &ctx)?;
    tauri::async_runtime::spawn_blocking(move || {
        let backend = HttpChatBackend::new(config);
        uxo_core::ai::suggest_metadata_ctx(&backend, &text)
    })
    .await
    .map_err(|e| AppError::Http(format!("meta join: {e}")))?
}

/// Вопрос по встрече с учётом имён говорящих.
#[tauri::command]
pub async fn ask_named(
    state: tauri::State<'_, AppState>,
    id: String,
    config: AiConfig,
    question: String,
    context: Option<uxo_core::ai::MeetingContext>,
) -> AppResult<String> {
    let ctx = context.unwrap_or_default();
    let text = named_transcript_text(&state.data_root, &id, &ctx)?;
    tauri::async_runtime::spawn_blocking(move || {
        let backend = HttpChatBackend::new(config);
        uxo_core::ai::answer_question(&backend, &text, &question)
    })
    .await
    .map_err(|e| AppError::Http(format!("ask join: {e}")))?
}

/// Сохраняет двоичный файл (экспорт DOCX). Данные — base64.
#[tauri::command]
pub fn save_binary_file(path: String, base64: String) -> AppResult<()> {
    let bytes = decode_base64(&base64)
        .ok_or_else(|| AppError::InvalidInput("bad base64".into()))?;
    std::fs::write(&path, bytes)?;
    Ok(())
}

/// Минимальный декодер base64 (стандартный алфавит, с `=`-дополнением).
fn decode_base64(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    }
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let trimmed: Vec<u8> = clean.iter().copied().take_while(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    for chunk in trimmed.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= val(c)? << (18 - 6 * i);
        }
        let n = match chunk.len() {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => return None,
        };
        for i in 0..n {
            out.push((acc >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode_base64;

    #[test]
    fn base64_roundtrip_known_values() {
        assert_eq!(decode_base64("TWFu").unwrap(), b"Man");
        assert_eq!(decode_base64("TWE=").unwrap(), b"Ma");
        assert_eq!(decode_base64("TQ==").unwrap(), b"M");
        assert_eq!(decode_base64("").unwrap(), b"");
        assert!(decode_base64("T").is_none());
        assert!(decode_base64("@@@@").is_none());
    }
}

#[tauri::command]
pub async fn suggest_metadata(
    state: tauri::State<'_, AppState>,
    id: String,
    config: AiConfig,
) -> AppResult<MetadataSuggestion> {
    let text = meeting_transcript_text(&state.data_root, &id)?;
    let backend = HttpChatBackend::new(config);
    uxo_core::ai::suggest_metadata(&backend, &text)
}

#[tauri::command]
pub async fn summarize(
    state: tauri::State<'_, AppState>,
    id: String,
    config: AiConfig,
) -> AppResult<String> {
    let text = meeting_transcript_text(&state.data_root, &id)?;
    let backend = HttpChatBackend::new(config);
    // Длинные разговоры (2–3 ч) не влезают в контекст — map-reduce по частям.
    let summary = uxo_core::ai::summarize_long(&backend, &text)?;
    service::save_summary(&state.data_root, &id, &summary)?;
    state.repo.lock().unwrap().update_status(&id, "summarized")?;
    Ok(summary)
}

#[tauri::command]
pub fn get_summary(state: tauri::State<AppState>, id: String) -> AppResult<Option<String>> {
    service::load_summary(&state.data_root, &id)
}

/// Литературный пересказ: ИИ переписывает расшифровку в связный текст, сохраняя
/// детали. Сохраняется в `literary.md`.
#[tauri::command]
pub async fn literary_text(
    state: tauri::State<'_, AppState>,
    id: String,
    config: AiConfig,
) -> AppResult<String> {
    let text = meeting_transcript_text(&state.data_root, &id)?;
    let backend = HttpChatBackend::new(config);
    // Длинные разговоры переписываем по частям и склеиваем (без сворачивания).
    let literary = uxo_core::ai::to_literary_long(&backend, &text)?;
    service::save_literary(&state.data_root, &id, &literary)?;
    Ok(literary)
}

#[tauri::command]
pub fn get_literary(state: tauri::State<AppState>, id: String) -> AppResult<Option<String>> {
    service::load_literary(&state.data_root, &id)
}

/// Краткое резюме (TL;DR). Сохраняется в `brief.md`.
#[tauri::command]
pub async fn brief_summary(
    state: tauri::State<'_, AppState>,
    id: String,
    config: AiConfig,
) -> AppResult<String> {
    let text = meeting_transcript_text(&state.data_root, &id)?;
    let backend = HttpChatBackend::new(config);
    let brief = uxo_core::ai::brief_summary_long(&backend, &text)?;
    service::save_brief(&state.data_root, &id, &brief)?;
    Ok(brief)
}

#[tauri::command]
pub fn get_brief(state: tauri::State<AppState>, id: String) -> AppResult<Option<String>> {
    service::load_brief(&state.data_root, &id)
}

/// ИИ-анализ разговора. Сохраняется в `analysis.md`.
#[tauri::command]
pub async fn analyze(
    state: tauri::State<'_, AppState>,
    id: String,
    config: AiConfig,
) -> AppResult<String> {
    let text = meeting_transcript_text(&state.data_root, &id)?;
    let backend = HttpChatBackend::new(config);
    let analysis = uxo_core::ai::analyze_long(&backend, &text)?;
    service::save_analysis(&state.data_root, &id, &analysis)?;
    Ok(analysis)
}

#[tauri::command]
pub fn get_analysis(state: tauri::State<AppState>, id: String) -> AppResult<Option<String>> {
    service::load_analysis(&state.data_root, &id)
}

#[tauri::command]
pub async fn ask(
    state: tauri::State<'_, AppState>,
    id: String,
    config: AiConfig,
    question: String,
) -> AppResult<String> {
    let text = meeting_transcript_text(&state.data_root, &id)?;
    let backend = HttpChatBackend::new(config);
    uxo_core::ai::answer_question(&backend, &text, &question)
}

/// Заметки пользователя к встрече (свободный текст).
#[tauri::command]
pub fn update_meeting_notes(
    state: tauri::State<AppState>,
    id: String,
    notes: String,
) -> AppResult<()> {
    state.repo.lock().unwrap().update_notes(&id, &notes)
}

/// Проверка ИИ-сервера: доступен ли, какие модели отдаёт и какую использовать
/// (модель на сервере могли обновить — тогда `changed` и новая `model`).
#[tauri::command]
pub async fn ai_check(config: AiConfig) -> AppResult<uxo_core::ai::AiCheck> {
    tauri::async_runtime::spawn_blocking(move || uxo_core::ai::check(&config))
        .await
        .map_err(|e| AppError::Http(format!("ai check join: {e}")))
}

#[tauri::command]
pub fn update_meeting_meta(
    state: tauri::State<AppState>,
    id: String,
    title: String,
    participants: String,
    topic: String,
) -> AppResult<()> {
    state
        .repo
        .lock()
        .unwrap()
        .update_meta(&id, &title, &participants, &topic)
}
