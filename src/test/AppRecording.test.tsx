import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";

const fixture = vi.hoisted(() => ({
  backend: { recording: true, paused: true },
  listeners: new Map<string, (event: { payload: string }) => void>(),
  stop: vi.fn(),
  pause: vi.fn(),
  resume: vi.fn(),
  state: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event, callback) => {
    fixture.listeners.set(event, callback);
    return () => fixture.listeners.delete(event);
  }),
}));
vi.mock("../api", () => ({
  api: {
    listMeetings: async () => [],
    listCollections: async () => [],
    recordingState: fixture.state,
    recordingLevels: async () => ({ mic: 0, system: 0 }),
    stopRecording: fixture.stop,
    pauseRecording: fixture.pause,
    resumeRecording: fixture.resume,
    updateHotkey: async () => {},
    setNotifications: async () => {},
    setAutorecord: async () => {},
  },
}));
vi.mock("../updater", () => ({
  findUpdate: async () => null,
  UPDATE_INTERVAL_MS: 3600000,
}));
vi.mock("../aimodel", () => ({
  syncServerModel: async () => {},
  AI_MODEL_EVENT: "synthetic-model-event",
}));
vi.mock("../aiauto", () => ({
  runAutoAi: async () => false,
  runAutoCorrect: async () => {},
}));
vi.mock("../appmenu", () => ({ useAppMenu: () => {} }));
vi.mock("../components/Sidebar", async () => {
  const { RecordButton } = await import("../components/RecordButton");
  return {
    Sidebar: (props: ComponentProps<typeof RecordButton>) => (
      <RecordButton {...props} />
    ),
  };
});
vi.mock("../components/MeetingView", () => ({ MeetingView: () => null }));
vi.mock("../components/RecordingMonitor", () => ({
  RecordingMonitor: () => null,
}));
vi.mock("../components/SettingsModal", () => ({ SettingsModal: () => null }));
vi.mock("../components/ImportModal", () => ({ ImportModal: () => null }));
vi.mock("../components/UpdateDialog", () => ({ UpdateDialog: () => null }));
vi.mock("../components/BugReportModal", () => ({ BugReportModal: () => null }));
vi.mock("../components/MacSetup", () => ({
  MacSetup: () => null,
  macSetupPending: () => false,
}));

import App from "../App";

// Real App handlers and record controls; synthetic IPC and unrelated screens.
describe("recording error recovery", () => {
  beforeEach(() => {
    localStorage.clear();
    fixture.listeners.clear();
    fixture.backend = { recording: true, paused: true };
    fixture.stop.mockReset();
    fixture.pause.mockReset();
    fixture.resume.mockReset();
    fixture.state
      .mockReset()
      .mockImplementation(async () => ({ ...fixture.backend }));
  });
  afterEach(cleanup);

  it("resets controls and reports a save failure after the backend has stopped", async () => {
    fixture.stop.mockImplementation(async () => {
      fixture.backend = { recording: false, paused: false };
      throw new Error("synthetic database failure");
    });
    render(<App />);
    const stop = await screen.findByRole("button", {
      name: /На паузе — завершить/,
    });
    const previousReads = fixture.state.mock.calls.length;
    await userEvent.click(stop);
    await screen.findByRole("button", { name: "Начать запись" });
    expect(screen.getByRole("status")).toHaveTextContent(
      "synthetic database failure",
    );
    expect(fixture.state.mock.calls.length).toBeGreaterThan(previousReads);
  });

  it("keeps live backend recording controls when a failed RPC never stopped it", async () => {
    fixture.backend.paused = false;
    fixture.stop.mockRejectedValue(new Error("synthetic transport failure"));
    render(<App />);
    await userEvent.click(
      await screen.findByRole("button", { name: /Завершить запись/ }),
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        "synthetic transport failure",
      ),
    );
    expect(
      screen.getByRole("button", { name: /Завершить запись/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Начать запись" }),
    ).not.toBeInTheDocument();
  });

  it("leaves a failed resume paused and shows its error", async () => {
    fixture.resume.mockRejectedValue(new Error("synthetic device failure"));
    render(<App />);
    await userEvent.click(
      await screen.findByRole("button", { name: /Продолжить/ }),
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        "synthetic device failure",
      ),
    );
    expect(
      screen.getByRole("button", { name: /На паузе — завершить/ }),
    ).toBeInTheDocument();
  });

  it("keeps stop controls when both the stop RPC and state refresh are unavailable", async () => {
    fixture.backend.paused = false;
    fixture.stop.mockRejectedValue(new Error("synthetic unavailable backend"));
    fixture.state
      .mockResolvedValueOnce({ ...fixture.backend })
      .mockRejectedValue(new Error("synthetic state failure"));
    render(<App />);
    await userEvent.click(
      await screen.findByRole("button", { name: /Завершить запись/ }),
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        "synthetic unavailable backend",
      ),
    );
    expect(
      screen.getByRole("button", { name: /Завершить запись/ }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Начать запись" }),
    ).not.toBeInTheDocument();
  });

  it("resynchronizes controls after a hotkey or automatic-stop error event", async () => {
    render(<App />);
    await screen.findByRole("button", { name: /На паузе — завершить/ });
    fixture.backend = { recording: false, paused: false };
    await act(async () => {
      fixture.listeners.get("recording-error")!({
        payload: "synthetic stop event",
      });
    });
    await screen.findByRole("button", { name: "Начать запись" });
    expect(screen.getByRole("status")).toHaveTextContent(
      "synthetic stop event",
    );
  });

  it("shows capture warnings without assuming which track failed", async () => {
    render(<App />);
    await screen.findByRole("button", { name: /На паузе — завершить/ });
    await act(async () => {
      fixture.listeners.get("recording-capture-warning")!({
        payload: "synthetic microphone warning",
      });
    });
    expect(screen.getByRole("status")).toHaveTextContent(
      "synthetic microphone warning",
    );
    expect(
      screen.queryByText(/голоса собеседников не записываются/),
    ).not.toBeInTheDocument();
  });
});
