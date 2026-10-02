use crate::audio::{concat_wavs, wav_duration_secs};
use crate::edit::{self, Range};
use crate::error::{AppError, AppResult};
use crate::model::Meeting;
use crate::recorder::Recorder;
use crate::storage::Repo;
use crate::transcript::{merge_tracks, single_speaker, Transcript};
use crate::transcriber::Transcriber;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Активная запись: id, папка и сегменты дорожек.
///
/// Запись ведётся СЕГМЕНТАМИ: каждый непрерывный кусок пишется в
/// `mic.part{N}.wav` / `system.part{N}.wav`. Пауза финализирует текущий сегмент,
/// возобновление открывает следующий. На стопе все сегменты склеиваются в единый
/// `mic.wav`/`system.wav` (см. [`stop_recording`]). Такой подход не плодит мелкие
/// файлы и переживает аварийный обрыв: осиротевшие части склеит
/// [`recover_orphan_recordings`] при следующем запуске.
#[derive(Debug, Clone)]
pub struct ActiveRecording {
    pub id: String,
    pub dir: PathBuf,
    /// Уже финализированные сегменты: `(mic_part, system_part)`.
    pub segments: Vec<(PathBuf, PathBuf)>,
    /// Сегмент, который пишется прямо сейчас; `None` — запись на паузе.
    pub current: Option<(PathBuf, PathBuf)>,
}

/// Пути сегмента №`idx` в папке встречи.
fn segment_paths(dir: &Path, idx: usize) -> (PathBuf, PathBuf) {
    (
        dir.join(format!("mic.part{idx}.wav")),
        dir.join(format!("system.part{idx}.wav")),
    )
}

/// Собирает части одной дорожки в `dst` и удаляет части. Один сегмент —
/// мгновенное переименование (без перекодирования); несколько — склейка
/// [`concat_wavs`]. Пустой список — валидный WAV нулевой длины.
fn assemble_track(parts: &[PathBuf], dst: &Path) -> AppResult<()> {
    if parts.len() == 1 && std::fs::rename(&parts[0], dst).is_ok() {
        return Ok(());
    }
    concat_wavs(parts, dst)?;
    for p in parts {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

/// Отклоняет id, который мог бы вырваться из каталога встреч (path traversal).
fn validate_id(id: &str) -> AppResult<()> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(AppError::InvalidInput(format!("invalid meeting id: {id}")));
    }
    Ok(())
}

fn meeting_dir(data_root: &Path, id: &str) -> PathBuf {
    data_root.join("meetings").join(id)
}

/// Начинает запись: создаёт папку встречи и стартует первый сегмент.
pub fn start_recording(
    recorder: &dyn Recorder,
    data_root: &Path,
    id: String,
) -> AppResult<ActiveRecording> {
    validate_id(&id)?;
    let dir = meeting_dir(data_root, &id);
    std::fs::create_dir_all(&dir)?;
    let (mic_path, system_path) = segment_paths(&dir, 0);
    recorder.start(&mic_path, &system_path)?;
    Ok(ActiveRecording {
        id,
        dir,
        segments: Vec::new(),
        current: Some((mic_path, system_path)),
    })
}

/// Ставит запись на паузу: финализирует текущий сегмент. Если запись уже на
/// паузе — без изменений.
pub fn pause_recording(
    recorder: &dyn Recorder,
    mut active: ActiveRecording,
) -> AppResult<ActiveRecording> {
    if let Some(current) = active.current.take() {
        recorder.stop()?;
        active.segments.push(current);
    }
    Ok(active)
}

/// Возобновляет запись: открывает следующий сегмент. Если запись не на паузе —
/// без изменений.
pub fn resume_recording(
    recorder: &dyn Recorder,
    mut active: ActiveRecording,
) -> AppResult<ActiveRecording> {
    if active.current.is_none() {
        let (mic_path, system_path) = segment_paths(&active.dir, active.segments.len());
        recorder.start(&mic_path, &system_path)?;
        active.current = Some((mic_path, system_path));
    }
    Ok(active)
}

/// Останавливает запись, склеивает все сегменты в единые `mic.wav`/`system.wav`,
/// удаляет временные части, измеряет длительность и сохраняет встречу в БД.
pub fn stop_recording(
    recorder: &dyn Recorder,
    repo: &Repo,
    active: &ActiveRecording,
    created_at: String,
) -> AppResult<Meeting> {
    let mut segments = active.segments.clone();
    // Если запись шла (не на паузе) — финализируем текущий сегмент.
    if let Some(current) = active.current.clone() {
        recorder.stop()?;
        segments.push(current);
    }

    let mic_final = active.dir.join("mic.wav");
    let system_final = active.dir.join("system.wav");
    let mic_parts: Vec<PathBuf> = segments.iter().map(|(m, _)| m.clone()).collect();
    let system_parts: Vec<PathBuf> = segments.iter().map(|(_, s)| s.clone()).collect();
    assemble_track(&mic_parts, &mic_final)?;
    assemble_track(&system_parts, &system_final)?;

    let duration_secs = wav_duration_secs(&mic_final)
        .unwrap_or(0)
        .max(wav_duration_secs(&system_final).unwrap_or(0));
    let meeting = Meeting {
        id: active.id.clone(),
        created_at,
        title: "Новая встреча".into(),
        participants: String::new(),
        topic: String::new(),
        duration_secs,
        folder: active.id.clone(),
        status: "recorded".into(),
        source: "recorded".into(),
        notes: String::new(),
    };
    repo.insert(&meeting)?;
    Ok(meeting)
}

