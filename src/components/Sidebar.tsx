import { useMemo, useRef, useState } from "react";
import type { Meeting, TranscribeState } from "../types";
import { getTheme, setTheme, type Theme } from "../theme";
import { RecordButton } from "./RecordButton";
import { MeetingList } from "./MeetingList";
import { MemiroMark } from "./MemiroMark";
import type { MeetingPatch } from "./MeetingEditDialog";
import { useAppMenu } from "../appmenu";
import { isMac } from "../platform";
import { openBugReport } from "../bugreport";

interface Props {
  meetings: Meeting[];
  activeId: string | null;
  recording: boolean;
  paused: boolean;
  elapsed: number;
  solo: boolean;
  progress: Record<string, TranscribeState>;
  open?: boolean;
  onStart: () => void;
  onStop: () => void;
  onPause: () => void;
  onResume: () => void;
  onSoloChange: (v: boolean) => void;
  onImport: () => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
  onEdit: (id: string, patch: MeetingPatch) => void | Promise<void>;
  onOpenSettings: () => void;
}

export function Sidebar(p: Props) {
  const [q, setQ] = useState("");
  const [theme, setThemeState] = useState<Theme>(() => getTheme());

  const searchRef = useRef<HTMLInputElement>(null);

  const pickTheme = (t: Theme) => {
    setTheme(t);
    setThemeState(t);
  };

  // Строка меню macOS: ⌘F — поиск, «Вид → тема».
  useAppMenu("find", () => searchRef.current?.focus());
  useAppMenu("theme", () => pickTheme(getTheme() === "dark" ? "light" : "dark"));

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return p.meetings;
    return p.meetings.filter((m) =>
      `${m.title} ${m.participants} ${m.topic} ${m.notes ?? ""}`
        .toLowerCase()
        .includes(needle),
    );
  }, [p.meetings, q]);

  return (
    <aside className={p.open ? "sidebar open" : "sidebar"}>
      {isMac && <div className="mac-drag" data-tauri-drag-region />}
      <div className="brand">
        <MemiroMark size={26} />
        <div className="brand-lockup">
          <span className="brand-word">
            memiro<span className="brand-ai">AI</span>
          </span>
          <span className="brand-divider" />
          <span className="brand-desc">
            память
            <br />
            ваших встреч
          </span>
        </div>
      </div>

      <RecordButton
        recording={p.recording}
        paused={p.paused}
        elapsed={p.elapsed}
        onStart={p.onStart}
        onStop={p.onStop}
        onPause={p.onPause}
        onResume={p.onResume}
      />

      {!p.recording && (
        <button
          type="button"
          className={p.solo ? "side-btn toggle on" : "side-btn toggle"}
          aria-pressed={p.solo}
          onClick={() => p.onSoloChange(!p.solo)}
          title="Заметка для себя: записывается только ваш микрофон, один голос «Я»"
        >
          <svg
            className="btn-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <circle cx="12" cy="7.5" r="3.5" />
            <path d="M5 20c0-3.9 3.1-7 7-7s7 3.1 7 7" />
          </svg>
          Заметка · я один
          <span className="side-btn-state" aria-hidden="true">
            {p.solo ? "✓" : ""}
          </span>
        </button>
      )}

      <button className="side-btn" onClick={p.onImport}>
        <svg
          className="btn-icon"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M12 3v11" />
          <path d="m8 10.5 4 4 4-4" />
          <path d="M5 20h14" />
        </svg>
        Импорт записи
      </button>

      <div className="search">
        <input
          ref={searchRef}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="Поиск встреч"
        />
      </div>

      <div className="meetings">
        <div className="section-label">Встречи</div>
        <MeetingList
          meetings={filtered}
          activeId={p.activeId}
          progress={p.progress}
          onSelect={p.onSelect}
          onDelete={p.onDelete}
          onEdit={p.onEdit}
        />
      </div>

      <div className="sidebar-footer">
        <div className="privacy-note">
          <svg
            className="pn-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.7"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <rect x="5" y="11" width="14" height="9" rx="2" />
            <path d="M8 11V8a4 4 0 0 1 8 0v3" />
          </svg>
          <span>Локально · открытый код</span>
        </div>
        <div className="footer-row">
          <div className="theme-switch" role="group" aria-label="Тема оформления">
            <button
              className={theme === "light" ? "active" : ""}
              onClick={() => pickTheme("light")}
              title="Светлая тема"
              aria-label="Светлая тема"
            >
              ☀
            </button>
            <button
              className={theme === "dark" ? "active" : ""}
              onClick={() => pickTheme("dark")}
              title="Тёмная тема"
              aria-label="Тёмная тема"
            >
              ☾
            </button>
          </div>
          <button
            type="button"
            className="bug-trigger"
            onClick={() => openBugReport()}
            title="Сообщить об ошибке"
            aria-label="Сообщить об ошибке"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <rect x="8" y="7" width="8" height="12" rx="4" />
              <path d="M9.5 7a2.5 2.5 0 0 1 5 0" />
              <path d="M4 12h4M16 12h4M5 17l3-1.5M19 17l-3-1.5M5 7l3 1.5M19 7l-3 1.5" />
            </svg>
          </button>
          <button
            type="button"
            className="settings-trigger"
            onClick={p.onOpenSettings}
            title="Настройки"
            aria-label="Настройки"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <circle cx="12" cy="12" r="3" />
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
          </button>
        </div>
      </div>
    </aside>
  );
}
