import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, beforeEach } from "vitest";

const openUrl = vi.fn(async (_url: string) => {});
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: (u: string) => openUrl(u) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(async () => null) }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn(async () => "0.11.0") }));
vi.mock("../api", () => ({
  api: {
    systemInfo: vi.fn(async () => ({ os: "windows", os_version: "Windows 11 (10.0.26100.1)", arch: "x86_64" })),
    getBackendLog: vi.fn(async () => String.raw`error at C:\Users\Ivan\AppData`),
    saveTextFile: vi.fn(async () => {}),
  },
}));

import { BugReportModal } from "../components/BugReportModal";

describe("BugReportModal", () => {
  beforeEach(() => {
    localStorage.clear();
    openUrl.mockClear();
  });

  it("sends a prefilled report to GitHub and remembers it", async () => {
    render(<BugReportModal prefill={{ title: "Ошибка во встрече", what: "model not found" }} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getAllByText(/Windows 11 \(10\.0\.26100\.1\) \(x86_64\)/).length).toBeGreaterThan(0));
    await userEvent.click(screen.getByRole("button", { name: "Отправить" }));
    expect(openUrl).toHaveBeenCalledTimes(1);
    const url = new URL(openUrl.mock.calls[0][0]);
    expect(url.searchParams.get("title")).toBe("[app] Ошибка во встрече");
    expect(url.searchParams.get("diagnostics")).toContain(String.raw`C:\Users\~`);
    expect(url.searchParams.get("diagnostics")).not.toContain("Ivan");
    expect(screen.getByText(/Спасибо/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: /Мои отчёты/ }));
    expect(screen.getByText("Ошибка во встрече")).toBeInTheDocument();
  });

  it("requires a description before sending", async () => {
    render(<BugReportModal onClose={vi.fn()} />);
    const send = screen.getByRole("button", { name: "Отправить" });
    expect(send).toBeDisabled();
    await userEvent.type(screen.getByLabelText(/Что случилось/), "Не работает запись");
    expect(send).toBeEnabled();
  });
});
