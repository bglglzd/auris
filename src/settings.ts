import type { AppSettings } from "./types";
import { defaultHotkey, isMac } from "./platform";

const KEY = "3uxo.settings";

/// Модель распознавания по умолчанию: NVIDIA Parakeet TDT 0.6B v3 — точный
/// русский с пунктуацией, в разы быстрее Whisper на обычном CPU и без
/// «галлюцинаций» на тишине. Для языков вне её 25 бэкенд сам берёт Whisper.
export const DEFAULT_MODEL = "parakeet-tdt-0.6b-v3";
/// Лучший Whisper (для языков, которых нет у Parakeet).
export const DEFAULT_WHISPER_MODEL = "large-v3-turbo-q8_0";

/// Версия схемы настроек. 2 — v0.8: новая модель по умолчанию + aiAuto.
const VERSION = 2;

const DEFAULTS: AppSettings = {
  ai: { base_url: "", api_key: "", model: "" },
  whisper: { whisperPath: "", model: DEFAULT_MODEL, language: "ru", vocabulary: "" },
  hotkey: defaultHotkey(),
  autoRecord: {
    enabled: false,
    apps: [],
    autoStop: true,
    startDelaySecs: 5,
    minKeepSecs: 12,
  },
  aiAuto: { title: true, summary: true, followModel: true, correct: true },
  notifications: !isMac,
  profanity: "censor",
};

export function getSettings(): AppSettings {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      const whisper = { ...DEFAULTS.whisper, ...(parsed.whisper ?? {}) };
      // До v0.8 модель по умолчанию была medium и сохранялась в настройки —
      // переводим на новую (лучше и быстрее). Явный выбор после v0.8 не трогаем.
      if ((parsed.version ?? 1) < 2 && (!whisper.model || whisper.model === "medium")) {
        whisper.model = DEFAULT_MODEL;
      }
      return {
        ai: { ...DEFAULTS.ai, ...(parsed.ai ?? {}) },
        whisper,
        hotkey: typeof parsed.hotkey === "string" ? parsed.hotkey : DEFAULTS.hotkey,
        autoRecord: { ...DEFAULTS.autoRecord, ...(parsed.autoRecord ?? {}) },
        aiAuto: { ...DEFAULTS.aiAuto, ...(parsed.aiAuto ?? {}) },
        notifications:
          typeof parsed.notifications === "boolean" ? parsed.notifications : DEFAULTS.notifications,
        profanity: parsed.profanity === "verbatim" ? "verbatim" : "censor",
      };
    }
  } catch {
    // malformed storage → defaults
  }
  return DEFAULTS;
}

/// Событие «настройки сохранены» — открытые экраны подхватывают изменения.
export const SETTINGS_EVENT = "memiro-settings";

export function saveSettings(settings: AppSettings): void {
  localStorage.setItem(KEY, JSON.stringify({ ...settings, version: VERSION }));
  if (typeof window !== "undefined") window.dispatchEvent(new Event(SETTINGS_EVENT));
}

/// true, если ИИ настроен достаточно для запросов. Модель можно не указывать —
/// тогда используется та, что сейчас отдаёт сервер.
export function isAiConfigured(s: AppSettings): boolean {
  return !!(s.ai.base_url && s.ai.api_key);
}

/// Языки, которые можно отметить в «Языки разговора» (пока два).
export const CONVERSATION_LANGUAGES: [string, string][] = [
  ["ru", "Русский"],
  ["en", "English"],
];
const CONV_CODES = CONVERSATION_LANGUAGES.map(([c]) => c);

/// Языки разговора: отмеченные пользователем, а если не отмечены — по
/// основному языку (русский → только русский; «определять автоматически» →
/// русский и английский). Основной язык вне списка (немецкий и т. п.) — без
/// ограничения (пусто).
export function conversationLanguages(w: AppSettings["whisper"]): string[] {
  const chosen = (w.languages ?? []).filter((l) => CONV_CODES.includes(l));
  if (chosen.length) return chosen;
  const main = w.language || "ru";
  if (main === "auto") return [...CONV_CODES];
  return CONV_CODES.includes(main) ? [main] : [];
}

/// Политика нецензурной лексики (по умолчанию — скрывать).
export function profanityPolicy(s: AppSettings = getSettings()): "censor" | "verbatim" {
  return s.profanity === "verbatim" ? "verbatim" : "censor";
}
