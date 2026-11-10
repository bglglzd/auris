import { useRef, useState } from "react";
import { api } from "../api";
import { getFixes, MEETING_TEXT_EVENT } from "../fixes";
import type { Fix } from "../fixes";

export function MeetingCorrections({ id, disabled }: { id: string; disabled: boolean }) {
  const [fixes, setRules] = useState(() => getFixes(id));
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const running = useRef(false);
  const apply = async (changes: Fix[]) => {
    if (disabled || running.current || !changes.length) return;
    running.current = true;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const count = await api.applyMeetingFixes(id, changes);
      setFrom(""); setTo("");
      setMessage(`Сохранено текстов: ${count}. Правила применяются и к новым расшифровкам и отчётам этой встречи.`);
    } catch (e) {
      setError(`Не удалось применить все исправления: ${String(e)}. Часть текстов могла сохраниться — проверьте их перед повтором.`);
    } finally {
      setRules(getFixes(id));
      window.dispatchEvent(new CustomEvent(MEETING_TEXT_EVENT, { detail: { id } }));
      running.current = false;
      setBusy(false);
    }
  };
  const add = () => {
    const fix = { from: from.trim(), to };
    if (!fix.from) return;
    void apply([fix]);
  };
  const remove = async (from: string) => {
    if (disabled || running.current) return;
    running.current = true;
    setBusy(true);
    try {
      await api.removeMeetingFix(id, from);
      setRules(getFixes(id)); setError("");
    } catch (e) { setError(String(e)); }
    finally { running.current = false; setBusy(false); }
  };
  return <details className="card">
    <summary className="card-head">Сквозные исправления</summary>
    <div className="card-body">
      <p className="muted">Заменить термин в расшифровке и всех сохранённых ИИ-отчётах этой встречи. Совпадение — целое слово без учёта регистра. Правила действуют по порядку; удаление правила не отменяет уже сохранённые правки.</p>
      {fixes.map((fix, index) => <div key={`${fix.from}-${index}`}>
        <span>{fix.from} → {fix.to || "(удалить)"} </span>
        <button className="btn ghost small" disabled={disabled || busy} onClick={() => void remove(fix.from)} aria-label={`Убрать правило ${fix.from}`}>Убрать</button>
      </div>)}
      <div className="field"><label htmlFor={`fix-from-${id}`}>Как написано</label><input id={`fix-from-${id}`} value={from} onChange={(e) => setFrom(e.target.value)} disabled={disabled || busy} /></div>
      <div className="field"><label htmlFor={`fix-to-${id}`}>Как правильно</label><input id={`fix-to-${id}`} value={to} onChange={(e) => setTo(e.target.value)} disabled={disabled || busy} /></div>
      <button className="btn primary" disabled={disabled || busy || !from.trim()} onClick={add}>{busy ? "Применяю…" : "Исправить везде"}</button>{" "}
      {fixes.length > 0 && <button className="btn ghost" disabled={disabled || busy} onClick={() => void apply(fixes)}>Применить правила заново</button>}
      {message && <p role="status">{message}</p>}
      {error && <p className="ai-error" role="alert">{error}</p>}
    </div>
  </details>;
}
