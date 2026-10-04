import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import type {
  Meeting,
  TrackFile,
  Transcript,
  AiConfig,
  MetadataSuggestion,
  WhisperConfig,
  RecState,
  ReportKind,
  TrackLevels,
  AudioRange,
  Waveform,
  AudioEditState,
  AiCheck,
  MeetingContext,
  ModelInfo,
  MissDiagnosis,
} from "./types";
import { logError, logInfo } from "./log";
import { conversationLanguages } from "./settings";

/// invoke с логированием ошибок в диагностику.
async function inv<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    logError(`invoke ${cmd}`, e);
    throw e;
  }
}

/// Пустые строки → undefined, чтобы на стороне Rust получился None.
function whisperOptions(w: WhisperConfig) {
  return {
    whisperPath: w.whisperPath || undefined,
    model: w.model || undefined,
    language: w.language || undefined,
    vocabulary: [w.vocabulary, w.learned].map((v) => v?.trim()).filter(Boolean).join("\n") || undefined,
    languages: conversationLanguages(w).length ? conversationLanguages(w) : undefined,
  };
}

export const api = {
  startRecording: (): Promise<string> => inv("start_recording"),
  stopRecording: (): Promise<Meeting> => inv("stop_recording"),
  pauseRecording: (): Promise<void> => inv("pause_recording"),
  resumeRecording: (): Promise<void> => inv("resume_recording"),
  importRecording: (path: string): Promise<Meeting> =>
    inv("import_recording", { path }),
  listMeetings: (): Promise<Meeting[]> => inv("list_meetings"),
  getMeeting: (id: string): Promise<Meeting> => inv("get_meeting", { id }),
  deleteMeeting: (id: string): Promise<void> => inv("delete_meeting", { id }),
  isRecording: (): Promise<boolean> => inv("is_recording"),
  recordingState: (): Promise<RecState> => inv("recording_state"),
  recordingLevels: (): Promise<TrackLevels> => inv("recording_levels"),

  transcribe: (
    id: string,
    whisper: WhisperConfig,
    speakerCount?: number | null,
    solo?: boolean,
    totalVoices?: boolean,
  ): Promise<Transcript> => {
    logInfo(
      `transcribe start id=${id} model=${whisper.model || "default"} speakers=${speakerCount ?? "auto"}${solo ? " solo" : ""}`,
    );
    return inv("transcribe", {
      id,
      options: whisperOptions(whisper),
      speakerCount: speakerCount ?? null,
      solo: solo ?? null,
      totalVoices: totalVoices ?? null,
    });
  },
  getTranscript: (id: string): Promise<Transcript | null> =>
    inv("get_transcript", { id }),
  /// Сохранить отредактированную расшифровку (правки whisper-ошибок/спикеров).
  saveTranscript: (id: string, transcript: Transcript): Promise<void> =>
    inv("save_transcript", { id, transcript }),
  /// Сохранить отредактированный ИИ-отчёт (brief|summary|analysis|literary).
  saveReport: (id: string, kind: ReportKind, content: string): Promise<void> =>
    inv("save_report", { id, kind, content }),

  suggestMetadata: (id: string, config: AiConfig): Promise<MetadataSuggestion> =>
    inv("suggest_metadata", { id, config }),
  summarize: (id: string, config: AiConfig): Promise<string> =>
    inv("summarize", { id, config }),
  getSummary: (id: string): Promise<string | null> => inv("get_summary", { id }),
  literaryText: (id: string, config: AiConfig): Promise<string> =>
    inv("literary_text", { id, config }),
  getLiterary: (id: string): Promise<string | null> =>
    inv("get_literary", { id }),
  briefSummary: (id: string, config: AiConfig): Promise<string> =>
    inv("brief_summary", { id, config }),
  getBrief: (id: string): Promise<string | null> => inv("get_brief", { id }),
  analyze: (id: string, config: AiConfig): Promise<string> =>
    inv("analyze", { id, config }),
  getAnalysis: (id: string): Promise<string | null> =>
    inv("get_analysis", { id }),
  ask: (id: string, config: AiConfig, question: string): Promise<string> =>
    inv("ask", { id, config, question }),
  updateMeetingMeta: (
    id: string,
    title: string,
    participants: string,
    topic: string,
  ): Promise<void> =>
    inv("update_meeting_meta", { id, title, participants, topic }),

  /// Заметки к встрече.
  updateMeetingNotes: (id: string, notes: string): Promise<void> =>
    inv("update_meeting_notes", { id, notes }),
  /// Проверить ИИ-сервер: доступность, модели, актуальная модель.
  aiCheck: (config: AiConfig): Promise<AiCheck> => inv("ai_check", { config }),

  saveTextFile: (path: string, content: string): Promise<void> =>
    inv("save_text_file", { path, content }),

  exportAudio: (id: string, trackFile: TrackFile, dest: string): Promise<void> =>
    inv("export_audio", { id, trackFile, dest }),

  /// Карта громкости дорожки для таймлайна редактора (`buckets` корзин).
  waveform: (
    id: string,
    trackFile: TrackFile,
    buckets: number,
  ): Promise<Waveform> => inv("waveform", { id, trackFile, buckets }),
  /// Какие дорожки есть у встречи и сохранён ли оригинал до правок.
  audioEditState: (id: string): Promise<AudioEditState> =>
    inv("audio_edit_state", { id }),
  /// Вырезать интервалы из всех дорожек встречи (оригинал сохраняется).
  applyAudioEdit: (id: string, cuts: AudioRange[]): Promise<Meeting> => {
    logInfo(`audio edit id=${id} cuts=${cuts.length}`);
    return inv("apply_audio_edit", { id, cuts });
  },
  /// Вернуть аудио и расшифровку встречи к оригиналу из бэкапа.
  revertAudioEdit: (id: string): Promise<Meeting> =>
    inv("revert_audio_edit", { id }),

  getBackendLog: (): Promise<string> => inv("get_backend_log"),

  // ---- Голоса (диаризация) ----
  /// Поменять число голосов в готовой расшифровке (мгновенно, без повторной
  /// расшифровки). null — определить автоматически.
  reclusterSpeakers: (id: string, speakerCount: number | null): Promise<Transcript> =>
    inv("recluster_speakers", { id, speakerCount }),
  /// Есть ли у встречи сохранённый анализ голосов.
  hasVoiceAnalysis: (id: string): Promise<boolean> =>
    inv("has_voice_analysis", { id }),
  /// Фоновое уточнение трудных мест (после расшифровки). Возвращает число улучшенных мест.
  refineTranscript: (id: string): Promise<number> => inv("refine_transcript", { id }),
  refineStatus: (id: string): Promise<{ pending: number; running: boolean }> => inv("refine_pending", { id }),
  cancelRefine: (id: string): Promise<void> => inv("cancel_refine", { id }),
  /// Распознать заново промежуток записи (пропуск в расшифровке).
  /// Правка пользователя: для дописанной реплики — разбор, почему её
  /// пропустило распознавание.
  recordCorrection: (
    id: string,
    correction: { kind: "added" | "edited"; start: number; end: number; before: string; after: string; speaker: string },
  ): Promise<MissDiagnosis | null> => inv("record_correction", { id, correction }),
  recognizeRange: (
    id: string,
    start: number,
    end: number,
    language?: string,
    languages?: string[],
  ): Promise<{ start_secs: number; end_secs: number; text: string }[]> =>
    inv("recognize_range", { id, start, end, language: language || null, languages: languages?.length ? languages : null }),
  /// Какая дорожка разделена по голосам: system.wav / mic.wav (живая встреча) / audio.wav.
  voiceAnalysisTrack: (id: string): Promise<string | null> =>
    inv("voice_analysis_track", { id }),

  // ---- Модели ----
  modelsStatus: (): Promise<ModelInfo[]> => inv("models_status"),
  downloadModel: (id: string): Promise<void> => inv("download_model", { id }),
  deleteModel: (id: string): Promise<void> => inv("delete_model", { id }),

  // ---- ИИ-отчёты (v0.8) ----
  generateReport: (
    id: string,
    kind: ReportKind,
    config: AiConfig,
    context: MeetingContext,
  ): Promise<string> => inv("generate_report", { id, kind, config, context }),
  getReports: (id: string): Promise<Partial<Record<ReportKind, string>>> =>
    inv("get_reports", { id }),
  suggestMeta: (
    id: string,
    config: AiConfig,
    context: MeetingContext,
  ): Promise<MetadataSuggestion> => inv("suggest_meta", { id, config, context }),
  askNamed: (
    id: string,
    config: AiConfig,
    question: string,
    context: MeetingContext,
  ): Promise<string> => inv("ask_named", { id, config, question, context }),

  /// Сохранить двоичный файл (DOCX) — данные в base64.
  saveBinaryFile: (path: string, base64: string): Promise<void> =>
    inv("save_binary_file", { path, base64 }),

  /// Зарегистрировать глобальную горячую клавишу старт/стоп записи.
  /// Пустая строка/null — выключить хоткей.
  updateHotkey: (accelerator: string | null): Promise<void> =>
    inv("update_hotkey", { accelerator: accelerator || null }),

  /// Обновить конфиг авто-записи звонков (фоновый монитор аудио-сессий).
  setAutorecord: (
    enabled: boolean,
    processes: string[],
    autoStop: boolean,
    startDelaySecs: number,
    minKeepSecs: number,
  ): Promise<void> =>
    inv("set_autorecord", {
      enabled,
      processes,
      autoStop,
      startDelaySecs,
      minKeepSecs,
    }),

  /// asset-URL дорожки для <audio>. `bust` добавляет метку версии — после
  /// правки аудио файл меняется по тому же пути, и без неё webview отдаёт
  /// старую копию из кеша.
  async trackUrl(
    id: string,
    trackFile: TrackFile,
    bust?: number,
  ): Promise<string> {
    const path: string = await inv("track_path", { id, trackFile });
    const url = convertFileSrc(path);
    return bust ? `${url}?v=${bust}` : url;
  },

  /// macOS: есть ли доступ к системному звуку («Запись экрана и системного
  /// звука»). `request` — показать системный запрос. На Windows всегда true.
  systemAudioAccess: (request = false): Promise<boolean> =>
    inv("system_audio_access", { request }),

  /// ОС, её версия и архитектура — для отчёта об ошибке.
  systemInfo: (): Promise<{ os: string; os_version: string; arch: string }> =>
    inv("system_info"),

  /// macOS: статус разрешений (микрофон, звук собеседников).
  macPermissions: (): Promise<import("./types").MacPermissionsState> => inv("mac_permissions"),
  requestMicAccess: (): Promise<void> => inv("request_mic_access"),
  requestSystemAudioAccess: (): Promise<void> => inv("request_system_audio_access"),

  /// Системные уведомления вкл/выкл; пробное — заодно спросит разрешение.
  setNotifications: (enabled: boolean): Promise<void> => inv("set_notifications", { enabled }),
  testNotification: (): Promise<void> => inv("test_notification"),

  /// macOS: открыть раздел «Конфиденциальность и безопасность».
  openPrivacySettings: (kind: "screen" | "mic"): Promise<void> =>
    inv("open_privacy_settings", { kind }),
};
