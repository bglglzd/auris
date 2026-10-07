import { describe, expect, it } from "vitest";
import { groupMeetings, mergeOrder, toggleSelected } from "../library";
import type { Meeting } from "../types";

const m = (id: string, at: string, collection = ""): Meeting => ({
  id,
  created_at: at,
  title: id,
  participants: "",
  topic: "",
  duration_secs: 60,
  folder: id,
  status: "recorded",
  collection,
});

describe("library", () => {
  const meetings = [m("c", "2026-10-03"), m("b", "2026-10-02", "f1"), m("a", "2026-10-01", "gone")];
  const cols = [
    { id: "f2", name: "Юристы", created_at: "" },
    { id: "f1", name: "Клиент", created_at: "" },
  ];

  it("groups meetings by folders, folders by name", () => {
    const g = groupMeetings(meetings, cols);
    expect(g.folders.map((f) => f.collection.name)).toEqual(["Клиент", "Юристы"]);
    expect(g.folders[0].meetings.map((x) => x.id)).toEqual(["b"]);
    expect(g.folders[1].meetings).toEqual([]);
    // Папки больше нет — встреча вне папок.
    expect(g.loose.map((x) => x.id)).toEqual(["c", "a"]);
  });

  it("merges in click order or by time", () => {
    let sel: string[] = [];
    sel = toggleSelected(sel, "c");
    sel = toggleSelected(sel, "a");
    sel = toggleSelected(sel, "b");
    expect(sel).toEqual(["c", "a", "b"]);
    expect(mergeOrder(sel, meetings, false)).toEqual(["c", "a", "b"]);
    expect(mergeOrder(sel, meetings, true)).toEqual(["a", "b", "c"]);
    expect(toggleSelected(sel, "a")).toEqual(["c", "b"]);
  });
});
