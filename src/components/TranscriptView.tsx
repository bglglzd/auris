import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { CSSProperties } from "react";
import type { Transcript } from "../types";
import type { SpeakerLabels } from "../labels";
import { nameForSpeaker } from "../labels";
import { clock } from "../export";
import { runLength } from "../speakers";

interface Props {
  transcript: Transcript | null;
  activeIndex: number;
  labels: SpeakerLabels;
  onSeek: (secs: number) => void;
  /// Режим правки: реплики становятся редактируемыми.
  editing?: boolean;
  /// Доступные id говорящих для переназначения (в режиме правки).
  speakerOptions?: string[];
  onEditText?: (index: number, text: string) => void;
  onEditSpeaker?: (index: number, speaker: string) => void;
  onDeleteSegment?: (index: number) => void;
  /// Быстрая смена говорящего у реплики (без режима правки): `speaker` —
  /// id голоса или `null` для нового голоса; `following` — и следующие подряд
  /// реплики того же голоса.
  onReassign?: (index: number, speaker: string | null, following: boolean) => void;
}

/// Инициалы для аватара спикера: 1–2 буквы из имени.
function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[1][0]).toUpperCase();
}

export function TranscriptView({
  transcript,
  activeIndex,
  labels,
  onSeek,
  editing = false,
  speakerOptions = [],
  onEditText,
  onEditSpeaker,
  onDeleteSegment,
  onReassign,
}: Props) {
  const activeRef = useRef<HTMLDivElement>(null);
  // Открытое меню «Кто говорит» (индекс реплики) и галочка «и следующие».
  // Меню — порталом в body с fixed-позицией: лента прокручивается и
  // обрезала бы его.
  const [menuAt, setMenuAt] = useState<number | null>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const [following, setFollowing] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  useEffect(() => {
    if (menuAt === null) return;
    const close = () => setMenuAt(null);
    // Инерционная прокрутка сразу после клика (трекпад Mac) не закрывает меню.
    const openedAt = Date.now();
    const onScroll = (e: Event) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      if (Date.now() - openedAt > 400) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("click", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, [menuAt]);
  // Под именем, а если снизу не помещается — над ним; не за краем окна.
  useLayoutEffect(() => {
    if (menuAt === null || !anchor) {
      setPos(null);
      return;
    }
    const h = menuRef.current?.offsetHeight ?? 0;
    const w = menuRef.current?.offsetWidth ?? 0;
    const below = anchor.bottom + 6;
    const top = below + h > window.innerHeight - 8 ? Math.max(8, anchor.top - 6 - h) : below;
    const left = Math.max(8, Math.min(anchor.left - 6, window.innerWidth - w - 8));
    setPos({ top, left });
  }, [menuAt, anchor, following]);

  useEffect(() => {
    if (editing) return;
    const el = activeRef.current;
    if (!el) return;
    try {
      el.scrollIntoView({ block: "nearest", behavior: "smooth" });
    } catch {
      // jsdom / unsupported — ignore
    }
  }, [activeIndex, editing]);

  // Порядок появления говорящих → стабильный цвет аватара.
  const speakerOrder = useMemo(
    () =>
      transcript
        ? Array.from(new Set(transcript.segments.map((s) => s.speaker)))
        : [],
    [transcript],
  );
  const speakerIdx = (id: string): number => {
    const i = speakerOrder.indexOf(id);
    return (i < 0 ? 0 : i) % 6;
  };

  if (!transcript || transcript.segments.length === 0) {
    return <div className="transcript-empty">Расшифровки пока нет.</div>;
  }

  return (
    <div className={editing ? "transcript editing" : "transcript"}>
      {transcript.segments.map((seg, i) => {
        const active = i === activeIndex;
        const name = nameForSpeaker(labels, seg.speaker);
        const idx = speakerIdx(seg.speaker);
        return (
          <div
            key={i}
            ref={!editing && active ? activeRef : undefined}
            className={`turn${!editing && active ? " active" : ""}`}
            onClick={editing ? undefined : () => onSeek(seg.start_secs)}
            title={editing ? undefined : "Перейти к этому моменту"}
          >
            <span
              className="turn-avatar"
              style={{ background: `var(--spk-${idx})` } as CSSProperties}
            >
              {initials(name)}
            </span>
            <div className="turn-body">
              <div className="turn-meta">
                {editing && speakerOptions.length > 1 ? (
                  <select
                    className="turn-speaker-sel"
                    value={seg.speaker}
                    onChange={(e) => onEditSpeaker?.(i, e.target.value)}
                    title="Кто говорит"
                  >
                    {speakerOptions.map((sp) => (
                      <option key={sp} value={sp}>
                        {nameForSpeaker(labels, sp)}
                      </option>
                    ))}
                  </select>
                ) : onReassign && !editing ? (
                  <span className="turn-who">
                    <button
                      type="button"
                      className="turn-name turn-name-btn"
                      aria-haspopup="menu"
                      aria-expanded={menuAt === i}
                      title="Сменить говорящего"
                      onClick={(e) => {
                        e.stopPropagation();
                        setFollowing(false);
                        setAnchor(e.currentTarget.getBoundingClientRect());
                        setMenuAt(menuAt === i ? null : i);
                      }}
                    >
                      {name}
                      <span className="turn-name-caret" aria-hidden="true">▾</span>
                    </button>
                    {menuAt === i && createPortal(
                      <div
                        ref={menuRef}
                        className="speaker-menu"
                        role="menu"
                        style={{ top: pos?.top ?? -9999, left: pos?.left ?? -9999 }}
                        onClick={(e) => e.stopPropagation()}
                      >
                        <div className="speaker-menu-title">Кто говорит</div>
                        {speakerOptions.map((sp) => (
                          <button
                            key={sp}
                            type="button"
                            role="menuitemradio"
                            aria-checked={sp === seg.speaker}
                            className={sp === seg.speaker ? "speaker-menu-item on" : "speaker-menu-item"}
                            onClick={() => {
                              setMenuAt(null);
                              if (sp !== seg.speaker) onReassign(i, sp, following);
                            }}
                          >
                            <span className="speaker-menu-dot" style={{ background: `var(--spk-${speakerIdx(sp)})` } as CSSProperties} />
                            {nameForSpeaker(labels, sp)}
                            {sp === seg.speaker && <span className="speaker-menu-check">✓</span>}
                          </button>
                        ))}
                        <button
                          type="button"
                          role="menuitem"
                          className="speaker-menu-item new"
                          onClick={() => {
                            setMenuAt(null);
                            onReassign(i, null, following);
                          }}
                        >
                          ＋ Новый голос
                        </button>
                        {runLength(transcript, i) > 1 && (
                          <label className="speaker-menu-following">
                            <input type="checkbox" checked={following} onChange={(e) => setFollowing(e.target.checked)} />
                            и следующие подряд ({runLength(transcript, i) - 1})
                          </label>
                        )}
                      </div>,
                      document.body,
                    )}
                  </span>
                ) : (
                  <span className="turn-name">{name}</span>
                )}
                <span className="turn-time">{clock(seg.start_secs)}</span>
                {editing && (
                  <button
                    type="button"
                    className="turn-del"
                    onClick={() => onDeleteSegment?.(i)}
                    title="Удалить реплику"
                  >
                    🗑
                  </button>
                )}
              </div>
              {editing ? (
                <textarea
                  className="turn-edit"
                  value={seg.text}
                  rows={Math.max(1, Math.ceil(seg.text.length / 60))}
                  onChange={(e) => onEditText?.(i, e.target.value)}
                />
              ) : (
                <div className="turn-text">{seg.text}</div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