/// Восстанавливает «осиротевшие» записи, оставшиеся после аварийного завершения
/// во время записи: в папках встреч ищет несклеенные сегменты
/// (`mic.part{N}.wav`/`system.part{N}.wav`), собирает их в единые
/// `mic.wav`/`system.wav`, удаляет части и добавляет встречу в БД, если её там
/// ещё нет. Слишком короткие огрызки (<1 с, не дошедшие до БД) удаляются целиком,
/// чтобы не плодить мусор. Возвращает число восстановленных встреч.
///
/// Запускается при старте приложения, когда активной записи заведомо нет —
/// поэтому любые части на диске действительно осиротевшие.
pub fn recover_orphan_recordings(
    repo: &Repo,
    data_root: &Path,
    created_at: String,
) -> AppResult<usize> {
    let meetings_dir = data_root.join("meetings");
    if !meetings_dir.exists() {
        return Ok(0);
    }
    let mut recovered = 0usize;
    for entry in std::fs::read_dir(&meetings_dir)? {
        let entry = entry?;
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir = entry.path();
        let id = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let (mic_parts, system_parts) = collect_segment_parts(&dir);
        if mic_parts.is_empty() && system_parts.is_empty() {
            continue; // нет осиротевших частей — встреча завершилась штатно
        }

        let mic_final = dir.join("mic.wav");
        let system_final = dir.join("system.wav");
        assemble_track(&mic_parts, &mic_final)?;
        assemble_track(&system_parts, &system_final)?;

        let duration_secs = wav_duration_secs(&mic_final)
            .unwrap_or(0)
            .max(wav_duration_secs(&system_final).unwrap_or(0));
        let already_known = repo.get(&id).is_ok();
        if duration_secs == 0 && !already_known {
            // Пустой огрызок (краш сразу после старта) — выкидываем целиком.
            let _ = std::fs::remove_dir_all(&dir);
            continue;
        }
        if !already_known {
            let meeting = Meeting {
                id: id.clone(),
                created_at: created_at.clone(),
                title: "Восстановленная запись".into(),
                participants: String::new(),
                topic: String::new(),
                duration_secs,
                folder: id.clone(),
                status: "recorded".into(),
                source: "recorded".into(),
                notes: String::new(),
            };
            repo.insert(&meeting)?;
        }
        recovered += 1;
    }
    Ok(recovered)
}

/// Собирает части сегментов дорожек в папке встречи, отсортированные по номеру.
fn collect_segment_parts(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut mic: Vec<(usize, PathBuf)> = Vec::new();
    let mut system: Vec<(usize, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(idx) = parse_part_index(&name, "mic") {
                mic.push((idx, e.path()));
            } else if let Some(idx) = parse_part_index(&name, "system") {
                system.push((idx, e.path()));
            }
        }
    }
    mic.sort_by_key(|(i, _)| *i);
    system.sort_by_key(|(i, _)| *i);
    (
        mic.into_iter().map(|(_, p)| p).collect(),
        system.into_iter().map(|(_, p)| p).collect(),
    )
}

/// Номер сегмента из имени файла: `mic.part3.wav` + `"mic"` → `Some(3)`.
fn parse_part_index(name: &str, track: &str) -> Option<usize> {
    name.strip_prefix(track)?
        .strip_prefix(".part")?
        .strip_suffix(".wav")?
        .parse::<usize>()
        .ok()
}

/// Декодирует внешний аудиофайл (телефон/диктофон) в `audio.wav` (16 кГц
/// моно) и строит запись о встрече с `source = "imported"`, НЕ трогая БД —
/// тяжёлый декод можно гонять в фоне (`spawn_blocking`). Заголовок — из имени
/// исходного файла. Вставку в БД делает вызывающий (см. [`import_recording`]).
pub fn import_to_meeting(
    data_root: &Path,
    id: String,
    src: &Path,
    created_at: String,
) -> AppResult<Meeting> {
    validate_id(&id)?;
    let dir = meeting_dir(data_root, &id);
    std::fs::create_dir_all(&dir)?;
    let dst = dir.join("audio.wav");
    crate::decode::decode_to_wav_16k_mono(src, &dst)?;
    let duration_secs = wav_duration_secs(&dst)?;
    let title = src
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Импортированная запись")
        .to_string();
    Ok(Meeting {
        id: id.clone(),
        created_at,
        title,
        participants: String::new(),
        topic: String::new(),
        duration_secs,
        folder: id,
        status: "recorded".into(),
        source: "imported".into(),
        notes: String::new(),
    })
}

/// Импортирует внешнюю аудиозапись как новую встречу: декодирует и сохраняет
/// её в БД. Удобно для тестов; в GUI декод и вставка разнесены (см. команду).
pub fn import_recording(
    repo: &Repo,
    data_root: &Path,
    id: String,
    src: &Path,
    created_at: String,
) -> AppResult<Meeting> {
    let meeting = import_to_meeting(data_root, id, src, created_at)?;
    repo.insert(&meeting)?;
    Ok(meeting)
}

/// Удаляет встречу из БД и стирает её папку с диска.
pub fn delete_meeting(repo: &Repo, data_root: &Path, id: &str) -> AppResult<()> {
    validate_id(id)?;
    repo.delete(id)?;
    let dir = meeting_dir(data_root, id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

/// Дорожки, которые может содержать папка встречи: записанная — `mic` +
/// `system`, импортированная — `audio`. При правке аудио ВСЕ существующие
/// дорожки режутся одним набором вырезов, иначе «Я» и «Собеседник» разъедутся.
pub const TRACK_FILES: [&str; 3] = ["mic.wav", "system.wav", "audio.wav"];

/// Абсолютный путь к файлу дорожки встречи (для проигрывания во фронтенде).
pub fn track_path(data_root: &Path, id: &str, track_file: &str) -> AppResult<PathBuf> {
    validate_id(id)?;
    let allowed = TRACK_FILES;
    if !allowed.contains(&track_file) {
        return Err(AppError::InvalidInput(format!(
            "unknown track file: {track_file}"
        )));
    }
    let path = meeting_dir(data_root, id).join(track_file);
    if !path.exists() {
        return Err(AppError::NotFound(track_file.to_string()));
    }
    Ok(path)
}

// ── Правка аудио: вырезание фрагментов с сохранением оригинала ───────────────

/// Состояние аудио-редактора встречи для фронтенда.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioEditState {
    /// Существующие дорожки встречи (см. [`TRACK_FILES`]).
    pub tracks: Vec<String>,
    /// Сохранён ли оригинал до правок — значит, правку можно отменить.
    pub has_original: bool,
}

/// Имя бэкапа дорожки: `mic.wav` → `mic.orig.wav`.
fn orig_name(track_file: &str) -> String {
    format!("{}.orig.wav", track_file.trim_end_matches(".wav"))
}

/// Дорожки встречи, которые реально лежат на диске.
fn existing_tracks(dir: &Path) -> Vec<String> {
    TRACK_FILES
        .iter()
        .filter(|t| dir.join(t).exists())
        .map(|t| t.to_string())
        .collect()
}

/// Какие дорожки есть у встречи и есть ли бэкап оригинала.
pub fn audio_edit_state(data_root: &Path, id: &str) -> AppResult<AudioEditState> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    Ok(AudioEditState {
        tracks: existing_tracks(&dir),
        has_original: TRACK_FILES.iter().any(|t| dir.join(orig_name(t)).exists()),
    })
}

