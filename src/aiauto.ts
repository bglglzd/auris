import type { Meeting } from "./types";
import { api } from "./api";
import { getSettings, isAiConfigured } from "./settings";
import { getLabels } from "./labels";
import { meetingContext } from "./speakers";
import { AI_AUTO_EVENT, REPORTS_EVENT } from "./reports";
import type { AiAutoDetail } from "./reports";
import { logError } from "./log";

function signal(detail: AiAutoDetail) {
  window.dispatchEvent(new CustomEvent(AI_AUTO_EVENT, { detail }));
}

/// Можно ли заменить заголовок автоматически: пользователь его не правил и
/// авто-заголовок ещё не ставился.
export function titleIsAuto(meetingId: string): boolean {
  return (
    localStorage.getItem(`3uxo.titleEdited.${meetingId}`) !== "1" &&
    localStorage.getItem(`3uxo.autotitle.${meetingId}`) !== "1"
  );
}

/// ИИ после расшифровки (если ключ настроен): заголовок/участники/тема и
/// «Итоги встречи». Всё в фоне; ошибки не мешают расшифровке. Возвращает
/// true, если метаданные встречи изменились.
export async function runAutoAi(meeting: Meeting): Promise<boolean> {
  const s = getSettings();
  if (!isAiConfigured(s)) return false;
  const wantTitle = s.aiAuto.title && titleIsAuto(meeting.id);
  let wantSummary = s.aiAuto.summary;
  if (!wantTitle && !wantSummary) return false;

  const ctx = meetingContext(meeting, getLabels(meeting.id));
  let changed = false;
  try {
    if (wantTitle) {
      signal({ id: meeting.id, busy: true, label: "Придумываю заголовок…" });
      const m = await api.suggestMeta(meeting.id, s.ai, ctx);
      if (m.title) {
        await api.updateMeetingMeta(
          meeting.id,
          m.title,
          meeting.participants || m.participants,
          meeting.topic || m.topic,
        );
        localStorage.setItem(`3uxo.autotitle.${meeting.id}`, "1");
        ctx.title = m.title;
        if (!ctx.participants) ctx.participants = m.participants;
        changed = true;
      }
    }
    if (wantSummary) {
      const existing = await api.getReports(meeting.id).catch(() => ({}));
      wantSummary = !("summary" in existing);
    }
    if (wantSummary) {
      signal({ id: meeting.id, busy: true, label: "Подвожу итоги встречи…" });
      await api.generateReport(meeting.id, "summary", s.ai, ctx);
      window.dispatchEvent(new CustomEvent(REPORTS_EVENT, { detail: { id: meeting.id } }));
    }
    signal({ id: meeting.id, busy: false });
  } catch (e) {
    logError("auto ai", e);
    signal({ id: meeting.id, busy: false, error: `ИИ после расшифровки: ${String(e)}` });
  }
  return changed;
}

/// Событие «расшифровка встречи изменилась» (ИИ-корректура) — detail: { id, changed }.
export const TRANSCRIPT_EVENT = "memiro-transcript-changed";

/// ИИ-корректура расшифровки (только текст, через ключ пользователя):
/// исправляет неверно услышанные слова, термины и пунктуацию. Запускается
/// после фонового уточнения трудных мест (иначе они мешали бы друг другу).
/// Возвращает число исправленных реплик (или null, если не запускалась).
export async function runAutoCorrect(meetingId: string, force = false): Promise<number | null> {
  const s = getSettings();
  if (!isAiConfigured(s)) return null;
  if (!force && s.aiAuto.correct === false) return null;
  signal({ id: meetingId, busy: true, label: "Исправляю ошибки распознавания…" });
  try {
    const glossary = [s.whisper.vocabulary, s.whisper.learned]
      .map((v) => v?.trim())
      .filter(Boolean)
      .join("\n");
    const r = await api.aiCorrectTranscript(meetingId, s.ai, glossary);
    signal({ id: meetingId, busy: false });
    window.dispatchEvent(new CustomEvent(TRANSCRIPT_EVENT, { detail: { id: meetingId, changed: r.changed } }));
    return r.changed;
  } catch (e) {
    logError("ai correct", e);
    signal({ id: meetingId, busy: false, error: `ИИ-правка расшифровки: ${String(e)}` });
    return null;
  }
}
