import { useState } from "react";
import type { Meeting } from "../types";
import { modEnter } from "../platform";

/// Что можно поправить у встречи из меню «⋯».
export interface MeetingPatch {
  title: string;
  participants: string;
  topic: string;
  notes: string;
}

interface Props {
  meeting: Meeting;
  /// Какое поле сфокусировать при открытии.
  focus?: "title" | "notes";
  onSave: (patch: MeetingPatch) => void | Promise<void>;
  onCancel: () => void;
}

/// Окно правки встречи: название, участники, тема и свободные заметки —
/// чтобы среди многих встреч не терялось, о чём была каждая.
export function MeetingEditDialog({ meeting, focus = "title", onSave, onCancel }: Props) {
  const [f, setF] = useState<MeetingPatch>({
    title: meeting.title,
    participants: meeting.participants,
    topic: meeting.topic,
    notes: meeting.notes ?? "",
  });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const set = (k: keyof MeetingPatch) => (e: { target: { value: string } }) =>
    setF((p) => ({ ...p, [k]: e.target.value }));

  const save = async () => {
    setSaving(true);
    setError("");
    try {
      await onSave({ ...f, title: f.title.trim() || meeting.title });
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  return (
    <div className="overlay" onClick={onCancel}>
      <div
        className="modal edit-meeting-modal"
        role="dialog"
        aria-label="Изменить встречу"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) void save();
        }}
      >
        <h2>Встреча</h2>
        <div className="field">
          <label htmlFor="em-title">Название</label>
          <input id="em-title" value={f.title} onChange={set("title")} autoFocus={focus === "title"} />
        </div>
        <div className="field-row">
          <div className="field">
            <label htmlFor="em-part">Участники</label>
            <input id="em-part" value={f.participants} onChange={set("participants")} placeholder="Участник Б, Участник А" />
          </div>
          <div className="field">
            <label htmlFor="em-topic">Тема</label>
            <input id="em-topic" value={f.topic} onChange={set("topic")} placeholder="О чём встреча" />
          </div>
        </div>
        <div className="field">
          <label htmlFor="em-notes">Заметки</label>
          <textarea
            id="em-notes"
            value={f.notes}
            onChange={set("notes")}
            rows={6}
            autoFocus={focus === "notes"}
            placeholder="Что важно помнить: контекст, договорённости, ссылки, к кому вернуться…"
          />
          <span className="hint">Первая строка заметки видна в списке встреч; по заметкам работает поиск.</span>
        </div>
        {error && <div className="ai-error">{error}</div>}
        <div className="modal-actions">
          <span className="hint">{modEnter()} — сохранить</span>
          <div className="modal-actions-btns">
            <button className="btn ghost" onClick={onCancel} disabled={saving}>
              Отмена
            </button>
            <button className="btn primary" onClick={save} disabled={saving}>
              {saving ? "Сохраняю…" : "Сохранить"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