/// Однократно копирует дорожки (и расшифровку) в `*.orig.*` — снимок «как было
/// до правок», чтобы правку всегда можно было отменить. Существующий бэкап НЕ
/// перетирается: иначе он стал бы копией предыдущей правки, а не оригинала.
///
/// Если на момент первой правки расшифровки ещё не было, а позже она появилась,
/// бэкапом станет ближайшая к оригиналу версия расшифровки — после возврата
/// оригинала её стоит обновить («↻ Заново»).
fn backup_originals(dir: &Path, tracks: &[String]) -> AppResult<()> {
    for t in tracks {
        let orig = dir.join(orig_name(t));
        if !orig.exists() {
            std::fs::copy(dir.join(t), &orig)?;
        }
    }
    let transcript = dir.join("transcript.json");
    let transcript_orig = dir.join("transcript.orig.json");
    if transcript.exists() && !transcript_orig.exists() {
        std::fs::copy(&transcript, &transcript_orig)?;
    }
    let diar = dir.join(DIAR_CACHE);
    let diar_orig = dir.join("diarization.orig.json");
    if diar.exists() && !diar_orig.exists() {
        std::fs::copy(&diar, &diar_orig)?;
    }
    Ok(())
}

/// Тяжёлая файловая часть правки аудио БЕЗ обращения к БД (можно гнать в
/// `spawn_blocking`): бэкап оригинала → вырезание `cuts` из ВСЕХ дорожек
/// встречи → пересчёт времён `transcript.json` под вырезы.
/// Возвращает новую длительность встречи в секундах (округление вниз).
pub fn apply_audio_edit_files(data_root: &Path, id: &str, cuts: &[Range]) -> AppResult<u64> {
    validate_id(id)?;
    let merged = edit::merge_ranges(cuts);
    if merged.is_empty() {
        return Err(AppError::InvalidInput(
            "не выбрано, что вырезать из записи".into(),
        ));
    }
    let dir = meeting_dir(data_root, id);
    let tracks = existing_tracks(&dir);
    if tracks.is_empty() {
        return Err(AppError::NotFound(format!("аудио встречи {id}")));
    }
    backup_originals(&dir, &tracks)?;

    let mut longest = 0.0f64;
    for t in &tracks {
        let src = dir.join(t);
        let tmp = dir.join(format!("{t}.edit.tmp"));
        let kept = match edit::apply_cuts(&src, &tmp, &merged) {
            Ok(k) => k,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        std::fs::rename(&tmp, &src)?;
        if kept > longest {
            longest = kept;
        }
    }

    // Расшифровка живёт в той же системе координат, что дорожки, — сдвигаем её
    // тем же набором вырезов, чтобы реплики не разъехались со звуком.
    if let Some(transcript) = load_transcript(data_root, id)? {
        let remapped = edit::remap_transcript(&transcript, &merged);
        save_transcript(data_root, id, &remapped)?;
    }
    remap_diar_cache(&dir, &merged)?;
    Ok(longest as u64)
}

/// Возвращает встречу к оригиналу из бэкапа (`*.orig.wav` +
/// `transcript.orig.json`). Бэкап остаётся на месте — правку можно повторить.
/// Возвращает длительность оригинала в секундах.
pub fn revert_audio_edit_files(data_root: &Path, id: &str) -> AppResult<u64> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    let mut restored = 0usize;
    let mut longest = 0u64;
    for t in TRACK_FILES {
        let orig = dir.join(orig_name(t));
        if !orig.exists() {
            continue;
        }
        let dst = dir.join(t);
        std::fs::copy(&orig, &dst)?;
        restored += 1;
        longest = longest.max(wav_duration_secs(&dst).unwrap_or(0));
    }
    if restored == 0 {
        return Err(AppError::InvalidState(
            "оригинал не сохранён — правок аудио не было".into(),
        ));
    }
    let transcript_orig = dir.join("transcript.orig.json");
    if transcript_orig.exists() {
        std::fs::copy(&transcript_orig, dir.join("transcript.json"))?;
    }
    let diar_orig = dir.join("diarization.orig.json");
    if diar_orig.exists() {
        std::fs::copy(&diar_orig, dir.join(DIAR_CACHE))?;
    }
    Ok(longest)
}

/// Тяжёлая часть расшифровки БЕЗ обращения к БД: читает дорожки,
/// прогоняет транскрайбер, пишет `transcript.json`. Вынесена отдельно,
/// чтобы вызывающий не держал блокировку БД во время долгой работы.
pub fn transcribe_to_file(
    transcriber: &dyn Transcriber,
    data_root: &Path,
    id: &str,
) -> AppResult<Transcript> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    let mic = transcriber.transcribe(&dir.join("mic.wav"))?;
    let system = transcriber.transcribe(&dir.join("system.wav"))?;
    let transcript = merge_tracks(mic, system);
    let json = serde_json::to_string_pretty(&transcript)?;
    std::fs::write(dir.join("transcript.json"), json)?;
    Ok(transcript)
}

