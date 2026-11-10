import type { Transcript } from "./types";

export interface Fix { from: string; to: string }
const keyFor = (id: string) => `3uxo.fixes.${id}`;
export const MEETING_TEXT_EVENT = "memiro-meeting-text-updated";

export function getFixes(id: string): Fix[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(keyFor(id)) ?? "[]");
    return Array.isArray(value) ? value.filter((item): item is Fix =>
      item && typeof item.from === "string" && !!item.from.trim() && typeof item.to === "string",
    ) : [];
  } catch { return []; }
}

export function setFixes(id: string, fixes: Fix[]): void {
  localStorage.setItem(keyFor(id), JSON.stringify(fixes));
}

export function applyFixes(text: string, fixes: Fix[]): string {
  return fixes.reduce((out, { from, to }) => {
    const needle = from.trim();
    if (!needle) return out;
    const escaped = needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const re = new RegExp(`(?<![\\p{L}\\p{N}\\p{M}_])${escaped}(?![\\p{L}\\p{N}\\p{M}_])`, "giu");
    // Callback replacement keeps $&, $1, $` and $' literal.
    return out.replace(re, () => to);
  }, text);
}

export function applyFixesToTranscript(transcript: Transcript, fixes: Fix[]): Transcript {
  return { ...transcript, segments: transcript.segments.map((segment) => ({ ...segment, text: applyFixes(segment.text, fixes) })) };
}

// A generated report and an explicit correction must not overwrite each other
// from stale reads. Separate meetings can still run concurrently.
const writes = new Map<string, Promise<unknown>>();
export async function withMeetingTextWrite<T>(id: string, action: () => Promise<T>): Promise<T> {
  const previous = writes.get(id) ?? Promise.resolve();
  const pending = previous.catch(() => {}).then(action);
  writes.set(id, pending);
  try { return await pending; }
  finally { if (writes.get(id) === pending) writes.delete(id); }
}
