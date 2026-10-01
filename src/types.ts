export interface Meeting {
  id: string;
  created_at: string;
  title: string;
  participants: string;
  topic: string;
  duration_secs: number;
  folder: string;
  status: string;
  /// "recorded" (записана приложением, 2 дорожки) | "imported" (один файл).
  /// Необязательно: бэкенд всегда присылает, но старые объекты могут не иметь.
  source?: string;
  /// Заметки пользователя к встрече (свободный текст).
  notes?: string;
}

export type TrackFile = "mic.wav" | "system.wav" | "audio.wav";

/// Идентификатор говорящего: "me"/"them" для записей, "spk0"/"spk1"/… для
/// импортированных (после диаризации).
export type Speaker = string;

export interface TranscriptSegment {
  speaker: Speaker;
  start_secs: number;
  end_secs: number;
  text: string;
}

export interface Transcript {
  segments: TranscriptSegment[];
}

export interface AiConfig {
  base_url: string;
  api_key: string;
  model: string;
}

export interface MetadataSuggestion {
  title: string;
  participants: string;
  topic: string;
}

/// Настройки локального Whisper. Пустые поля означают «найди сам».
export interface WhisperConfig {
  whisperPath: string;
  model: string;
  language: string;
  /// Словарь: термины, имена, команды — по строке или через запятую.
  vocabulary: string;
}

/// Авто-запись звонков: следим за аудио-сессиями выбранных приложений и
/// автоматически стартуем запись при звонке.
export interface AutoRecordConfig {
  enabled: boolean;
  /// Ключи приложений для слежения (см. AUTO_RECORD_APPS в labels/настройках).
  apps: string[];
  /// Останавливать запись по завершении звонка.
  autoStop: boolean;
  /// Сколько секунд звонок должен держаться до старта записи — отсекает
  /// короткие звуки уведомлений (Telegram «дзынь»). 0 — старт сразу.
  startDelaySecs: number;
  /// Авто-удалять авто-записи короче этого порога (сек) как мусорные огрызки
  /// уведомлений. 0 — не удалять.
  minKeepSecs: number;
}

/// Состояние записи: идёт ли запись и стоит ли она на паузе.
export interface RecState {
  recording: boolean;
  paused: boolean;
}

/// Вид ИИ-отчёта. `brief` — устаревшее «Краткое резюме» (новые не создаются,
/// сохранённые показываются и экспортируются).
export type ReportKind =
  | "summary"
  | "tasks"
  | "analysis"
  | "literary"
  | "followup"
  | "agent"
  | "brief";

/// Контекст встречи для ИИ: заголовок, участники и имена говорящих.
export interface MeetingContext {
  title: string;
  participants: string;
  names: Record<string, string>;
  /// Скрывать нецензурную лексику в тексте для ИИ.
  censor?: boolean;
}

/// Статус локальной модели (экран «Модели» в настройках).
export interface ModelInfo {
  id: string;
  kind: "whisper" | "parakeet" | "diarize";
  installed: boolean;
  bytes: number;
}

/// Что ИИ делает сам после расшифровки (если ключ настроен).
export interface AiAutoConfig {
  /// Придумать заголовок/участников/тему.
  title: boolean;
  /// Сразу построить «Итоги встречи».
  summary: boolean;
  /// Следить за моделью на ИИ-сервере: если её обновили (сменилось имя) —
  /// переключаться на актуальную автоматически.
  followModel: boolean;
}

/// Результат проверки ИИ-сервера.
export interface AiCheck {
  ok: boolean;
  models: string[];
  /// Какую модель использовать (актуальная на сервере).
  model: string;
  /// Настроенной модели на сервере больше нет — выбрана другая.
  changed: boolean;
  latency_ms: number;
  error?: string | null;
}

export interface AppSettings {
  ai: AiConfig;
  whisper: WhisperConfig;
  /// Глобальная горячая клавиша старт/стоп записи (акселератор Tauri,
  /// напр. "Ctrl+Shift+R"). Пусто — хоткей выключен.
  hotkey: string;
  autoRecord: AutoRecordConfig;
  aiAuto: AiAutoConfig;
  /// Системные уведомления о старте/стоп записи. На macOS по умолчанию выкл —
  /// не просим лишнее разрешение.
  notifications: boolean;
  /// Нецензурная лексика: `censor` — скрывать пометкой «[нецензурно]»
  /// (по умолчанию), `verbatim` — дословно. Применяется при показе/экспорте/ИИ.
  profanity?: "censor" | "verbatim";
}

/// Разрешения macOS: `granted` / `denied` / `undetermined` / `unknown`.
export type PermStatus = "granted" | "denied" | "undetermined" | "unknown";
export interface MacPermissionsState {
  mic: PermStatus;
  system_audio: PermStatus;
  /// `audio` — «Только запись системного звука» (macOS 14.2+), `screen` —
  /// «Запись экрана и системного звука» (13–14.1).
  system_audio_mode: "audio" | "screen";
}

/// Состояние расшифровки одной встречи (живёт на уровне приложения, чтобы
/// переживать переход между встречами).
export interface TranscribeState {
  running: boolean;
  percent: number;
  stage?: string;
  /// Сколько фрагментов готово / всего (для счётчика прогресса).
  done?: number;
  total?: number;
  error?: string;
  /// Меняется при завершении — сигнал перезагрузить расшифровку.
  doneToken?: number;
}

/** Текущий уровень (пик) дорожек записи, 0..1000. */
export type TrackLevels = { mic: number; system: number };

/// Интервал дорожки в секундах — то, что вырезает аудио-редактор.
/// Имена полей совпадают с `edit::Range` в ядре.
export interface AudioRange {
  start_secs: number;
  end_secs: number;
}

/// Карта громкости дорожки для таймлайна: по корзине на пиксель.
/// `peaks`/`rms` — 0..1000, та же шкала, что у живых уровней записи.
export interface Waveform {
  peaks: number[];
  rms: number[];
  duration_secs: number;
  sample_rate: number;
}

/// Состояние аудио-редактора встречи: какие дорожки есть и сохранён ли
/// оригинал до правок (значит, правку можно отменить).
export interface AudioEditState {
  tracks: TrackFile[];
  has_original: boolean;
}