/// Расшифровка ТОЛЬКО дорожки микрофона записанной встречи (соло-режим «я один»):
/// пишет `transcript.json`, всем сегментам один говорящий `me`. Дорожка
/// собеседника и диаризация пропускаются — это заметка на один голос.
pub fn transcribe_solo_to_file(
    transcriber: &dyn Transcriber,
    data_root: &Path,
    id: &str,
) -> AppResult<Transcript> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    let segs = transcriber.transcribe(&dir.join("mic.wav"))?;
    let transcript = single_speaker(segs, crate::transcript::ME);
    let json = serde_json::to_string_pretty(&transcript)?;
    std::fs::write(dir.join("transcript.json"), json)?;
    Ok(transcript)
}

/// Расшифровка ОДНОЙ дорожки (`audio.wav`) импортированной встречи: пишет
/// `transcript.json`, всем сегментам один говорящий `spk0` (диаризация — M3).
pub fn transcribe_single_to_file(
    transcriber: &dyn Transcriber,
    data_root: &Path,
    id: &str,
) -> AppResult<Transcript> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    let segs = transcriber.transcribe(&dir.join("audio.wav"))?;
    let transcript = crate::transcript::single_speaker(segs, "spk0");
    let json = serde_json::to_string_pretty(&transcript)?;
    std::fs::write(dir.join("transcript.json"), json)?;
    Ok(transcript)
}

/// Расшифровывает обе дорожки встречи, сохраняет `transcript.json` и
/// ставит статус "transcribed".
pub fn transcribe_meeting(
    transcriber: &dyn Transcriber,
    repo: &Repo,
    data_root: &Path,
    id: &str,
) -> AppResult<Transcript> {
    let transcript = transcribe_to_file(transcriber, data_root, id)?;
    repo.update_status(id, "transcribed")?;
    Ok(transcript)
}

/// Сохраняет уже готовую расшифровку в `transcript.json`.
pub fn save_transcript(data_root: &Path, id: &str, transcript: &Transcript) -> AppResult<()> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(transcript)?;
    std::fs::write(dir.join("transcript.json"), json)?;
    Ok(())
}

/// Читает сохранённую расшифровку встречи, если она есть.
pub fn load_transcript(data_root: &Path, id: &str) -> AppResult<Option<Transcript>> {
    validate_id(id)?;
    let path = meeting_dir(data_root, id).join("transcript.json");
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(path)?;
    let transcript: Transcript = serde_json::from_str(&data)?;
    Ok(Some(transcript))
}

/// Сохраняет выжимку встречи в файл `summary.md`.
pub fn save_summary(data_root: &Path, id: &str, summary: &str) -> AppResult<()> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("summary.md"), summary)?;
    Ok(())
}

/// Читает сохранённую выжимку, если есть.
pub fn load_summary(data_root: &Path, id: &str) -> AppResult<Option<String>> {
    validate_id(id)?;
    let path = meeting_dir(data_root, id).join("summary.md");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(std::fs::read_to_string(path)?))
}

/// Сохраняет литературный пересказ встречи в файл `literary.md`.
pub fn save_literary(data_root: &Path, id: &str, text: &str) -> AppResult<()> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("literary.md"), text)?;
    Ok(())
}

/// Читает сохранённый литературный пересказ, если есть.
pub fn load_literary(data_root: &Path, id: &str) -> AppResult<Option<String>> {
    validate_id(id)?;
    let path = meeting_dir(data_root, id).join("literary.md");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(std::fs::read_to_string(path)?))
}

/// Сохраняет краткое резюме встречи в `brief.md`.
pub fn save_brief(data_root: &Path, id: &str, text: &str) -> AppResult<()> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("brief.md"), text)?;
    Ok(())
}

/// Читает сохранённое краткое резюме, если есть.
pub fn load_brief(data_root: &Path, id: &str) -> AppResult<Option<String>> {
    validate_id(id)?;
    let path = meeting_dir(data_root, id).join("brief.md");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(std::fs::read_to_string(path)?))
}

/// Сохраняет ИИ-анализ встречи в `analysis.md`.
pub fn save_analysis(data_root: &Path, id: &str, text: &str) -> AppResult<()> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("analysis.md"), text)?;
    Ok(())
}

/// Читает сохранённый ИИ-анализ, если есть.
pub fn load_analysis(data_root: &Path, id: &str) -> AppResult<Option<String>> {
    validate_id(id)?;
    let path = meeting_dir(data_root, id).join("analysis.md");
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(std::fs::read_to_string(path)?))
}

// ── ИИ-отчёты (общий механизм) ─────────────────────────────────────────────

/// Виды ИИ-отчётов и их файлы в папке встречи. `brief` — устаревшее «Краткое
/// резюме» (новые не создаются, старые показываются и экспортируются).
pub const REPORT_KINDS: &[(&str, &str)] = &[
    ("summary", "summary.md"),
    ("tasks", "tasks.md"),
    ("analysis", "analysis.md"),
    ("literary", "literary.md"),
    ("followup", "followup.md"),
    ("agent", "agent.md"),
    ("brief", "brief.md"),
];

fn report_file(kind: &str) -> AppResult<&'static str> {
    REPORT_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, f)| *f)
        .ok_or_else(|| AppError::InvalidInput(format!("unknown report kind: {kind}")))
}

/// Сохраняет ИИ-отчёт вида `kind` (см. [`REPORT_KINDS`]).
pub fn save_report(data_root: &Path, id: &str, kind: &str, text: &str) -> AppResult<()> {
    validate_id(id)?;
    let file = report_file(kind)?;
    let dir = meeting_dir(data_root, id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(file), text)?;
    Ok(())
}

/// Читает ИИ-отчёт вида `kind`, если он есть.
pub fn load_report(data_root: &Path, id: &str, kind: &str) -> AppResult<Option<String>> {
    validate_id(id)?;
    let path = meeting_dir(data_root, id).join(report_file(kind)?);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(std::fs::read_to_string(path)?))
}

