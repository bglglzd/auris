import { describe, it, expect } from "vitest";
import { findGaps, insertSegments, setSegmentText, speakerNear } from "../transcriptedit";

const t = {
  segments: [
    { speaker: "spk0", start_secs: 5, end_secs: 8, text: "а" },
    { speaker: "spk1", start_secs: 8.5, end_secs: 10, text: "б" },
    { speaker: "spk0", start_secs: 20, end_secs: 22, text: "в" },
  ],
};

describe("transcript edits", () => {
  it("finds gaps at the start, between phrases and at the end", () => {
    expect(findGaps(t, 3, 30)).toEqual([
      { after: -1, start: 0, end: 5 },
      { after: 1, start: 10, end: 20 },
      { after: 2, start: 22, end: 30 },
    ]);
    expect(findGaps(t, 3)).toHaveLength(2);
    expect(findGaps({ segments: [] })).toEqual([]);
  });
  it("edits, deletes and inserts phrases", () => {
    expect(setSegmentText(t, 1, " новое ").segments[1].text).toBe("новое");
    expect(setSegmentText(t, 1, "  ").segments).toHaveLength(2);
    const add = { speaker: "spk1", start_secs: 12, end_secs: 15, text: "пропущено" };
    const r = insertSegments(t, [add]);
    expect(r.index).toBe(2);
    expect(r.transcript.segments.map((s) => s.text)).toEqual(["а", "б", "пропущено", "в"]);
    expect(speakerNear(t, { after: 1, start: 10, end: 20 })).toBe("spk1");
    expect(speakerNear(t, { after: -1, start: 0, end: 5 })).toBe("spk0");
  });
});
