import { describe, it, expect, vi } from "vitest";

const { downloadAndInstall, relaunch } = vi.hoisted(() => ({
  relaunch: vi.fn(async () => {}),
  downloadAndInstall: vi.fn(async (cb: (e: unknown) => void) => {
  cb({ event: "Started", data: { contentLength: 200 } });
  cb({ event: "Progress", data: { chunkLength: 50 } });
  cb({ event: "Progress", data: { chunkLength: 150 } });
  cb({ event: "Finished" });
}),
}));
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn(async () => ({
    version: "0.9.0",
    currentVersion: "0.8.0",
    body: "## Что нового\n- пункт",
    downloadAndInstall,
  })),
}));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch }));

import { findUpdate, installUpdate } from "../updater";

describe("updater", () => {
  it("finds an update without installing it", async () => {
    const u = await findUpdate();
    expect(u?.version).toBe("0.9.0");
    expect(u?.notes).toBe("## Что нового\n- пункт");
    expect(downloadAndInstall).not.toHaveBeenCalled();
  });

  it("installs with progress and relaunches after consent", async () => {
    const u = (await findUpdate())!;
    const seen: number[] = [];
    await installUpdate(u, (p) => seen.push(p));
    expect(seen).toEqual([0, 25, 100, 100]);
    expect(relaunch).toHaveBeenCalled();
  });
});

import { notesForPlatform } from "../updater";

describe("notesForPlatform", () => {
  const notes = [
    "Новое для всех:",
    "• Быстрее расшифровка",
    "• [mac] Меньше разрешений на Mac",
    "• [win] Исправлен звук на Windows",
    "",
    "### macOS",
    "• Окно «Подготовка Mac»",
    "### Windows",
    "• Хоткей не конфликтует",
  ].join("\n");

  it("keeps common lines and only the Mac parts on macOS", () => {
    const m = notesForPlatform(notes, "macos");
    expect(m).toContain("Быстрее расшифровка");
    expect(m).toContain("• Меньше разрешений на Mac");
    expect(m).toContain("Подготовка Mac");
    expect(m).not.toMatch(/Windows|Хоткей|### macOS/);
  });

  it("keeps only the Windows parts on Windows", () => {
    const w = notesForPlatform(notes, "windows");
    expect(w).toContain("• Исправлен звук на Windows");
    expect(w).toContain("Хоткей не конфликтует");
    expect(w).not.toMatch(/Mac/);
  });

  it("non-platform headings stay common; empty result gets a generic line", () => {
    expect(notesForPlatform("## Что нового\n- общий пункт", "windows")).toBe("## Что нового\n- общий пункт");
    expect(notesForPlatform("### macOS\n- только мак", "windows")).toBe("Исправления и улучшения стабильности.");
    expect(notesForPlatform("", "windows")).toBe("");
  });
});
