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

/// Новый текст реплики; пустой текст — удалить реплику.
export function setSegmentText(t: Transcript, index: number, text: string): Transcript {
  if (!text.trim()) return { segments: t.segments.filter((_, j) => j !== index) };
  return { segments: t.segments.map((s, j) => (j === index ? { ...s, text: text.trim() } : s)) };
}

/// Вставляет реплики по времени; возвращает расшифровку и индекс первой
/// вставленной.
export function insertSegments(t: Transcript, add: TranscriptSegment[]): { transcript: Transcript; index: number } {
  const segments = [...t.segments, ...add].sort((a, b) => a.start_secs - b.start_secs);
  const index = add.length ? segments.indexOf(add[0]) : -1;
  return { transcript: { segments }, index };
}

/// Говорящий для вставки в пропуск: у реплики перед ним, иначе — после.
export function speakerNear(t: Transcript, gap: Gap): string {
  return t.segments[gap.after]?.speaker ?? t.segments[gap.after + 1]?.speaker ?? "spk0";
}
