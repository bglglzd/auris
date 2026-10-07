import { useEffect, useRef, useState } from "react";
import type { DragEvent } from "react";
import { createPortal } from "react-dom";
import type { Collection, Meeting, TranscribeState } from "../types";
import { formatClock } from "../util";
import { groupMeetings } from "../library";
import { ConfirmDialog } from "./ConfirmDialog";
import { MeetingEditDialog } from "./MeetingEditDialog";
import type { MeetingPatch } from "./MeetingEditDialog";

interface Props {
  meetings: Meeting[];
  activeId?: string | null;
  progress?: Record<string, TranscribeState>;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
  /// Сохранить правку встречи из меню «⋯» (название, участники, тема, заметки).
  onEdit?: (id: string, patch: MeetingPatch) => void | Promise<void>;
  /// Папки списка встреч.
  collections?: Collection[];
  /// Идёт поиск: показываем только папки с найденными встречами.
  searching?: boolean;
  onMove?: (id: string, collection: string) => void | Promise<void>;
  onRenameFolder?: (id: string, name: string) => void | Promise<void>;
  onDeleteFolder?: (id: string) => void | Promise<void>;
  /// Режим объединения: клик по записи выбирает её; номера — порядок склейки.
  picking?: boolean;
  /// Номер записи в склейке (1, 2, 3…) или 0 — не выбрана.
  pickNumber?: (id: string) => number;
  onPick?: (id: string) => void;
}

const OPEN_KEY = "3uxo.folders.closed";

function shortDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString(undefined, {
    day: "2-digit",
    month: "short",
  });
}

/// Первая строка заметки — подсказка в списке.
function notePreview(notes?: string): string {
  const line = (notes ?? "").split("\n").find((l) => l.trim()) ?? "";
  return line.trim();
}