/// Все сохранённые ИИ-отчёты встречи: `(вид, текст)`.
pub fn load_reports(data_root: &Path, id: &str) -> AppResult<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (kind, _) in REPORT_KINDS {
        if let Some(text) = load_report(data_root, id, kind)? {
            out.push((kind.to_string(), text));
        }
    }
    Ok(out)
}

// ── Кеш диаризации ──────────────────────────────────────────────────────────

/// Результат анализа голосов дорожки: фрагменты речи с эмбеддингами. По нему
/// разметка говорящих пересчитывается мгновенно под другое число голосов.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DiarCache {
    /// Какая дорожка анализировалась: "audio.wav" (импорт) или "system.wav".
    pub track: String,
    pub windows: Vec<crate::cluster::EmbWindow>,
}

const DIAR_CACHE: &str = "diarization.json";

pub fn save_diar_cache(data_root: &Path, id: &str, cache: &DiarCache) -> AppResult<()> {
    validate_id(id)?;
    let dir = meeting_dir(data_root, id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(DIAR_CACHE), serde_json::to_string(cache)?)?;
    Ok(())
}

pub fn load_diar_cache(data_root: &Path, id: &str) -> AppResult<Option<DiarCache>> {
    validate_id(id)?;
    let path = meeting_dir(data_root, id).join(DIAR_CACHE);
    if !path.exists() {
        return Ok(None);
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?).ok())
}

