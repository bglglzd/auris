import { useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { save } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import { copyToClipboard } from "../clipboard";
import { getLogText } from "../log";
import { getSettings, isAiConfigured } from "../settings";
import { getTheme } from "../theme";
import { platform } from "../platform";
import {
  FREQUENCY_LABEL,
  IMPACT_LABEL,
  REPORTS_LIST_URL,
  emptyDraft,
  issueUrl,
  loadReports,
  rememberReport,
  reportMarkdown,
  sanitize,
  tailLines,
} from "../bugreport";
import type { BugDraft, BugEnv, Frequency, Impact, SentReport } from "../bugreport";

interface Props {
  prefill?: Partial<BugDraft>;
  onClose: () => void;
}

const OS_NAME: Record<string, string> = { macos: "macOS", windows: "Windows", linux: "Linux" };

/// Среда: версия приложения и система («macOS 15.1 (aarch64)»).
async function collectEnv(): Promise<BugEnv> {
  const version = await getVersion().catch(() => "?");
  try {
    const s = await api.systemInfo();
    const os = s.os_version.startsWith("Windows") ? s.os_version : `${OS_NAME[s.os] ?? s.os} ${s.os_version}`.trim();
    return { version, platform: `${os} (${s.arch})` };
  } catch {
    return { version, platform: OS_NAME[platform] ?? platform };
  }
}

/// Диагностика: настройки (без ключей) и свежие строки логов, очищенные от
/// личного.
async function collectDiagnostics(env: BugEnv): Promise<string> {
  const s = getSettings();
  const lines = [
    `Memiro AI ${env.version} · ${env.platform}`,
    `Распознавание: ${s.whisper.model || "по умолчанию"} · язык ${s.whisper.language || "авто"}`,
    `ИИ: ${isAiConfigured(s) ? `подключён (${s.ai.model || "модель сервера"})` : "не подключён"}`,
    `Авто-запись: ${s.autoRecord.enabled ? "вкл" : "выкл"} · горячая клавиша: ${s.hotkey || "нет"} · тема: ${getTheme()}`,
  ];
  const front = tailLines(getLogText(), 60);
  let back = "";
  try {
    back = tailLines(await api.getBackendLog(), 80);
  } catch {
    /* бэкенд-лог недоступен */
  }
  return sanitize(
    [lines.join("\n"), "--- журнал интерфейса ---", front.trim(), back.trim() && "--- журнал приложения ---", back.trim()]
      .filter(Boolean)
      .join("\n"),
  );
}

function fmtDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleString(undefined, { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
}

/// «Сообщить об ошибке»: форма → заполненный issue на GitHub (или копия / файл).
export function BugReportModal({ prefill, onClose }: Props) {
  const [tab, setTab] = useState<"new" | "mine">("new");
  const [d, setD] = useState<BugDraft>(() => emptyDraft(prefill));
  const [env, setEnv] = useState<BugEnv>({ version: "…", platform: "…" });
  const [diag, setDiag] = useState("");
  const [history, setHistory] = useState<SentReport[]>(() => loadReports());
  const [sent, setSent] = useState<"" | "github" | "copy" | "file">("");
  const [note, setNote] = useState("");

  useEffect(() => {
    let alive = true;
    void (async () => {
      const e = await collectEnv();
      const text = await collectDiagnostics(e);
      if (alive) {
        setEnv(e);
        setDiag(text);
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  const set = (k: "title" | "what" | "steps" | "expected") => (e: { target: { value: string } }) =>
    setD((p) => ({ ...p, [k]: e.target.value }));
  const ready = d.what.trim().length > 0;
  const markdown = useMemo(() => reportMarkdown(d, env, diag), [d, env, diag]);

  const done = (via: SentReport["via"]) => {
    setHistory(rememberReport({ title: d.title.trim() || d.what.trim().split("\n")[0].slice(0, 80), at: new Date().toISOString(), version: env.version, via }));
    setSent(via);
  };

  const sendGithub = async () => {
    const { url, truncated } = issueUrl(d, env, diag);
    if (truncated) {
      // Полный отчёт — в буфер: можно вставить в issue целиком.
      await copyToClipboard(markdown);
      setNote("Диагностика длинная — в форму попали последние строки, полный отчёт скопирован: его можно вставить в комментарий.");
    }
    try {
      await openUrl(url);
    } catch {
      window.open(url, "_blank");
    }
    done("github");
  };

  const copy = async () => {
    await copyToClipboard(markdown);
    done("copy");
  };

  const saveFile = async () => {
    const path = await save({
      defaultPath: `memiro-bug-report-${new Date().toISOString().slice(0, 10)}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    }).catch(() => null);
    if (!path) return;
    await api.saveTextFile(path, markdown);
    done("file");
  };

  const openList = () => {
    void openUrl(REPORTS_LIST_URL).catch(() => window.open(REPORTS_LIST_URL, "_blank"));
  };

  return (
    <div className="overlay" onClick={onClose}>
      <div
        className="modal bug-modal"
        role="dialog"
        aria-label="Сообщить об ошибке"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
        }}
      >
        <h2>Сообщить об ошибке</h2>
        <p className="lead">
          Отчёт попадёт разработчикам Memiro — ошибки разбираются и исправляются по плану, исправления приходят
          обновлением.
        </p>

        <div className="bug-tabs" role="tablist">
          <button role="tab" aria-selected={tab === "new"} className={tab === "new" ? "seg-btn on" : "seg-btn"} onClick={() => setTab("new")}>
            Новый отчёт
          </button>
          <button role="tab" aria-selected={tab === "mine"} className={tab === "mine" ? "seg-btn on" : "seg-btn"} onClick={() => setTab("mine")}>
            Мои отчёты{history.length ? ` · ${history.length}` : ""}
          </button>
        </div>

        {tab === "mine" ? (
          <div className="bug-history">
            {history.length === 0 ? (
              <p className="hint">Вы ещё не отправляли отчётов.</p>
            ) : (
              <ul>
                {history.map((r, i) => (
                  <li key={i}>
                    <span className="bh-title">{r.title}</span>
                    <span className="bh-meta">
                      {fmtDate(r.at)} · v{r.version} · {r.via === "github" ? "GitHub" : r.via === "copy" ? "скопирован" : "файл"}
                    </span>
                  </li>
                ))}
              </ul>
            )}
            <button type="button" className="link-btn" onClick={openList}>
              Статус всех отчётов на GitHub →
            </button>
          </div>
        ) : sent ? (
          <div className="bug-done" role="status">
            <div className="bug-done-icon" aria-hidden="true">✓</div>
            <div>
              <strong>Спасибо! Отчёт подготовлен.</strong>
              <p className="hint">
                {sent === "github"
                  ? "В браузере открылась форма GitHub с вашим отчётом — проверьте текст и нажмите «Submit new issue»."
                  : sent === "copy"
                    ? "Отчёт скопирован — пришлите его разработчикам удобным способом."
                    : "Отчёт сохранён файлом — пришлите его разработчикам."}
              </p>
              {note && <p className="hint">{note}</p>}
            </div>
          </div>
        ) : (
          <>
            <div className="field">
              <label htmlFor="bug-title">Коротко</label>
              <input id="bug-title" value={d.title} onChange={set("title")} placeholder="Например: не расшифровывается запись" autoFocus />
            </div>
            <div className="field">
              <label htmlFor="bug-what">Что случилось *</label>
              <textarea id="bug-what" value={d.what} onChange={set("what")} rows={3} placeholder="Что пошло не так, текст ошибки, если был" />
            </div>
            <div className="field-row">
              <div className="field">
                <label htmlFor="bug-steps">Как повторить</label>
                <textarea id="bug-steps" value={d.steps} onChange={set("steps")} rows={3} placeholder={"1. Открыл встречу\n2. Нажал «Расшифровать»"} />
              </div>
              <div className="field">
                <label htmlFor="bug-expected">Что ожидали</label>
                <textarea id="bug-expected" value={d.expected} onChange={set("expected")} rows={3} placeholder="Как должно было быть" />
              </div>
            </div>
            <div className="bug-choice">
              <div role="group" aria-label="Насколько мешает">
                <span className="bug-choice-label">Насколько мешает</span>
                {(Object.keys(IMPACT_LABEL) as Impact[]).map((k) => (
                  <button key={k} type="button" className={d.impact === k ? "seg-btn on" : "seg-btn"} onClick={() => setD((p) => ({ ...p, impact: k }))}>
                    {IMPACT_LABEL[k]}
                  </button>
                ))}
              </div>
              <div role="group" aria-label="Как часто">
                <span className="bug-choice-label">Как часто</span>
                {(Object.keys(FREQUENCY_LABEL) as Frequency[]).map((k) => (
                  <button key={k} type="button" className={d.frequency === k ? "seg-btn on" : "seg-btn"} onClick={() => setD((p) => ({ ...p, frequency: k }))}>
                    {FREQUENCY_LABEL[k]}
                  </button>
                ))}
              </div>
            </div>
            <label className="bug-check">
              <input
                type="checkbox"
                checked={d.includeDiagnostics}
                onChange={(e) => setD((p) => ({ ...p, includeDiagnostics: e.target.checked }))}
              />
              Приложить диагностику — версия, система, настройки и журнал без ключей, путей с именем и текста встреч
            </label>
            {d.includeDiagnostics && (
              <details className="bug-diag">
                <summary>Что будет отправлено</summary>
                <pre>{diag || "Собираю…"}</pre>
              </details>
            )}
            <p className="hint">
              «Отправить» откроет GitHub с заполненным отчётом (нужен бесплатный аккаунт). Без аккаунта —
              скопируйте отчёт или сохраните файлом и пришлите нам.
            </p>
          </>
        )}

        <div className="modal-actions">
          <span className="hint">{env.version !== "…" ? `Memiro AI ${env.version} · ${env.platform}` : ""}</span>
          <div className="modal-actions-btns">
            {tab === "new" && !sent ? (
              <>
                <button className="btn ghost" onClick={copy} disabled={!ready}>
                  Скопировать
                </button>
                <button className="btn ghost" onClick={saveFile} disabled={!ready}>
                  Сохранить файл
                </button>
                <button className="btn primary" onClick={sendGithub} disabled={!ready}>
                  Отправить
                </button>
              </>
            ) : (
              <button className="btn primary" onClick={onClose}>
                Готово
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