function loadClosed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(OPEN_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

export function MeetingList({
  meetings,
  activeId,
  progress,
  onSelect,
  onDelete,
  onEdit,
  collections = [],
  searching = false,
  onMove,
  onRenameFolder,
  onDeleteFolder,
  picking = false,
  pickNumber,
  onPick,
}: Props) {
  const [confirmId, setConfirmId] = useState<string | null>(null);
  const [menuId, setMenuId] = useState<string | null>(null);
  // Меню «⋯» встречи в режиме «В папку…».
  const [moveMenu, setMoveMenu] = useState(false);
  const [folderMenu, setFolderMenu] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [confirmFolder, setConfirmFolder] = useState<Collection | null>(null);
  const [closed, setClosed] = useState<Set<string>>(() => loadClosed());
  const [dropOn, setDropOn] = useState<string | null>(null);
  const [edit, setEdit] = useState<{ id: string; focus: "title" | "notes" } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // Меню закрывается кликом мимо и по Esc.
  useEffect(() => {
    if (!menuId && !folderMenu) return;
    const onDown = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuId(null);
        setFolderMenu(null);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMenuId(null);
        setFolderMenu(null);
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuId, folderMenu]);

  const toggleFolder = (id: string) => {
    const next = new Set(closed);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setClosed(next);
    try {
      localStorage.setItem(OPEN_KEY, JSON.stringify([...next]));
    } catch {
      /* без хранилища — только на сеанс */
    }
  };

  const { folders, loose } = groupMeetings(meetings, collections);
  const shownFolders = searching ? folders.filter((f) => f.meetings.length > 0) : folders;
  if (meetings.length === 0 && shownFolders.length === 0) {
    return <p className="section-label">{searching ? "Ничего не найдено" : "Пока нет записей"}</p>;
  }
  const editing = edit ? meetings.find((m) => m.id === edit.id) : undefined;

  const dropProps = (folderId: string) => ({
    onDragOver: (e: DragEvent) => {
      if (picking || !e.dataTransfer.types.includes("text/x-memiro-meeting")) return;
      e.preventDefault();
      setDropOn(folderId);
    },
    onDragLeave: () => setDropOn((d) => (d === folderId ? null : d)),
    onDrop: (e: DragEvent) => {
      const id = e.dataTransfer.getData("text/x-memiro-meeting");
      setDropOn(null);
      if (id) void onMove?.(id, folderId);
    },
  });

  const renderMeeting = (m: Meeting) => {
    const tr = progress?.[m.id];
    const note = notePreview(m.notes);
    const n = picking ? (pickNumber?.(m.id) ?? 0) : 0;
    const cls = picking ? (n ? "m-item picked" : "m-item") : m.id === activeId ? "m-item active" : "m-item";
    return (
      <div key={m.id} className={menuId === m.id ? "m-row menu-open" : "m-row"}>
        <button
          className={cls}
          onClick={() => (picking ? onPick?.(m.id) : onSelect(m.id))}
          title={picking ? "Выбрать для объединения" : m.notes?.trim() ? m.notes : undefined}
          aria-pressed={picking ? n > 0 : undefined}
          draggable={!picking && !!onMove}
          onDragStart={(e) => {
            e.dataTransfer.setData("text/x-memiro-meeting", m.id);
            e.dataTransfer.effectAllowed = "move";
          }}
        >
          {picking && <span className={n ? "m-pick on" : "m-pick"} aria-hidden="true">{n || ""}</span>}
          <span className="m-title">{m.title}</span>
          <span className="m-sub">
            {tr?.running ? (
              <span className="m-badge">● расшифровка {tr.percent}%</span>
            ) : (
              <>
                {shortDate(m.created_at)} · {formatClock(m.duration_secs)}
              </>
            )}
          </span>
          {note && <span className="m-note">{note}</span>}
        </button>
        {!picking && (
          <button
            type="button"
            className="m-more"
            aria-label="Действия со встречей"
            aria-haspopup="menu"
            aria-expanded={menuId === m.id}
            title="Переименовать, заметки, папка, удалить"
            onClick={() => {
              setMoveMenu(false);
              setMenuId(menuId === m.id ? null : m.id);
            }}
          >
            ⋯
          </button>
        )}
        {menuId === m.id && (
          <div className="m-menu" role="menu" ref={menuRef}>
            {moveMenu ? (
              <>
                <div className="m-menu-title">В папку</div>
                {collections.map((c) => (
                  <button
                    key={c.id}
                    role="menuitemradio"
                    aria-checked={m.collection === c.id}
                    onClick={() => {
                      setMenuId(null);
                      if (m.collection !== c.id) void onMove?.(m.id, c.id);
                    }}
                  >
                    📁 {c.name}
                    {m.collection === c.id ? " ✓" : ""}
                  </button>
                ))}
                {collections.length === 0 && <div className="m-menu-hint">Папок пока нет — создайте её кнопкой над списком.</div>}
                {m.collection && (
                  <button
                    role="menuitem"
                    onClick={() => {
                      setMenuId(null);
                      void onMove?.(m.id, "");
                    }}
                  >
                    ↩ Без папки
                  </button>
                )}
              </>
            ) : (
              <>
                <button
                  role="menuitem"
                  onClick={() => {
                    setMenuId(null);
                    setEdit({ id: m.id, focus: "title" });
                  }}
                >
                  ✎ Переименовать
                </button>
                <button
                  role="menuitem"
                  onClick={() => {
                    setMenuId(null);
                    setEdit({ id: m.id, focus: "notes" });
                  }}
                >
                  🗒 Заметки и детали
                </button>
                {onMove && (
                  <button role="menuitem" onClick={() => setMoveMenu(true)}>
                    📁 В папку…
                  </button>
                )}
                <button
                  role="menuitem"
                  className="danger"
                  onClick={() => {
                    setMenuId(null);
                    setConfirmId(m.id);
                  }}
                >
                  🗑 Удалить встречу
                </button>
              </>
            )}
          </div>
        )}
      </div>
    );
  };

  return (
    <>
      {shownFolders.map(({ collection: c, meetings: inside }) => {
        const open = searching || !closed.has(c.id);
        return (
          <div key={c.id} className={dropOn === c.id ? "folder drop" : "folder"} {...dropProps(c.id)}>
            <div className={folderMenu === c.id ? "folder-head menu-open" : "folder-head"}>
              {renaming?.id === c.id ? (
                <input
                  className="folder-input"
                  autoFocus
                  value={renaming.name}
                  aria-label="Имя папки"
                  onChange={(e) => setRenaming({ id: c.id, name: e.target.value })}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") setRenaming(null);
                    if (e.key === "Enter" && renaming.name.trim()) {
                      void onRenameFolder?.(c.id, renaming.name.trim());
                      setRenaming(null);
                    }
                  }}
                  onBlur={() => {
                    if (renaming.name.trim() && renaming.name.trim() !== c.name) void onRenameFolder?.(c.id, renaming.name.trim());
                    setRenaming(null);
                  }}
                />
              ) : (
                <button
                  type="button"
                  className="folder-toggle"
                  aria-expanded={open}
                  onClick={() => toggleFolder(c.id)}
                  title={open ? "Свернуть папку" : "Развернуть папку"}
                >
                  <span className="folder-caret" aria-hidden="true">{open ? "▾" : "▸"}</span>
                  <span className="folder-icon" aria-hidden="true">📁</span>
                  <span className="folder-name">{c.name}</span>
                  <span className="folder-count">{inside.length}</span>
                </button>
              )}
              {!picking && renaming?.id !== c.id && (
                <button
                  type="button"
                  className="m-more"
                  aria-label="Действия с папкой"
                  aria-haspopup="menu"
                  aria-expanded={folderMenu === c.id}
                  title="Переименовать или удалить папку"
                  onClick={() => setFolderMenu(folderMenu === c.id ? null : c.id)}
                >
                  ⋯
                </button>
              )}
              {folderMenu === c.id && (
                <div className="m-menu" role="menu" ref={menuRef}>
                  <button
                    role="menuitem"
                    onClick={() => {
                      setFolderMenu(null);
                      setRenaming({ id: c.id, name: c.name });
                    }}
                  >
                    ✎ Переименовать папку
                  </button>
                  <button
                    role="menuitem"
                    className="danger"
                    onClick={() => {
                      setFolderMenu(null);
                      setConfirmFolder(c);
                    }}
                  >
                    🗑 Удалить папку
                  </button>
                </div>
              )}
            </div>
            {open && (
              <div className="folder-body">
                {inside.length === 0 ? (
                  <div className="folder-empty">Перетащите сюда встречу или выберите «⋯ → В папку…»</div>
                ) : (
                  inside.map(renderMeeting)
                )}
              </div>
            )}
          </div>
        );
      })}

      <div className={dropOn === "" ? "loose drop" : "loose"} {...dropProps("")}>
        {loose.map(renderMeeting)}
      </div>

      {/* Окна — порталом в body: у сайдбара backdrop-filter, и position:fixed
          внутри него позиционировался бы относительно сайдбара, а не окна. */}
      {editing && edit && createPortal(
        <MeetingEditDialog
          meeting={editing}
          focus={edit.focus}
          onCancel={() => setEdit(null)}
          onSave={async (patch) => {
            await onEdit?.(editing.id, patch);
            setEdit(null);
          }}
        />,
        document.body,
      )}

      {confirmId && createPortal(
        <ConfirmDialog
          message="Удалить эту встречу? Запись и расшифровка будут стёрты безвозвратно."
          onConfirm={() => {
            onDelete(confirmId);
            setConfirmId(null);
          }}
          onCancel={() => setConfirmId(null)}
        />,
        document.body,
      )}

      {confirmFolder && createPortal(
        <ConfirmDialog
          message={`Удалить папку «${confirmFolder.name}»? Встречи из неё останутся в списке.`}
          onConfirm={() => {
            void onDeleteFolder?.(confirmFolder.id);
            setConfirmFolder(null);
          }}
          onCancel={() => setConfirmFolder(null)}
        />,
        document.body,
      )}
    </>
  );
}
