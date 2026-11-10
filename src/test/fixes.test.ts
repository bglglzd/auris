import { beforeEach, describe, expect, it, vi } from "vitest";
import { applyFixes, applyFixesToTranscript, getFixes, setFixes, withMeetingTextWrite } from "../fixes";
import { api } from "../api";
import { invoke } from "@tauri-apps/api/core";
import { REPORT_ORDER } from "../reports";
import type { ReportKind, Transcript } from "../types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), convertFileSrc: vi.fn() }));
vi.mock("../log", () => ({ logInfo: vi.fn(), logError: vi.fn() }));
const call = vi.mocked(invoke);
const transcript: Transcript = { segments: [{ speaker: "spk0", start_secs: 0, end_secs: 1, text: "Термин", origin: "user" }] };
beforeEach(() => { localStorage.clear(); vi.clearAllMocks(); });

describe("meeting corrections", () => {
  it("uses whole Unicode words and literal replacement text", () => {
    expect(applyFixes("Термин термин ТЕРМИН терминал _термин термин_ e\u0301", [{ from: "термин", to: "$& $1 $` $'" }]))
      .toBe("$& $1 $` $' $& $1 $` $' $& $1 $` $' терминал _термин термин_ e\u0301");
    expect(applyFixes("C++ (C++) C++++", [{ from: "C++", to: "Rust" }])).toBe("Rust (Rust) Rust++");
    expect(applyFixes("e\u0301", [{ from: "e", to: "X" }])).toBe("e\u0301");
    expect(applyFixes("один два", [{ from: " ", to: "X" }, { from: "один", to: "" }])).toBe(" два");
  });

  it("retains transcript timing, speakers, origins and stored input", () => {
    const fixed = applyFixesToTranscript(transcript, [{ from: "термин", to: "Correct" }]);
    expect(fixed.segments[0]).toEqual({ ...transcript.segments[0], text: "Correct" });
    expect(transcript.segments[0].text).toBe("Термин");
  });

  it("isolates rules per meeting and rejects malformed storage", () => {
    setFixes("one", [{ from: "A", to: "B" }]);
    expect(getFixes("two")).toEqual([]);
    localStorage.setItem("3uxo.fixes.two", '[{"from":1,"to":"x"},{"from":" ","to":"x"},{"from":"A","to":"B"}]');
    expect(getFixes("two")).toEqual([{ from: "A", to: "B" }]);
    localStorage.setItem("3uxo.fixes.two", "invalid");
    expect(getFixes("two")).toEqual([]);
  });

  it("updates the transcript and every current/legacy report kind", async () => {
    const reports = Object.fromEntries(REPORT_ORDER.map((kind) => [kind, `Термин ${kind}`]));
    call.mockImplementation(async (command) => command === "get_transcript" ? structuredClone(transcript) : command === "get_reports" ? reports : undefined);
    const fix = [{ from: "термин", to: "$&" }];
    expect(await api.applyMeetingFixes("synthetic", fix)).toBe(8);
    expect(call).toHaveBeenCalledWith("save_transcript", { id: "synthetic", transcript: { segments: [{ ...transcript.segments[0], text: "$&" }] } });
    for (const kind of REPORT_ORDER) expect(call).toHaveBeenCalledWith("save_report", { id: "synthetic", kind, content: `$& ${kind}` });
    expect(getFixes("synthetic")).toEqual(fix);
  });

  it.each(REPORT_ORDER)("applies saved rules to newly generated %s reports", async (kind: ReportKind) => {
    setFixes("synthetic", [{ from: "термин", to: "$&" }]);
    call.mockImplementation(async (command) => command === "generate_report" ? `Термин ${kind}` : undefined);
    expect(await api.generateReport("synthetic", kind, { base_url: "", api_key: "", model: "" }, { title: "", participants: "", names: {} })).toBe(`$& ${kind}`);
    expect(call).toHaveBeenCalledWith("save_report", { id: "synthetic", kind, content: `$& ${kind}` });
  });

  it("applies saved rules to new transcription without changing segment metadata", async () => {
    setFixes("synthetic", [{ from: "термин", to: "Correct" }]);
    call.mockImplementation(async (command) => command === "transcribe" ? structuredClone(transcript) : undefined);
    expect((await api.transcribe("synthetic", { whisperPath: "", model: "", language: "ru", vocabulary: "" })).segments[0]).toEqual({ ...transcript.segments[0], text: "Correct" });
    expect(call).toHaveBeenCalledWith("save_transcript", { id: "synthetic", transcript: { segments: [{ ...transcript.segments[0], text: "Correct" }] } });
  });

  it("waits for report generation before reading and correcting its current saved text", async () => {
    let finish!: () => void;
    const gate = new Promise<void>((resolve) => { finish = resolve; });
    let content = "stale";
    call.mockImplementation(async (command, args) => {
      if (command === "generate_report") { await gate; content = "Термин fresh"; return content; }
      if (command === "get_transcript") return null;
      if (command === "get_reports") return { tasks: content };
      if (command === "save_report") { content = (args as { content: string }).content; }
    });
    const generating = api.generateReport("synthetic", "tasks", { base_url: "", api_key: "", model: "" }, { title: "", participants: "", names: {} });
    const fix = [{ from: "термин", to: "Correct" }];
    const fixing = api.applyMeetingFixes("synthetic", fix);
    await Promise.resolve();
    expect(call.mock.calls.some(([command]) => command === "get_reports")).toBe(false);
    finish(); await generating; await fixing;
    expect(content).toBe("Correct fresh");
  });

  it("surfaces partial write failures and leaves the per-meeting queue usable", async () => {
    const failure = new Error("synthetic disk error");
    call.mockImplementation(async (command) => {
      if (command === "get_transcript") return structuredClone(transcript);
      if (command === "get_reports") return { tasks: "Термин" };
      if (command === "save_report") throw failure;
    });
    const fix = [{ from: "термин", to: "Correct" }];
    await expect(api.applyMeetingFixes("synthetic", fix)).rejects.toBe(failure);
    expect(call.mock.calls.some(([command]) => command === "save_transcript")).toBe(true);
    expect(await withMeetingTextWrite("synthetic", async () => "usable")).toBe("usable");
    expect(await withMeetingTextWrite("other", async () => "independent")).toBe("independent");
  });

  it("merges unrelated stale-draft edits without undoing completed corrections", async () => {
    const base = { segments: [transcript.segments[0], { ...transcript.segments[0], start_secs: 1, end_secs: 2, text: "Second" }] };
    let stored = structuredClone(base);
    call.mockImplementation(async (command, args) => {
      if (command === "get_transcript") return structuredClone(stored);
      if (command === "get_reports") return {};
      if (command === "save_transcript") stored = structuredClone((args as { transcript: Transcript }).transcript);
    });
    await api.applyMeetingFixes("synthetic", [{ from: "Термин", to: "Correct" }]);
    const edited = structuredClone(base); edited.segments[1].text = "Manual second edit";
    const merged = await api.saveTranscriptEdits("synthetic", base, edited);
    expect(merged.segments.map((segment) => segment.text)).toEqual(["Correct", "Manual second edit"]);
    expect(stored).toEqual(merged);
  });

  it("rejects conflicting transcript/report edits without overwriting saved state", async () => {
    const current = applyFixesToTranscript(transcript, [{ from: "Термин", to: "Correct" }]);
    call.mockImplementation(async (command) => command === "get_transcript" ? current : command === "get_reports" ? { tasks: "Correct" } : undefined);
    const edited = applyFixesToTranscript(transcript, [{ from: "Термин", to: "Conflicting" }]);
    await expect(api.saveTranscriptEdits("synthetic", transcript, edited)).rejects.toThrow(/изменился/);
    await expect(api.saveReportEdit("synthetic", "tasks", "Термин", "Conflicting")).rejects.toThrow(/изменился/);
    expect(call.mock.calls.some(([command]) => command === "save_transcript" || command === "save_report")).toBe(false);
    const structural = { segments: [...current.segments, { ...current.segments[0], start_secs: 2, end_secs: 3 }] };
    call.mockResolvedValueOnce(structural);
    await expect(api.saveTranscriptEdits("synthetic", transcript, edited)).rejects.toThrow(/изменился/);
  });

  it("retains both rule upserts queued from independent stale UI instances", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const first = withMeetingTextWrite("synthetic", () => gate);
    call.mockImplementation(async (command) => command === "get_transcript" ? null : command === "get_reports" ? {} : undefined);
    const one = api.applyMeetingFixes("synthetic", [{ from: "Alpha", to: "Beta" }]);
    const two = api.applyMeetingFixes("synthetic", [{ from: "Gamma", to: "Delta" }]);
    release(); await first; await one; await two;
    expect(getFixes("synthetic")).toEqual([{ from: "Alpha", to: "Beta" }, { from: "Gamma", to: "Delta" }]);
    await api.removeMeetingFix("synthetic", "ALPHA");
    expect(getFixes("synthetic")).toEqual([{ from: "Gamma", to: "Delta" }]);
  });

  it("reapplies active rules after the supported AI post-transcription pass", async () => {
    setFixes("synthetic", [{ from: "Термин", to: "Correct" }]);
    call.mockImplementation(async (command) => command === "ai_correct_transcript" ? { changed: 1, rejected: 0 } : command === "get_transcript" ? structuredClone(transcript) : undefined);
    expect(await api.aiCorrectTranscript("synthetic", { base_url: "", api_key: "", model: "" })).toEqual({ changed: 1, rejected: 0 });
    expect(call).toHaveBeenCalledWith("save_transcript", { id: "synthetic", transcript: { segments: [{ ...transcript.segments[0], text: "Correct" }] } });
  });
});
