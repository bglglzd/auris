import { beforeEach, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MeetingCorrections } from "../components/MeetingCorrections";
import { api } from "../api";
import { getFixes, MEETING_TEXT_EVENT, setFixes } from "../fixes";
vi.mock("../api", () => ({ api: { applyMeetingFixes: vi.fn(), removeMeetingFix: vi.fn() } }));
beforeEach(() => { localStorage.clear(); vi.clearAllMocks(); });

it("applies an explicit rule through the UI and refreshes stored text", async () => {
  vi.mocked(api.applyMeetingFixes).mockImplementation(async (id, changes) => { setFixes(id, changes); return 8; });
  vi.mocked(api.removeMeetingFix).mockImplementation(async (id) => { setFixes(id, []); });
  const changed = vi.fn(); window.addEventListener(MEETING_TEXT_EVENT, changed);
  render(<MeetingCorrections id="synthetic" disabled={false} />);
  await userEvent.click(screen.getByText("Сквозные исправления"));
  await userEvent.type(screen.getByRole("textbox", { name: "Как написано" }), "Термин");
  await userEvent.type(screen.getByRole("textbox", { name: "Как правильно" }), "$&");
  await userEvent.click(screen.getByRole("button", { name: "Исправить везде" }));
  expect(await screen.findByRole("status")).toHaveTextContent("Сохранено текстов: 8");
  expect(api.applyMeetingFixes).toHaveBeenCalledWith("synthetic", [{ from: "Термин", to: "$&" }]);
  expect(changed).toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Убрать правило Термин" }));
  expect(getFixes("synthetic")).toEqual([]);
  expect(api.applyMeetingFixes).toHaveBeenCalledTimes(1);
  window.removeEventListener(MEETING_TEXT_EVENT, changed);
});

it("reports partial failure, reloads actual rules and enables retry", async () => {
  vi.mocked(api.applyMeetingFixes).mockRejectedValue(new Error("synthetic write failure"));
  render(<MeetingCorrections id="synthetic" disabled={false} />);
  await userEvent.click(screen.getByText("Сквозные исправления"));
  await userEvent.type(screen.getByRole("textbox", { name: "Как написано" }), "Термин");
  await userEvent.click(screen.getByRole("button", { name: "Исправить везде" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Часть текстов могла сохраниться");
  await waitFor(() => expect(screen.getByRole("button", { name: "Исправить везде" })).toBeEnabled());
});
