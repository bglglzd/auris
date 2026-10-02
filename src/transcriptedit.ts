/// Правка расшифровки прямо в ленте: текст реплики, пропуски (паузы в
/// тексте, где речь могла потеряться), вставка недостающих реплик.

import type { Transcript, TranscriptSegment } from "./types";

export interface Gap {
  /// Индекс реплики перед пропуском (−1 — пропуск в начале записи).
  after: number;
  start: number;
  end: number;
}

/// Промежутки без реплик длиннее `minGap` секунд — между репликами, в начале
/// и (если известна длительность) в конце записи.
export function findGaps(t: Transcript, minGap = 3, duration = 0): Gap[] {
  const segs = t.segments;
  const out: Gap[] = [];
  if (segs.length === 0) return out;
  if (segs[0].start_secs >= minGap) out.push({ after: -1, start: 0, end: segs[0].start_secs });
  let reach = segs[0].end_secs;
  for (let i = 1; i < segs.length; i++) {
    const s = segs[i];
    if (s.start_secs - reach >= minGap) out.push({ after: i - 1, start: reach, end: s.start_secs });
    reach = Math.max(reach, s.end_secs);
  }
  if (duration > 0 && duration - reach >= minGap) out.push({ after: segs.length - 1, start: reach, end: duration });
  return out;
}

/// Новый текст реплики (и, если задан, говорящий); пустой текст — удалить
/// реплику. Исправленная реплика помечается «edited» (дописанная остаётся
/// «user»).
export function setSegmentText(t: Transcript, index: number, text: string, speaker?: string): Transcript {
  if (!text.trim()) return { segments: t.segments.filter((_, j) => j !== index) };
  return {
    segments: t.segments.map((s, j) => {
      if (j !== index) return s;
      const changed = s.text.trim() !== text.trim();
      const origin = s.origin ?? (changed ? "edited" : undefined);
      return { ...s, text: text.trim(), speaker: speaker ?? s.speaker, ...(origin ? { origin } : {}) };
    }),
  };
}

/// Пустая реплика пользователя после реплики `index`: с её конца до
/// следующей (не дольше 5 с; если следующая вплотную — 3 с поверх).
export function newSegmentAfter(t: Transcript, index: number, duration = 0): TranscriptSegment {
  const s = t.segments[index];
  const next = t.segments[index + 1];
  // Следующая начинается вплотную (или раньше) — встаём чуть раньше неё,
  // чтобы новая реплика оказалась сразу под той, где нажали «＋».
  const start = next && next.start_secs <= s.end_secs ? Math.max(s.start_secs, next.start_secs - 0.01) : s.end_secs;
  let end = next && next.start_secs > start + 0.5 ? Math.min(next.start_secs, start + 5) : start + 3;
  if (duration > 0) end = Math.min(end, Math.max(duration, start + 0.5));
  return { speaker: s.speaker, start_secs: start, end_secs: end, text: "", origin: "user" };
}

/// Пустая реплика пользователя с момента `time` (кнопка в плеере): говорящий —
/// у реплики, звучащей в этот момент или перед ним.
export function newSegmentAt(t: Transcript, time: number, duration = 0): TranscriptSegment {
  const before = [...t.segments].reverse().find((s) => s.start_secs <= time);
  const speaker = before?.speaker ?? t.segments[0]?.speaker ?? "spk0";
  let end = time + 4;
  if (duration > 0) end = Math.min(end, Math.max(duration, time + 0.5));
  return { speaker, start_secs: time, end_secs: end, text: "", origin: "user" };
}

/// Вставляет реплики по времени (при равном начале — после имеющихся);
/// возвращает расшифровку и индекс первой вставленной.
export function insertSegments(t: Transcript, add: TranscriptSegment[]): { transcript: Transcript; index: number } {
  const segments = [...t.segments, ...add].sort((a, b) => a.start_secs - b.start_secs);
  const index = add.length ? segments.indexOf(add[0]) : -1;
  return { transcript: { segments }, index };
}

/// Говорящий для вставки в пропуск: у реплики перед ним, иначе — после.
export function speakerNear(t: Transcript, gap: Gap): string {
  return t.segments[gap.after]?.speaker ?? t.segments[gap.after + 1]?.speaker ?? "spk0";
}
