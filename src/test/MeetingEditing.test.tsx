import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { MeetingView } from "../components/MeetingView";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), convertFileSrc: (path: string) => path }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(async () => null) }));
vi.mock("../log", () => ({ logInfo: vi.fn(), logError: vi.fn() }));
vi.mock("../components/AiPanel", () => ({ AiPanel: () => null }));
vi.mock("../components/AudioEditor", () => ({ AudioEditor: () => null }));
vi.mock("../components/ExportModal", () => ({ ExportModal: () => null }));
vi.mock("../components/SpeakersPanel", () => ({ SpeakersPanel: () => null }));
vi.mock("../components/CopyLogButton", () => ({ CopyLogButton: () => null }));

const call = vi.mocked(invoke);
const segment = (text: string, at = 0) => ({ speaker: "spk0", start_secs: at, end_secs: at + 1, text });
const meeting = (id: string) => ({ id, created_at: "2026-01-01T00:00:00Z", title: "Synthetic meeting", participants: "", topic: "", duration_secs: 4, folder: id, status: "transcribed", source: "recorded", notes: "", collection: "" });

beforeEach(() => { localStorage.clear(); call.mockReset(); });
afterEach(cleanup);

// These are synthetic IPC stores. Production API queue, rule storage and UI
// handlers are actual candidate code; no native process/model is invoked.

it("saves a newly inserted manual phrase through the actual meeting UI", async () => {
  const id = "new-phrase";
  let transcript = { segments: [segment("Alpha"), segment("Other", 2)] };
  let saves = 0;
  call.mockImplementation(async (command, args: any) => {
    if (command === "get_transcript") return structuredClone(transcript);
    if (command === "get_reports") return {};
    if (command === "save_transcript") { saves++; transcript = structuredClone(args.transcript); }
    if (command === "refine_pending") return { running: false, pending: 0 };
    if (command === "has_voice_analysis" || command === "ai_correction_available") return false;
    if (command === "track_path") return "synthetic.wav";
  });
  render(<MeetingView meeting={meeting(id)} onTranscribe={vi.fn()} onMetaSaved={vi.fn()} />);
  await screen.findByRole("button", { name: "Редактировать расшифровку" });
  await userEvent.click(screen.getAllByRole("button", { name: "Добавить реплику после этой" })[0]);
  const input = screen.getByPlaceholderText("Впишите, что слышно в записи…");
  await userEvent.type(input, "New manual phrase");
  await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
  await waitFor(() => expect(transcript.segments.map(segment => segment.text)).toContain("New manual phrase"));
  expect(saves).toBe(1);
  expect(transcript.segments.map(s => s.text)).toContain("New manual phrase");
});

it("rejects an inline draft whose original text changed after editing opened", async () => {
  const id = "inline-base";
  let transcript = { segments: [segment("Alpha"), segment("Other", 2)] };
  let saves = 0;
  call.mockImplementation(async (command, args: any) => {
    if (command === "get_transcript") return structuredClone(transcript);
    if (command === "get_reports") return {};
    if (command === "save_transcript") { saves++; transcript = structuredClone(args.transcript); }
    if (command === "refine_pending") return { running: false, pending: 0 };
    if (command === "has_voice_analysis" || command === "ai_correction_available") return false;
    if (command === "track_path") return "synthetic.wav";
  });
  render(<MeetingView meeting={meeting(id)} onTranscribe={vi.fn()} onMetaSaved={vi.fn()} />);
  await screen.findByRole("button", { name: "Редактировать расшифровку" });
  await userEvent.click(screen.getAllByRole("button", { name: "Исправить текст реплики" })[0]);
  await userEvent.clear(screen.getByDisplayValue("Alpha"));
  await userEvent.type(screen.getByPlaceholderText("Что было сказано…"), "Manual conflicting");
  await userEvent.click(screen.getByText("Сквозные исправления"));
  await userEvent.type(screen.getByRole("textbox", { name: "Как написано" }), "Alpha");
  await userEvent.type(screen.getByRole("textbox", { name: "Как правильно" }), "Correct");
  await userEvent.click(screen.getByRole("button", { name: "Исправить везде" }));
  await screen.findByRole("status");
  // Flush the actual MeetingView refresh triggered by MEETING_TEXT_EVENT.
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
  expect(transcript.segments[0].text).toBe("Correct");
  const beforeSave = saves;
  await userEvent.click(screen.getByRole("button", { name: "Сохранить" }));
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); });
  expect(transcript.segments[0].text).toBe("Correct");
  expect(saves).toBe(beforeSave);
  expect(screen.getByDisplayValue("Manual conflicting")).toBeTruthy();
});

it("preserves a pending correction while changing only the speaker", async () => {
  const id = "speaker-during-correction";
  let transcript = { segments: [segment("Alpha"), segment("Other", 2)] };
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  let saves = 0;
  call.mockImplementation(async (command, args: any) => {
    if (command === "get_transcript") return structuredClone(transcript);
    if (command === "get_reports") return {};
    if (command === "save_transcript") { saves++; if (saves === 1) await gate; transcript = structuredClone(args.transcript); }
    if (command === "refine_pending") return { running: false, pending: 0 };
    if (command === "has_voice_analysis" || command === "ai_correction_available") return false;
    if (command === "track_path") return "synthetic.wav";
  });
  render(<MeetingView meeting={meeting(id)} onTranscribe={vi.fn()} onMetaSaved={vi.fn()} />);
  await screen.findByRole("button", { name: "Редактировать расшифровку" });
  await userEvent.click(screen.getByText("Сквозные исправления"));
  await userEvent.type(screen.getByRole("textbox", { name: "Как написано" }), "Alpha");
  await userEvent.type(screen.getByRole("textbox", { name: "Как правильно" }), "Correct");
  await userEvent.click(screen.getByRole("button", { name: "Исправить везде" }));
  await waitFor(() => expect(saves).toBe(1));
  await userEvent.click(screen.getAllByTitle("Сменить говорящего")[0]);
  await userEvent.click(screen.getByRole("menuitem", { name: "＋ Новый голос" }));
  await act(async () => { release(); });
  await screen.findByRole("status");
  await waitFor(() => expect(saves).toBe(2));
  expect(transcript.segments[0].speaker).not.toBe("spk0");
  expect(transcript.segments[0].text).toBe("Correct");
});
