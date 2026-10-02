import type { Meeting, MeetingContext, Transcript } from "./types";
import type { SpeakerLabels } from "./labels";
import { profanityPolicy } from "./settings";

/// Сводка по одному голосу в расшифровке.
export interface SpeakerStat {
  id: string;
  /// Сколько секунд говорил.
  secs: number;
  /// Доля от всей речи, 0..1.
  share: number;
  /// Сколько реплик.
  turns: number;
  /// Самая длинная реплика — «образец голоса» для прослушивания.
  sample: { start: number; end: number; text: string };
}

/// Голоса в порядке первого появления, со временем речи и образцом.
export function speakerStats(transcript: Transcript | null): SpeakerStat[] {
  if (!transcript) return [];
  const map = new Map<string, SpeakerStat>();
  let total = 0;
  for (const s of transcript.segments) {
    const dur = Math.max(0, s.end_secs - s.start_secs);
    total += dur;
    let st = map.get(s.speaker);
    if (!st) {
      st = {
        id: s.speaker,
        secs: 0,
        share: 0,
        turns: 0,
        sample: { start: s.start_secs, end: s.end_secs, text: s.text },
      };
      map.set(s.speaker, st);
    }
    st.secs += dur;
    st.turns += 1;
    if (dur > st.sample.end - st.sample.start) {
      st.sample = { start: s.start_secs, end: s.end_secs, text: s.text };
    }
  }
  const out = Array.from(map.values());
  for (const st of out) st.share = total > 0 ? st.secs / total : 0;
  return out;
}

/// Переносит все реплики голоса `from` на голос `into` («это один человек»).
export function mergeSpeakers(t: Transcript, from: string, into: string): Transcript {
  return {
    segments: t.segments.map((s) => (s.speaker === from ? { ...s, speaker: into } : s)),
  };
}

/// Сколько реплик подряд, начиная с `index`, говорит тот же голос.
export function runLength(t: Transcript, index: number): number {
  const sp = t.segments[index]?.speaker;
  let n = 0;
  while (index + n < t.segments.length && t.segments[index + n].speaker === sp) n++;
  return n;
}

/// Меняет говорящего у реплики `index` (и, если `following`, у следующих
/// подряд реплик того же голоса) — ручная правка ошибок разделения голосов.
export function reassignSegment(t: Transcript, index: number, speaker: string, following = false): Transcript {
  const count = following ? runLength(t, index) : 1;
  return {
    segments: t.segments.map((s, j) => (j >= index && j < index + count ? { ...s, speaker } : s)),
  };
}

/// Id для нового голоса: следующий свободный `spkN`.
export function newSpeakerId(t: Transcript): string {
  let max = -1;
  for (const s of t.segments) {
    const m = /^spk(\d+)$/.exec(s.speaker);
    if (m) max = Math.max(max, Number(m[1]));
  }
  return `spk${max + 1}`;
}

/// Перенумеровывает `spkN` по порядку появления (после слияния не остаётся
/// «дыр» вроде Спикер 1, Спикер 3) и переносит подписи на новые номера.
export function renumberSpeakers(
  t: Transcript,
  labels: SpeakerLabels,
): { transcript: Transcript; labels: SpeakerLabels } {
  const map = new Map<string, string>();
  let next = 0;
  for (const s of t.segments) {
    if (/^spk\d+$/.test(s.speaker) && !map.has(s.speaker)) {
      map.set(s.speaker, `spk${next++}`);
    }
  }
  const transcript = {
    segments: t.segments.map((s) => {
      const to = map.get(s.speaker);
      return to ? { ...s, speaker: to } : s;
    }),
  };
  const out: SpeakerLabels = {};
  for (const [k, v] of Object.entries(labels)) {
    if (/^spk\d+$/.test(k)) {
      const to = map.get(k);
      if (to) out[to] = v;
    } else {
      out[k] = v;
    }
  }
  return { transcript, labels: out };
}

/// Контекст встречи для ИИ: заголовок, участники, имена голосов.
export function meetingContext(meeting: Meeting, labels: SpeakerLabels): MeetingContext {
  const names: Record<string, string> = {};
  for (const [k, v] of Object.entries(labels)) {
    if (v && v.trim()) names[k] = v.trim();
  }
  return { title: meeting.title, participants: meeting.participants, names, censor: profanityPolicy() === "censor" };
}
