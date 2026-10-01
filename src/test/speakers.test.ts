import { describe, it, expect } from "vitest";
import { mergeSpeakers, renumberSpeakers, speakerStats, meetingContext } from "../speakers";
import type { Meeting, Transcript } from "../types";

const t: Transcript = {
  segments: [
    { speaker: "me", start_secs: 0, end_secs: 2, text: "привет" },
    { speaker: "spk0", start_secs: 2, end_secs: 8, text: "длинная реплика" },
    { speaker: "spk1", start_secs: 8, end_secs: 9, text: "да" },
    { speaker: "spk2", start_secs: 9, end_secs: 12, text: "третий" },
    { speaker: "spk0", start_secs: 12, end_secs: 13, text: "ещё" },
  ],
};

describe("speakers", () => {
  it("computes talk time, share and the longest sample", () => {
    const st = speakerStats(t);
    expect(st.map((s) => s.id)).toEqual(["me", "spk0", "spk1", "spk2"]);
    const s0 = st[1];
    expect(s0.secs).toBe(7);
    expect(s0.turns).toBe(2);
    expect(s0.sample.start).toBe(2);
    expect(st.reduce((a, s) => a + s.share, 0)).toBeCloseTo(1);
    expect(speakerStats(null)).toEqual([]);
  });

  it("merges a voice into another and renumbers without gaps", () => {
    const merged = mergeSpeakers(t, "spk1", "spk0");
    expect(merged.segments[2].speaker).toBe("spk0");
    const { transcript, labels } = renumberSpeakers(merged, {
      me: "Я сам",
      spk0: "Олег",
      spk1: "Лишний",
      spk2: "Анна",
    });
    expect(transcript.segments.map((s) => s.speaker)).toEqual([
      "me",
      "spk0",
      "spk0",
      "spk1",
      "spk0",
    ]);
    expect(labels).toEqual({ me: "Я сам", spk0: "Олег", spk1: "Анна" });
  });

  it("builds AI context with non-empty names only", () => {
    const m = { id: "x", title: "Т", participants: "П" } as Meeting;
    expect(meetingContext(m, { spk0: " Олег ", spk1: "  " })).toEqual({
      title: "Т",
      participants: "П",
      names: { spk0: "Олег" },
      censor: true,
    });
  });
});
