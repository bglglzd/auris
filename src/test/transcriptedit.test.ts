import { describe, it, expect } from "vitest";
import { findGaps, insertSegments, newSegmentAfter, newSegmentAt, setSegmentText, speakerNear } from "../transcriptedit";

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

  it("marks user phrases and places new bubbles", () => {
    expect(setSegmentText(t, 0, "исправил").segments[0].origin).toBe("edited");
    expect(setSegmentText(t, 0, "а").segments[0].origin).toBeUndefined();
    expect(setSegmentText(t, 0, "а", "spk1").segments[0].speaker).toBe("spk1");
    const user = { ...t.segments[0], text: "", origin: "user" as const };
    expect(setSegmentText({ segments: [user] }, 0, "дописал").segments[0].origin).toBe("user");
    // После реплики: до следующей, если она не вплотную; иначе 3 с поверх.
    expect(newSegmentAfter(t, 0)).toMatchObject({ speaker: "spk0", start_secs: 8, end_secs: 11, origin: "user" });
    expect(newSegmentAfter(t, 1)).toMatchObject({ start_secs: 10, end_secs: 15 });
    expect(newSegmentAfter(t, 2, 23)).toMatchObject({ start_secs: 22, end_secs: 23 });
    expect(newSegmentAt(t, 9, 30)).toMatchObject({ speaker: "spk1", start_secs: 9, end_secs: 13 });
    // Вставка после реплики с тем же началом встаёт после неё.
    const r = insertSegments(t, [newSegmentAfter(t, 0)]);
    expect(r.index).toBe(1);
    // Следующая вплотную: новая всё равно сразу после нажатой.
    const tight = { segments: [t.segments[0], { ...t.segments[1], start_secs: 8 }] };
    expect(insertSegments(tight, [newSegmentAfter(tight, 0)]).index).toBe(1);
  });
});
