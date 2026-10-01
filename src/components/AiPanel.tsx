import { useEffect, useState } from "react";
import type { Meeting, ReportKind } from "../types";
import type { SpeakerLabels } from "../labels";
import { api } from "../api";
import { getSettings, isAiConfigured } from "../settings";
import { stripMarkdown } from "../export";
import { meetingContext } from "../speakers";
import { AI_AUTO_EVENT, PRESETS, REPORT_META, REPORT_ORDER } from "../reports";
import type { AiAutoDetail } from "../reports";
import { Markdown } from "./Markdown";
import { openBugReport } from "../bugreport";
import { CopyButton } from "./CopyButton";

interface Props {
  meeting: Meeting;
  labels: SpeakerLabels;
  /// Есть ли расшифровка (без неё ИИ работать не с чем).
  hasTranscript: boolean;
  reports: Partial<Record<ReportKind, string>>;
  onReport: (kind: ReportKind, text: string) => void;
  onMetaSaved: () => void;
}

export function AiPanel({ meeting, labels, hasTranscript, reports, onReport, onMetaSaved }: Props) {
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [question, setQuestion] = useState("");
  const [answer, setAnswer] = useState("");
  const [editKind, setEditKind] = useState<ReportKind | null>(null);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);
  // Фоновая работа ИИ после расшифровки (авто-заголовок/итоги).
  const [auto, setAuto] = useState<AiAutoDetail | null>(null);
  const configured = isAiConfigured(getSettings());

  useEffect(() => {
    setAnswer("");
    setEditKind(null);
    setError("");
    setAuto(null);
  }, [meeting.id]);

  useEffect(() => {
    const on = (e: Event) => {
      const d = (e as CustomEvent<AiAutoDetail>).detail;
      if (d.id !== meeting.id) return;
      setAuto(d.busy ? d : null);
      if (d.error) setError(d.error);
    };
    window.addEventListener(AI_AUTO_EVENT, on);
    return () => window.removeEventListener(AI_AUTO_EVENT, on);
  }, [meeting.id]);

  const aiCfg = () => {
    const s = getSettings();
    if (!isAiConfigured(s)) {
      setError("Подключите ИИ в «Настройки → Искусственный интеллект» (адрес, ключ, модель).");
      return null;
    }
    if (!hasTranscript) {
      setError("Сначала расшифруйте встречу — ИИ работает с текстом разговора.");
      return null;
    }
    setError("");
    return s.ai;
  };

  const ctx = () => meetingContext(meeting, labels);

  const generate = async (kind: ReportKind) => {
    const c = aiCfg();
    if (!c || busy) return;
    setBusy(kind);
    try {
      onReport(kind, await api.generateReport(meeting.id, kind, c, ctx()));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy("");
    }
  };

  const doSuggest = async () => {
    const c = aiCfg();
    if (!c || busy) return;
    setBusy("suggest");
    try {
      const m = await api.suggestMeta(meeting.id, c, ctx());
      await api.updateMeetingMeta(
        meeting.id,
        m.title || meeting.title,
        m.participants || meeting.participants,
        m.topic || meeting.topic,
      );
      onMetaSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy("");
    }
  };

  const doAsk = async () => {
    const c = aiCfg();
    if (!c || busy || !question.trim()) return;
    setBusy("ask");
    try {
      setAnswer(await api.askNamed(meeting.id, c, question, ctx()));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy("");
    }
  };

  const saveEdit = async (kind: ReportKind) => {
    setSaving(true);
    try {
      await api.saveReport(meeting.id, kind, draft);
      onReport(kind, draft);
      setEditKind(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const block = (kind: ReportKind) => {
    const content = reports[kind];
    if (!content) return null;
    const meta = REPORT_META[kind];
    const isEditing = editKind === kind;
    return (
      <div className="ai-block" key={kind}>
        <div className="ai-block-head">
          <span className="ai-block-title">
            <span className="ai-block-icon">{meta.icon}</span> {meta.title}
          </span>
          {isEditing ? (
            <div className="btn-row">
              <button className="btn ghost" onClick={() => setEditKind(null)} disabled={saving}>
                Отмена
              </button>
              <button
                className="btn primary"
                onClick={() => saveEdit(kind)}
                disabled={saving}
                title="Сохранить правки отчёта"
              >
                {saving ? "…" : "✓ Сохранить"}
              </button>
            </div>
          ) : (
            <div className="btn-row">
              <CopyButton
                // Инструкция для ИИ копируется как есть: агенты понимают Markdown.
                text={() => (kind === "agent" ? content : stripMarkdown(content))}
                label="📋 Копировать"
                title={
                  kind === "agent"
                    ? "Скопировать промпт (с разметкой) — вставьте его в ИИ-агента"
                    : "Скопировать как обычный текст, без Markdown"
                }
              />
              <button
                className="btn ghost"
                onClick={() => {
                  setError("");
                  setEditKind(kind);
                  setDraft(content);
                }}
                title="Исправить текст отчёта"
              >
                ✎ Редактировать
              </button>
            </div>
          )}
        </div>
        {isEditing ? (
          <textarea
            className="ai-edit"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            rows={Math.min(24, Math.max(6, draft.split("\n").length + 1))}
          />
        ) : (
          <div className="summary-text">
            <Markdown>{content}</Markdown>
          </div>
        )}
      </div>
    );
  };

  const anyBusy = busy !== "" || !!auto;

  return (
    <div className="card">
      <div className="card-head">
        <h3>ИИ-ассистент</h3>
        <span
          className="ai-key-pill"
          title={
            getSettings().ai.model
              ? `Модель: ${getSettings().ai.model} · через ваш ключ`
              : "ИИ-функции работают через ваш API-ключ"
          }
        >
          {getSettings().ai.model ? `ИИ · ${getSettings().ai.model}` : "ИИ · ваш ключ"}
        </span>
        <div className="spacer" />
        <button
          className="btn ghost"
          onClick={doSuggest}
          disabled={anyBusy}
          title="Придумать заголовок, участников и тему по разговору"
        >
          {busy === "suggest" ? "…" : "✨ Заголовок"}
        </button>
      </div>
      <div className="card-body">
        {!configured && (
          <p className="hint ai-setup">
            Подключите свою модель в «Настройки → Искусственный интеллект» — и после
            каждой расшифровки Memiro сам придумает заголовок и подведёт итоги.
          </p>
        )}
        {auto && (
          <div className="ai-auto">
            <span className="spin">◜</span> {auto.label ?? "ИИ работает…"}
          </div>
        )}
        {error && (
          <div className="ai-error">
            {error}{" "}
            <button
              type="button"
              className="link-btn"
              onClick={() => openBugReport({ title: "Ошибка ИИ", what: error })}
            >
              Сообщить об ошибке
            </button>
          </div>
        )}

        <div className="preset-grid">
          {PRESETS.map((k) => {
            const m = REPORT_META[k];
            const done = !!reports[k];
            return (
              <button
                key={k}
                type="button"
                className={done ? "preset done" : "preset"}
                onClick={() => generate(k)}
                disabled={anyBusy}
                title={done ? "Создать заново" : m.what}
              >
                <span className="preset-top">
                  <span className="preset-icon">{m.icon}</span>
                  <span className="preset-title">{m.title}</span>
                  {done && <span className="preset-state">↻</span>}
                </span>
                <span className="preset-what">{busy === k ? m.busy : m.what}</span>
              </button>
            );
          })}
        </div>

        {REPORT_ORDER.map(block)}

        <div className="ask-row">
          <input
            value={question}
            onChange={(e) => setQuestion(e.target.value)}
            placeholder="Спросить по встрече: «Что решили по срокам?»"
            onKeyDown={(e) => {
              if (e.key === "Enter") doAsk();
            }}
          />
          <button className="btn primary" onClick={doAsk} disabled={anyBusy || !question.trim()}>
            {busy === "ask" ? "…" : "Спросить"}
          </button>
        </div>
        {answer && (
          <div className="ai-answer">
            <div className="ai-block-head">
              <span className="ai-block-title">Ответ</span>
              <CopyButton
                text={() => stripMarkdown(answer)}
                label="📋 Копировать"
                title="Скопировать ответ как обычный текст, без Markdown"
              />
            </div>
            <Markdown>{answer}</Markdown>
          </div>
        )}
      </div>
    </div>
  );
}