/// Сдвигает времена кеша диаризации под вырезы аудио-редактора.
fn remap_diar_cache(dir: &Path, cuts: &[Range]) -> AppResult<()> {
    let path = dir.join(DIAR_CACHE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let Ok(mut cache) = serde_json::from_str::<DiarCache>(&text) else {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    };
    cache.windows = cache
        .windows
        .into_iter()
        .filter_map(|mut w| {
            let spans: Vec<(f64, f64)> = w
                .spans()
                .into_iter()
                .map(|(a, b)| (edit::map_time(a, cuts), edit::map_time(b, cuts)))
                .filter(|(a, b)| b - a >= 0.05)
                .collect();
            if spans.is_empty() {
                return None;
            }
            w.start_secs = spans[0].0;
            w.end_secs = spans[spans.len() - 1].1;
            w.spans = spans;
            Some(w)
        })
        .collect();
    std::fs::write(&path, serde_json::to_string(&cache)?)?;
    Ok(())
}

/// Переразмечает говорящих в сохранённой расшифровке по кешу диаризации под
/// новое число голосов (`None` — авто). Текст реплик (в т.ч. правки
/// пользователя) не меняется. Для записанной встречи трогаются только реплики
/// собеседников — «Я» остаётся «Я».
pub fn recluster_transcript(
    data_root: &Path,
    id: &str,
    num_speakers: Option<usize>,
) -> AppResult<Transcript> {
    let cache = load_diar_cache(data_root, id)?.ok_or_else(|| {
        AppError::InvalidState(
            "для этой встречи нет данных о голосах — расшифруйте её заново".into(),
        )
    })?;
    let transcript = load_transcript(data_root, id)?.ok_or_else(|| {
        AppError::InvalidState("нет расшифровки — сначала расшифруйте встречу".into())
    })?;
    let diar = crate::cluster::diarize_windows(&cache.windows, num_speakers);
    use crate::transcript::{ME, THEM};
    let relabeled = match cache.track.as_str() {
        // Звонок: голоса собеседников, «Я» не трогаем.
        "system.wav" => crate::transcript::relabel_speakers(&transcript, &diar, |s| s.speaker != ME, Some(THEM)),
        // Живая встреча: голоса в микрофоне (реплики из звонка не трогаем);
        // один голос — «Я».
        "mic.wav" => crate::transcript::relabel_speakers(&transcript, &diar, |s| s.speaker != THEM, Some(ME)),
        _ => crate::transcript::relabel_speakers(&transcript, &diar, |_| true, None),
    };
    save_transcript(data_root, id, &relabeled)?;
    Ok(relabeled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::MockRecorder;
    use crate::transcriber::MockTranscriber;
    use crate::transcript::Segment;

    fn setup() -> (tempfile::TempDir, Repo, MockRecorder) {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repo::open_in_memory().unwrap();
        let rec = MockRecorder::new(2);
        (dir, repo, rec)
    }

    #[test]
    fn start_creates_folder_and_first_segment() {
        let (dir, _repo, rec) = setup();
        let active =
            start_recording(&rec, dir.path(), "m1".into()).unwrap();
        assert!(dir.path().join("meetings/m1").exists());
        let (mic, system) = active.current.as_ref().unwrap();
        assert!(mic.ends_with("mic.part0.wav"));
        assert!(system.ends_with("system.part0.wav"));
        assert!(active.segments.is_empty());
    }

    #[test]
    fn pause_resume_merges_segments_into_single_file() {
        let (dir, repo, rec) = setup(); // MockRecorder пишет 2 с тишины на стоп
        let active = start_recording(&rec, dir.path(), "m1".into()).unwrap();
        let active = pause_recording(&rec, active).unwrap();
        assert_eq!(active.segments.len(), 1);
        assert!(active.current.is_none());
        let active = resume_recording(&rec, active).unwrap();
        assert!(active.current.is_some());
        let m = stop_recording(&rec, &repo, &active, "2026-06-04T10:00:00Z".into()).unwrap();
        // Два сегмента по 2 с → 4 с в едином файле; части удалены.
        assert_eq!(m.duration_secs, 4);
        assert!(dir.path().join("meetings/m1/mic.wav").exists());
        assert!(!dir.path().join("meetings/m1/mic.part0.wav").exists());
        assert!(!dir.path().join("meetings/m1/mic.part1.wav").exists());
    }

    #[test]
    fn recover_orphan_concats_parts_and_inserts_meeting() {
        let (dir, repo, _rec) = setup();
        let mdir = dir.path().join("meetings/orphan");
        std::fs::create_dir_all(&mdir).unwrap();
        crate::audio::write_silence_wav(&mdir.join("mic.part0.wav"), 1).unwrap();
        crate::audio::write_silence_wav(&mdir.join("mic.part1.wav"), 1).unwrap();
        crate::audio::write_silence_wav(&mdir.join("system.part0.wav"), 1).unwrap();
        crate::audio::write_silence_wav(&mdir.join("system.part1.wav"), 1).unwrap();

        let n = recover_orphan_recordings(&repo, dir.path(), "2026-06-04T10:00:00Z".into()).unwrap();
        assert_eq!(n, 1);
        assert!(mdir.join("mic.wav").exists());
        assert!(!mdir.join("mic.part0.wav").exists());
        let m = repo.get("orphan").unwrap();
        assert_eq!(m.duration_secs, 2);
        assert_eq!(m.title, "Восстановленная запись");
    }

    #[test]
    fn recover_skips_finalized_meetings() {
        let (dir, repo, rec) = setup();
        let active = start_recording(&rec, dir.path(), "m1".into()).unwrap();
        stop_recording(&rec, &repo, &active, "2026-06-04T10:00:00Z".into()).unwrap();
        // После штатного стопа частей нет → восстанавливать нечего.
        let n = recover_orphan_recordings(&repo, dir.path(), "2026-06-04T11:00:00Z".into()).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn stop_saves_meeting_with_duration() {
        let (dir, repo, rec) = setup();
        let active = start_recording(&rec, dir.path(), "m1".into()).unwrap();
        let m = stop_recording(&rec, &repo, &active, "2026-06-04T10:00:00Z".into()).unwrap();
        assert_eq!(m.duration_secs, 2);
        assert_eq!(m.status, "recorded");
        assert_eq!(repo.get("m1").unwrap(), m);
    }

    #[test]
    fn delete_removes_db_row_and_folder() {
        let (dir, repo, rec) = setup();
        let active = start_recording(&rec, dir.path(), "m1".into()).unwrap();
        stop_recording(&rec, &repo, &active, "2026-06-04T10:00:00Z".into()).unwrap();
        delete_meeting(&repo, dir.path(), "m1").unwrap();
        assert!(repo.list().unwrap().is_empty());
        assert!(!dir.path().join("meetings/m1").exists());
    }

    #[test]
    fn track_path_rejects_unknown_file() {
        let (dir, _repo, _rec) = setup();
        assert!(matches!(
            track_path(dir.path(), "m1", "secrets.txt"),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn rejects_path_traversal_id() {
        let (dir, repo, rec) = setup();
        assert!(matches!(
            start_recording(&rec, dir.path(), "../escape".into()),
            Err(AppError::InvalidInput(_))
        ));
        assert!(matches!(
            track_path(dir.path(), "../escape", "mic.wav"),
            Err(AppError::InvalidInput(_))
        ));
        assert!(matches!(
            delete_meeting(&repo, dir.path(), "../escape"),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn track_path_returns_existing_file() {
        let (dir, repo, rec) = setup();
        let active = start_recording(&rec, dir.path(), "m1".into()).unwrap();
        stop_recording(&rec, &repo, &active, "2026-06-04T10:00:00Z".into()).unwrap();
        let p = track_path(dir.path(), "m1", "mic.wav").unwrap();
        assert!(p.exists());
    }

    fn seg(start: f64, text: &str) -> Segment {
        Segment { start_secs: start, end_secs: start + 1.0, text: text.into() }
    }

    #[test]
    fn load_transcript_none_before_transcription() {
        let (dir, repo, rec) = setup();
        let active = start_recording(&rec, dir.path(), "m1".into()).unwrap();
        stop_recording(&rec, &repo, &active, "2026-06-04T10:00:00Z".into()).unwrap();
        assert!(load_transcript(dir.path(), "m1").unwrap().is_none());
    }

    #[test]
    fn reports_by_kind_roundtrip_and_reject_unknown() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_report(dir.path(), "m1", "tasks").unwrap().is_none());
        save_report(dir.path(), "m1", "tasks", "| задача |").unwrap();
        save_report(dir.path(), "m1", "followup", "письмо").unwrap();
        assert_eq!(load_report(dir.path(), "m1", "tasks").unwrap().unwrap(), "| задача |");
        let all = load_reports(dir.path(), "m1").unwrap();
        let kinds: Vec<&str> = all.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(kinds, vec!["tasks", "followup"]);
        assert!(save_report(dir.path(), "m1", "../evil", "x").is_err());
        // Старые функции и общий механизм пишут одни и те же файлы.
        save_summary(dir.path(), "m1", "итоги").unwrap();
        assert_eq!(load_report(dir.path(), "m1", "summary").unwrap().unwrap(), "итоги");
    }

    fn win(a: f64, b: f64, e: &[f32]) -> crate::cluster::EmbWindow {
        crate::cluster::EmbWindow { start_secs: a, end_secs: b, embedding: e.to_vec(), spans: vec![] }
    }

    #[test]
    fn recluster_relabels_without_touching_text_or_me() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let t = crate::transcript::merge_tracks(
            vec![seg(0.0, "я говорю")],
            vec![seg(10.0, "первый"), seg(30.0, "второй")],
        );
        save_transcript(root, "m1", &t).unwrap();
        // Два явно разных голоса на системной дорожке.
        let mut windows = Vec::new();
        for k in 0..8 {
            windows.push(win(8.0 + k as f64 * 0.5, 8.5 + k as f64 * 0.5, &[1.0, 0.0, 0.0]));
            windows.push(win(28.0 + k as f64 * 0.5, 28.5 + k as f64 * 0.5, &[0.0, 1.0, 0.0]));
        }
        save_diar_cache(root, "m1", &DiarCache { track: "system.wav".into(), windows }).unwrap();

        let r = recluster_transcript(root, "m1", None).unwrap();
        let sp: Vec<&str> = r.segments.iter().map(|s| s.speaker.as_str()).collect();
        assert_eq!(sp, vec!["me", "spk0", "spk1"]);
        assert_eq!(r.segments[0].text, "я говорю");
        // Один голос → «Собеседник».
        let r = recluster_transcript(root, "m1", Some(1)).unwrap();
        let sp: Vec<&str> = r.segments.iter().map(|s| s.speaker.as_str()).collect();
        assert_eq!(sp, vec!["me", "them", "them"]);
        // Сохранено на диск.
        assert_eq!(load_transcript(root, "m1").unwrap().unwrap(), r);
    }

    #[test]
    fn recluster_without_cache_is_error() {
        let dir = tempfile::tempdir().unwrap();
        save_transcript(dir.path(), "m1", &Transcript::default()).unwrap();
        assert!(recluster_transcript(dir.path(), "m1", None).is_err());
    }

    #[test]
    fn summary_save_and_load_roundtrip() {
        let (dir, _repo, _rec) = setup();
        assert!(load_summary(dir.path(), "m1").unwrap().is_none());
        save_summary(dir.path(), "m1", "## выжимка").unwrap();
        assert_eq!(load_summary(dir.path(), "m1").unwrap().unwrap(), "## выжимка");
    }

    #[test]
    fn transcribe_meeting_writes_and_sets_status() {
        let (dir, repo, rec) = setup();
        let active = start_recording(&rec, dir.path(), "m1".into()).unwrap();
        stop_recording(&rec, &repo, &active, "2026-06-04T10:00:00Z".into()).unwrap();

        let t = MockTranscriber::new(vec![seg(0.0, "привет")], vec![seg(1.0, "здравствуй")]);
        let transcript = transcribe_meeting(&t, &repo, dir.path(), "m1").unwrap();

        assert_eq!(transcript.segments.len(), 2);
        assert_eq!(transcript.segments[0].speaker, "me");
        assert_eq!(transcript.segments[1].speaker, "them");
        assert_eq!(repo.get("m1").unwrap().status, "transcribed");
        // и читается обратно
        let loaded = load_transcript(dir.path(), "m1").unwrap().unwrap();
        assert_eq!(loaded, transcript);
    }

    #[test]
    fn import_recording_decodes_and_inserts() {
        let (dir, repo, _rec) = setup();
        // «Файл с телефона»: простой WAV (16 кГц моно), имя с пробелом.
        let src = dir.path().join("voice memo.wav");
        crate::audio::write_silence_wav(&src, 1).unwrap();

        let m = import_recording(
            &repo,
            dir.path(),
            "imp1".into(),
            &src,
            "2026-06-04T10:00:00Z".into(),
        )
        .unwrap();

        assert_eq!(m.source, "imported");
        assert_eq!(m.title, "voice memo");
        assert!(dir.path().join("meetings/imp1/audio.wav").exists());
        assert_eq!(repo.get("imp1").unwrap().source, "imported");
    }

    // ── Правка аудио ─────────────────────────────────────────────────────────

    /// Кладёт на диск «записанную» встречу с двумя дорожками по `secs` секунд.
    fn make_recorded(root: &Path, repo: &Repo, id: &str, secs: u64) {
        let dir = root.join("meetings").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        crate::audio::write_silence_wav(&dir.join("mic.wav"), secs).unwrap();
        crate::audio::write_silence_wav(&dir.join("system.wav"), secs).unwrap();
        repo.insert(&Meeting {
            id: id.into(),
            created_at: "2026-08-18T10:00:00Z".into(),
            title: "Встреча".into(),
            participants: String::new(),
            topic: String::new(),
            duration_secs: secs,
            folder: id.into(),
            status: "recorded".into(),
            source: "recorded".into(),
            notes: String::new(),
        })
        .unwrap();
    }

    #[test]
    fn audio_edit_state_lists_tracks_without_backup() {
        let (dir, repo, _rec) = setup();
        make_recorded(dir.path(), &repo, "m1", 3);
        let st = audio_edit_state(dir.path(), "m1").unwrap();
        assert_eq!(st.tracks, vec!["mic.wav", "system.wav"]);
        assert!(!st.has_original);
    }

    #[test]
    fn apply_cuts_both_tracks_and_keeps_original() {
        let (dir, repo, _rec) = setup();
        make_recorded(dir.path(), &repo, "m1", 4);
        let mdir = dir.path().join("meetings/m1");

        let secs = apply_audio_edit_files(dir.path(), "m1", &[Range::new(1.0, 2.0)]).unwrap();

        assert_eq!(secs, 3);
        // Обе дорожки укоротились одинаково — синхронность сохранена.
        assert_eq!(wav_duration_secs(&mdir.join("mic.wav")).unwrap(), 3);
        assert_eq!(wav_duration_secs(&mdir.join("system.wav")).unwrap(), 3);
        // Оригинал сохранён целиком, временных файлов не осталось.
        assert_eq!(wav_duration_secs(&mdir.join("mic.orig.wav")).unwrap(), 4);
        assert_eq!(wav_duration_secs(&mdir.join("system.orig.wav")).unwrap(), 4);
        assert!(!mdir.join("mic.wav.edit.tmp").exists());
        assert!(audio_edit_state(dir.path(), "m1").unwrap().has_original);
    }

    #[test]
    fn second_apply_does_not_overwrite_first_backup() {
        let (dir, repo, _rec) = setup();
        make_recorded(dir.path(), &repo, "m1", 5);
        let mdir = dir.path().join("meetings/m1");

        apply_audio_edit_files(dir.path(), "m1", &[Range::new(0.0, 1.0)]).unwrap();
        let secs = apply_audio_edit_files(dir.path(), "m1", &[Range::new(0.0, 1.0)]).unwrap();

        assert_eq!(secs, 3); // 5 → 4 → 3
        // Бэкап всё ещё указывает на исходные 5 секунд.
        assert_eq!(wav_duration_secs(&mdir.join("mic.orig.wav")).unwrap(), 5);
    }

    #[test]
    fn apply_remaps_transcript_times() {
        let (dir, repo, _rec) = setup();
        make_recorded(dir.path(), &repo, "m1", 6);
        let transcript = Transcript {
            segments: vec![
                crate::transcript::TranscriptSegment {
                    speaker: "me".into(),
                    start_secs: 0.0,
                    end_secs: 1.0,
                    text: "до".into(),
                },
                crate::transcript::TranscriptSegment {
                    speaker: "them".into(),
                    start_secs: 2.2,
                    end_secs: 2.8,
                    text: "вырезанное".into(),
                },
                crate::transcript::TranscriptSegment {
                    speaker: "me".into(),
                    start_secs: 4.0,
                    end_secs: 5.0,
                    text: "после".into(),
                },
            ],
        };
        save_transcript(dir.path(), "m1", &transcript).unwrap();

        apply_audio_edit_files(dir.path(), "m1", &[Range::new(2.0, 3.0)]).unwrap();

        let out = load_transcript(dir.path(), "m1").unwrap().unwrap();
        let texts: Vec<&str> = out.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["до", "после"]);
        assert_eq!(out.segments[1].start_secs, 3.0);
        // Бэкап расшифровки хранит все три реплики.
        let orig: Transcript = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("meetings/m1/transcript.orig.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(orig.segments.len(), 3);
    }

    #[test]
    fn revert_restores_tracks_and_transcript() {
        let (dir, repo, _rec) = setup();
        make_recorded(dir.path(), &repo, "m1", 4);
        let transcript = Transcript {
            segments: vec![crate::transcript::TranscriptSegment {
                speaker: "me".into(),
                start_secs: 1.2,
                end_secs: 1.8,
                text: "вырезанное".into(),
            }],
        };
        save_transcript(dir.path(), "m1", &transcript).unwrap();
        apply_audio_edit_files(dir.path(), "m1", &[Range::new(1.0, 2.0)]).unwrap();
        assert!(load_transcript(dir.path(), "m1").unwrap().unwrap().segments.is_empty());

        let secs = revert_audio_edit_files(dir.path(), "m1").unwrap();

        assert_eq!(secs, 4);
        let mdir = dir.path().join("meetings/m1");
        assert_eq!(wav_duration_secs(&mdir.join("mic.wav")).unwrap(), 4);
        assert_eq!(wav_duration_secs(&mdir.join("system.wav")).unwrap(), 4);
        assert_eq!(load_transcript(dir.path(), "m1").unwrap().unwrap(), transcript);
        // Бэкап остался — правку можно повторить.
        assert!(audio_edit_state(dir.path(), "m1").unwrap().has_original);
    }

    #[test]
    fn apply_to_imported_meeting_cuts_single_track() {
        let (dir, repo, _rec) = setup();
        let src = dir.path().join("voice.wav");
        crate::audio::write_silence_wav(&src, 3).unwrap();
        import_recording(&repo, dir.path(), "imp1".into(), &src, "2026-08-18T10:00:00Z".into())
            .unwrap();

        let secs = apply_audio_edit_files(dir.path(), "imp1", &[Range::new(0.0, 1.0)]).unwrap();

        assert_eq!(secs, 2);
        let mdir = dir.path().join("meetings/imp1");
        assert_eq!(wav_duration_secs(&mdir.join("audio.wav")).unwrap(), 2);
        assert!(mdir.join("audio.orig.wav").exists());
    }

    #[test]
    fn apply_without_cuts_is_rejected() {
        let (dir, repo, _rec) = setup();
        make_recorded(dir.path(), &repo, "m1", 2);
        assert!(matches!(
            apply_audio_edit_files(dir.path(), "m1", &[]),
            Err(AppError::InvalidInput(_))
        ));
        // Пустой интервал тоже не считается правкой.
        assert!(matches!(
            apply_audio_edit_files(dir.path(), "m1", &[Range::new(1.0, 1.0)]),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn revert_without_backup_is_error() {
        let (dir, repo, _rec) = setup();
        make_recorded(dir.path(), &repo, "m1", 2);
        assert!(matches!(
            revert_audio_edit_files(dir.path(), "m1"),
            Err(AppError::InvalidState(_))
        ));
    }

    #[test]
    fn audio_edit_rejects_path_traversal_id() {
        let (dir, _repo, _rec) = setup();
        assert!(matches!(
            apply_audio_edit_files(dir.path(), "../escape", &[Range::new(0.0, 1.0)]),
            Err(AppError::InvalidInput(_))
        ));
        assert!(matches!(
            audio_edit_state(dir.path(), "../escape"),
            Err(AppError::InvalidInput(_))
        ));
        assert!(matches!(
            revert_audio_edit_files(dir.path(), "../escape"),
            Err(AppError::InvalidInput(_))
        ));
    }

    #[test]
    fn transcribe_solo_writes_me_only() {
        let (dir, repo, rec) = setup();
        let active = start_recording(&rec, dir.path(), "m1".into()).unwrap();
        stop_recording(&rec, &repo, &active, "2026-06-04T10:00:00Z".into()).unwrap();
        // mic → «моя заметка»; system-дорожка в соло игнорируется.
        let t = MockTranscriber::new(vec![seg(0.0, "моя заметка")], vec![seg(1.0, "шум")]);
        let transcript = transcribe_solo_to_file(&t, dir.path(), "m1").unwrap();
        assert_eq!(transcript.segments.len(), 1);
        assert!(transcript.segments.iter().all(|s| s.speaker == "me"));
    }

    #[test]
    fn transcribe_single_writes_one_speaker() {
        let (dir, repo, _rec) = setup();
        let src = dir.path().join("rec.wav");
        crate::audio::write_silence_wav(&src, 1).unwrap();
        import_recording(&repo, dir.path(), "imp2".into(), &src, "2026-06-04T10:00:00Z".into())
            .unwrap();

        // MockTranscriber для не-"mic.wav" (т.е. audio.wav) отдаёт system-сегменты.
        let t = MockTranscriber::new(vec![], vec![seg(0.0, "первая"), seg(1.0, "вторая")]);
        let transcript = transcribe_single_to_file(&t, dir.path(), "imp2").unwrap();

        assert_eq!(transcript.segments.len(), 2);
        assert!(transcript.segments.iter().all(|s| s.speaker == "spk0"));
    }
}
