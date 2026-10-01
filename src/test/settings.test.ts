import { describe, it, expect, beforeEach } from "vitest";
import { getSettings, saveSettings, isAiConfigured } from "../settings";

describe("settings", () => {
  beforeEach(() => localStorage.clear());

  it("returns defaults when empty", () => {
    const s = getSettings();
    expect(s.ai).toEqual({ base_url: "", api_key: "", model: "" });
    expect(s.whisper).toEqual({
      whisperPath: "",
      model: "parakeet-tdt-0.6b-v3",
      language: "ru",
    });
    expect(s.aiAuto).toEqual({ title: true, summary: true, followModel: true });
    expect(s.hotkey).toBe("Ctrl+Shift+R");
    expect(s.autoRecord).toEqual({
      enabled: false,
      apps: [],
      autoStop: true,
      startDelaySecs: 5,
      minKeepSecs: 12,
    });
  });

  it("round-trips saved settings", () => {
    saveSettings({
      ai: { base_url: "u", api_key: "k", model: "m" },
      whisper: { whisperPath: "p", model: "wm", language: "ru" },
      hotkey: "Alt+Shift+5",
      autoRecord: {
        enabled: true,
        apps: ["telegram"],
        autoStop: false,
        startDelaySecs: 5,
        minKeepSecs: 12,
      },
      aiAuto: { title: false, summary: true, followModel: true },
      notifications: true,
    });
    const s = getSettings();
    expect(s.whisper.model).toBe("wm");
    expect(s.aiAuto.title).toBe(false);
    expect(s.ai.base_url).toBe("u");
    expect(s.whisper.language).toBe("ru");
    expect(s.hotkey).toBe("Alt+Shift+5");
    expect(s.autoRecord.apps).toEqual(["telegram"]);
  });

  it("merges partial stored settings with defaults", () => {
    localStorage.setItem("3uxo.settings", JSON.stringify({ ai: { base_url: "x" } }));
    const s = getSettings();
    expect(s.ai.base_url).toBe("x");
    expect(s.ai.model).toBe("");
    expect(s.whisper.whisperPath).toBe("");
  });

  it("migrates the old default model once", () => {
    localStorage.setItem(
      "3uxo.settings",
      JSON.stringify({ whisper: { model: "medium", language: "ru" } }),
    );
    expect(getSettings().whisper.model).toBe("parakeet-tdt-0.6b-v3");
    // После сохранения в v0.8 явный выбор medium уважается.
    saveSettings({ ...getSettings(), whisper: { whisperPath: "", model: "medium", language: "ru" } });
    expect(getSettings().whisper.model).toBe("medium");
    // Нестандартный выбор не трогаем.
    localStorage.setItem("3uxo.settings", JSON.stringify({ whisper: { model: "small" } }));
    expect(getSettings().whisper.model).toBe("small");
  });

  it("falls back to defaults on malformed storage", () => {
    localStorage.setItem("3uxo.settings", "not json");
    expect(getSettings().ai.base_url).toBe("");
  });

  it("isAiConfigured reflects completeness", () => {
    expect(
      isAiConfigured({
        ai: { base_url: "u", api_key: "k", model: "m" },
        whisper: { whisperPath: "", model: "", language: "" },
        hotkey: "Ctrl+Shift+R",
        autoRecord: {
          enabled: false,
          apps: [],
          autoStop: true,
          startDelaySecs: 5,
          minKeepSecs: 12,
        },
        aiAuto: { title: true, summary: true, followModel: true },
        notifications: true,
      }),
    ).toBe(true);
    expect(isAiConfigured(getSettings())).toBe(false);
  });
});
