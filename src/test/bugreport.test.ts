import { describe, expect, it, beforeEach } from "vitest";
import {
  MAX_URL,
  emptyDraft,
  issueTitle,
  issueUrl,
  loadReports,
  rememberReport,
  reportMarkdown,
  sanitize,
  tailLines,
} from "../bugreport";

const env = { version: "0.11.0", platform: "macOS 15.1 (aarch64)" };

describe("bug reports", () => {
  beforeEach(() => localStorage.clear());

  it("removes personal data from logs", () => {
    const log = [
      String.raw`open C:\Users\Ivan Petrov\AppData\Roaming\com.3uxo.app\3uxo.db`,
      "path /Users/anna/Library/Application Support/com.3uxo.app",
      "home /home/oleg/.local/share",
      "key sk-abcdefghijklmnop1234 and Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.x",
      'api_key="secret-value" token: abc123xyz',
      "mail me at ivan.petrov@example.com",
    ].join("\n");
    const out = sanitize(log);
    expect(out).not.toMatch(/Ivan|anna|oleg|abcdefghijklmnop|eyJhbGci|secret-value|abc123xyz|example\.com/);
    expect(out).toContain(String.raw`C:\Users\~`);
    expect(out).toContain("/Users/~/Library");
    expect(out).toContain("sk-***");
  });

  it("keeps the newest log lines", () => {
    expect(tailLines("a\nb\nc\nd", 2)).toBe("c\nd");
    expect(tailLines("a\nb", 5)).toBe("a\nb");
  });

  it("builds a prefilled GitHub issue form", () => {
    const d = emptyDraft({ title: "Не расшифровывается", what: "Ошибка модели", steps: "1. Нажал" });
    const { url, truncated } = issueUrl(d, env, "лог");
    const u = new URL(url);
    expect(u.pathname).toBe("/bglglzd/auris/issues/new");
    expect(u.searchParams.get("template")).toBe("app_report.yml");
    expect(u.searchParams.get("title")).toBe("[app] Не расшифровывается");
    expect(u.searchParams.get("what")).toBe("Ошибка модели");
    expect(u.searchParams.get("version")).toBe("0.11.0");
    expect(u.searchParams.get("platform")).toBe("macOS 15.1 (aarch64)");
    expect(u.searchParams.get("impact")).toContain("Мешает");
    expect(u.searchParams.get("diagnostics")).toBe("лог");
    expect(truncated).toBe(false);
  });

  it("trims long diagnostics to fit the URL limit, newest lines kept", () => {
    const diag = Array.from({ length: 3000 }, (_, i) => `line ${i} ${"x".repeat(20)}`).join("\n");
    const { url, truncated } = issueUrl(emptyDraft({ what: "x" }), env, diag);
    expect(truncated).toBe(true);
    expect(url.length).toBeLessThanOrEqual(MAX_URL);
    expect(new URL(url).searchParams.get("diagnostics")).toContain("line 2999");
  });

  it("omits diagnostics when the user opts out", () => {
    const d = emptyDraft({ what: "x", includeDiagnostics: false });
    expect(new URL(issueUrl(d, env, "лог").url).searchParams.has("diagnostics")).toBe(false);
    expect(reportMarkdown(d, env, "лог")).not.toContain("Диагностика");
  });

  it("falls back to the first line of the description for the title", () => {
    expect(issueTitle(emptyDraft({ what: "Пропал звук\nподробности" }))).toBe("[app] Пропал звук");
  });

  it("remembers sent reports, newest first", () => {
    rememberReport({ title: "a", at: "2026-10-01T10:00:00Z", version: "0.11.0", via: "github" });
    rememberReport({ title: "b", at: "2026-10-01T11:00:00Z", version: "0.11.0", via: "copy" });
    expect(loadReports().map((r) => r.title)).toEqual(["b", "a"]);
  });
});
