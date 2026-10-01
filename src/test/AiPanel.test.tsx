import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { useState } from "react";
import { AiPanel } from "../components/AiPanel";
import type { Meeting, ReportKind } from "../types";
import { api } from "../api";

vi.mock("../settings", () => ({
  getSettings: () => ({
    ai: { base_url: "u", api_key: "k", model: "m" },
    whisper: { whisperPath: "", model: "", language: "" },
  }),
  isAiConfigured: () => true,
  profanityPolicy: () => "censor",
}));

vi.mock("../api", () => ({
  api: {
    generateReport: vi.fn(async (_id: string, kind: string) => `отчёт ${kind}`),
    suggestMeta: vi.fn(async () => ({ title: "T", participants: "P", topic: "Y" })),
    updateMeetingMeta: vi.fn(async () => {}),
    saveReport: vi.fn(async () => {}),
    askNamed: vi.fn(async () => "ответ"),
  },
}));

const meeting: Meeting = {
  id: "a",
  created_at: "2026-06-04T10:00:00Z",
  title: "t",
  participants: "",
  topic: "",
  duration_secs: 1,
  folder: "a",
  status: "transcribed",
};

/// Обёртка с состоянием отчётов, как в MeetingView.
function Host({ onMetaSaved = vi.fn(), hasTranscript = true }) {
  const [reports, setReports] = useState<Partial<Record<ReportKind, string>>>({});
  return (
    <AiPanel
      meeting={meeting}
      labels={{ spk0: "Олег" }}
      hasTranscript={hasTranscript}
      reports={reports}
      onReport={(k, t) => setReports((r) => ({ ...r, [k]: t }))}
      onMetaSaved={onMetaSaved}
    />
  );
}

describe("AiPanel", () => {
  beforeEach(() => vi.clearAllMocks());

  it.each([
    ["Итоги встречи", "summary"],
    ["Задачи", "tasks"],
    ["Разбор разговора", "analysis"],
    ["Чистовой текст", "literary"],
    ["Письмо по итогам", "followup"],
    ["Инструкция для ИИ", "agent"],
  ])("creates «%s»", async (title, kind) => {
    render(<Host />);
    await userEvent.click(screen.getByRole("button", { name: new RegExp(title) }));
    expect(await screen.findByText(`отчёт ${kind}`)).toBeInTheDocument();
    expect(api.generateReport).toHaveBeenCalledWith(
      "a",
      kind,
      expect.anything(),
      expect.objectContaining({ names: { spk0: "Олег" } }),
    );
  });

  it("explains every preset (no duplicates)", () => {
    render(<Host />);
    expect(screen.getByText(/кто отвечает/)).toBeInTheDocument();
    expect(screen.getByText(/Готовое письмо/)).toBeInTheDocument();
    expect(screen.queryByText("Выжимка")).toBeNull();
  });

  it("suggests metadata and saves it", async () => {
    const onMetaSaved = vi.fn();
    render(<Host onMetaSaved={onMetaSaved} />);
    await userEvent.click(screen.getByRole("button", { name: /Заголовок/i }));
    expect(api.suggestMeta).toHaveBeenCalled();
    expect(api.updateMeetingMeta).toHaveBeenCalledWith("a", "T", "P", "Y");
    expect(onMetaSaved).toHaveBeenCalled();
  });

  it("asks to transcribe first when there is no transcript", async () => {
    render(<Host hasTranscript={false} />);
    await userEvent.click(screen.getByRole("button", { name: /Итоги встречи/ }));
    expect(await screen.findByText(/Сначала расшифруйте/)).toBeInTheDocument();
    expect(api.generateReport).not.toHaveBeenCalled();
  });

  it("answers a question", async () => {
    render(<Host />);
    await userEvent.type(screen.getByPlaceholderText(/Спросить/i), "вопрос?");
    await userEvent.click(screen.getByRole("button", { name: /^Спросить$/i }));
    expect(await screen.findByText("ответ")).toBeInTheDocument();
  });
});
