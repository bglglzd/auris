/// Отчёты об ошибках из приложения → GitHub Issues (форма app_report.yml).
///
/// Отправка без ключей и серверов: приложение собирает отчёт и открывает в
/// браузере заполненную форму issue — пользователь проверяет текст и жмёт
/// «Submit». Без аккаунта GitHub отчёт можно скопировать или сохранить файлом.
/// Диагностика очищается от личного: путей с именем пользователя, ключей API,
/// токенов, e-mail. Текст встреч в отчёт не попадает.

export const ISSUES_REPO = "https://github.com/bglglzd/auris";
export const REPORT_TEMPLATE = "app_report.yml";
/// GitHub отвечает 414 на очень длинные ссылки — держим запас.
export const MAX_URL = 7600;

export type Impact = "blocks" | "annoys" | "minor";
export type Frequency = "always" | "sometimes" | "once";

export const IMPACT_LABEL: Record<Impact, string> = {
  blocks: "Не могу работать",
  annoys: "Мешает, но можно обойти",
  minor: "Мелочь",
};
export const FREQUENCY_LABEL: Record<Frequency, string> = {
  always: "каждый раз",
  sometimes: "иногда",
  once: "один раз",
};

export interface BugDraft {
  title: string;
  what: string;
  steps: string;
  expected: string;
  impact: Impact;
  frequency: Frequency;
  includeDiagnostics: boolean;
}

export interface BugEnv {
  version: string;
  /// Человекочитаемая система: «macOS 15.1 (aarch64)».
  platform: string;
}

export function emptyDraft(prefill?: Partial<BugDraft>): BugDraft {
  return {
    title: "",
    what: "",
    steps: "",
    expected: "",
    impact: "annoys",
    frequency: "sometimes",
    includeDiagnostics: true,
    ...prefill,
  };
}

/// Убирает личное из лога: домашние папки (имя пользователя), ключи API,
/// токены, e-mail.
export function sanitize(text: string): string {
  return text
    .replace(/([A-Za-z]:\\+Users\\+)[^\\/\s"']+/gi, "$1~")
    .replace(/(\/Users\/|\/home\/)[^/\s"']+/g, "$1~")
    .replace(/\bsk-[A-Za-z0-9_-]{8,}/g, "sk-***")
    .replace(/(bearer\s+)[A-Za-z0-9._~+/=-]{8,}/gi, "$1***")
    .replace(/((?:api[_-]?key|token|secret|password)["'\s:=]+)[^\s"',}]+/gi, "$1***")
    .replace(/[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g, "***@***");
}

/// Последние `maxLines` строк (свежие ошибки — в конце лога).
export function tailLines(text: string, maxLines: number): string {
  const lines = text.split("\n");
  return lines.length <= maxLines ? text : lines.slice(-maxLines).join("\n");
}

export function issueTitle(d: BugDraft): string {
  const t = (d.title.trim() || d.what.trim().split("\n")[0] || "Ошибка").slice(0, 120);
  return `[app] ${t}`;
}

export function impactLine(d: BugDraft): string {
  return `${IMPACT_LABEL[d.impact]} · ${FREQUENCY_LABEL[d.frequency]}`;
}

/// Полный отчёт в Markdown — для копирования и файла.
export function reportMarkdown(d: BugDraft, env: BugEnv, diagnostics: string): string {
  const parts = [
    `# ${issueTitle(d)}`,
    "",
    `**Версия:** Memiro AI ${env.version}  `,
    `**Система:** ${env.platform}  `,
    `**Насколько мешает:** ${impactLine(d)}`,
    "",
    "## Что случилось",
    d.what.trim() || "—",
  ];
  if (d.steps.trim()) parts.push("", "## Как воспроизвести", d.steps.trim());
  if (d.expected.trim()) parts.push("", "## Что ожидалось", d.expected.trim());
  if (d.includeDiagnostics && diagnostics.trim()) {
    parts.push("", "## Диагностика", "```text", diagnostics.trim(), "```");
  }
  return parts.join("\n") + "\n";
}

/// Ссылка на заполненную форму issue. Диагностика урезается с начала (старые
/// строки), пока ссылка не уложится в лимит; `truncated` — урезали ли.
export function issueUrl(
  d: BugDraft,
  env: BugEnv,
  diagnostics: string,
): { url: string; truncated: boolean } {
  const build = (diag: string) => {
    const q = new URLSearchParams({
      template: REPORT_TEMPLATE,
      title: issueTitle(d),
      what: d.what.trim(),
      steps: d.steps.trim(),
      expected: d.expected.trim(),
      impact: impactLine(d),
      version: env.version,
      platform: env.platform,
    });
    if (diag) q.set("diagnostics", diag);
    return `${ISSUES_REPO}/issues/new?${q.toString()}`;
  };
  let diag = d.includeDiagnostics ? diagnostics.trim() : "";
  let url = build(diag);
  let truncated = false;
  while (url.length > MAX_URL && diag) {
    truncated = true;
    const lines = diag.split("\n");
    diag = lines.length > 4 ? ["…", ...lines.slice(Math.ceil(lines.length / 4))].join("\n") : "";
    url = build(diag);
  }
  return { url, truncated };
}

// ---------- «Мои отчёты» (локально, только у пользователя) ----------

export interface SentReport {
  title: string;
  at: string;
  version: string;
  via: "github" | "copy" | "file";
}

const KEY = "3uxo.bugreports";
const MAX_HISTORY = 50;

export function loadReports(): SentReport[] {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || "[]");
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

export function rememberReport(r: SentReport): SentReport[] {
  const list = [r, ...loadReports()].slice(0, MAX_HISTORY);
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    /* хранилище недоступно — история не критична */
  }
  return list;
}

/// Ссылка на список отчётов из приложения на GitHub.
export const REPORTS_LIST_URL = `${ISSUES_REPO}/issues?q=${encodeURIComponent("is:issue [app] in:title")}`;

// ---------- Открыть форму из любого места ----------

export const BUG_REPORT_EVENT = "memiro-bug-report";

/// Открывает окно «Сообщить об ошибке», по желанию — с заполненными полями
/// (напр. текст ошибки расшифровки).
export function openBugReport(prefill?: Partial<BugDraft>): void {
  window.dispatchEvent(new CustomEvent(BUG_REPORT_EVENT, { detail: prefill ?? {} }));
}
