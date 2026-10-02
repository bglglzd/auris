import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { CSSProperties } from "react";
import type { Transcript } from "../types";
import type { SpeakerLabels } from "../labels";
import { nameForSpeaker } from "../labels";
import { clock } from "../export";
import { newSpeakerId, runLength } from "../speakers";
import type { Gap } from "../transcriptedit";

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
  /// Правка текста одной реплики прямо в ленте (пустой текст — удалить);
  /// `speaker` — выбранный говорящий для новой реплики.
  onSaveText?: (index: number, text: string, speaker?: string) => void;
  /// Исходный (не цензурированный) текст реплики для правки.
  rawText?: (index: number) => string;
  /// Пропуски — паузы в тексте, где речь могла потеряться.
  gaps?: Gap[];
  onPlayRange?: (start: number, end: number) => void;
  onRecognizeGap?: (gap: Gap) => Promise<void>;
  /// Вставляет пустую реплику в пропуск; возвращает её индекс (для правки).
  onAddManual?: (gap: Gap) => Promise<number>;
  /// Вставляет пустую реплику после реплики `index`; возвращает её индекс.
  onAddAfter?: (index: number) => number;
  /// Открыть правку реплики снаружи (кнопка «＋ Реплика» в плеере).
  openRequest?: { index: number; nonce: number } | null;
  /// Почему реплика не распозналась (для дописанных пользователем).
  whyMissed?: (index: number) => string | undefined;
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
  onSaveText,
  rawText,
  gaps = [],
  onPlayRange,
  onRecognizeGap,
  onAddManual,
  onAddAfter,
  openRequest,
  whyMissed,
}: Props) {
  // Правка одной реплики на месте: индекс и черновик текста.
  const [inlineAt, setInlineAt] = useState<number | null>(null);
  const [inlineText, setInlineText] = useState("");
  // Говорящий новой реплики (выбирается прямо в редакторе).
  const [inlineSpeaker, setInlineSpeaker] = useState<string | undefined>(undefined);
  const [busyGap, setBusyGap] = useState<number | null>(null);
  const openInline = (i: number, text: string) => {
    setInlineAt(i);
    setInlineText(text);
    setInlineSpeaker(undefined);
  };
  const commitInline = () => {
    if (inlineAt === null) return;
    onSaveText?.(inlineAt, inlineText, inlineSpeaker);
    setInlineAt(null);
  };
  // Запрос снаружи: открыть правку и показать реплику.
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!openRequest) return;
    openInline(openRequest.index, "");
    requestAnimationFrame(() => {
      try {
        listRef.current
          ?.querySelector(`[data-turn="${openRequest.index}"]`)
          ?.scrollIntoView({ block: "center", behavior: "smooth" });
      } catch {
        // jsdom
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openRequest]);
  const cancelInline = () => {
    // Новая (ещё пустая) реплика, которую передумали писать, — убрать.
    if (inlineAt !== null && !(rawText?.(inlineAt) ?? "").trim()) onSaveText?.(inlineAt, "");
    setInlineAt(null);
  };
  const gapRow = (g: Gap) => (
    <div key={`gap-${g.start}`} className="gap-row" onClick={(e) => e.stopPropagation()}>
      <span className="gap-line" aria-hidden="true" />
      <span className="gap-label">
        Пропуск {clock(g.start)}–{clock(g.end)} · {Math.round(g.end - g.start)} с
      </span>
      {onPlayRange && (
        <button type="button" className="gap-btn" onClick={() => onPlayRange(g.start, g.end)} title="Послушать это место">
          ▶ Послушать
        </button>
      )}
      {onRecognizeGap && (
        <button
          type="button"
          className="gap-btn"
          disabled={busyGap !== null}
          onClick={() => {
            setBusyGap(g.start);
            void onRecognizeGap(g).finally(() => setBusyGap(null));
          }}
          title="Распознать это место заново"
        >
          {busyGap === g.start ? "Распознаю…" : "↻ Распознать"}
        </button>
      )}
      {onAddManual && (
        <button
          type="button"
          className="gap-btn"
          onClick={() => void onAddManual(g).then((i) => i >= 0 && openInline(i, ""))}
          title="Дописать реплику вручную"
        >
          ＋ Дописать
        </button>
      )}
      <span className="gap-line" aria-hidden="true" />
    </div>
  );
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
    // Во время правки (всей ленты или одной реплики) лента не уезжает.
    if (editing || inlineAt !== null) return;
    const el = activeRef.current;
    if (!el) return;
    try {
      el.scrollIntoView({ block: "nearest", behavior: "smooth" });
    } catch {
      // jsdom / unsupported — ignore
    }
  }, [activeIndex, editing, inlineAt]);

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
  // Id для «Новый голос» в новой реплике.
  const newVoiceId = useMemo(() => (transcript ? newSpeakerId(transcript) : null), [transcript]);

  if (!transcript || transcript.segments.length === 0) {
    return <div className="transcript-empty">Расшифровки пока нет.</div>;
  }

  return (
    <div ref={listRef} className={editing ? "transcript editing" : "transcript"}>
      {!editing && gaps.filter((g) => g.after === -1).map(gapRow)}
      {transcript.segments.map((seg, i) => {
        const active = i === activeIndex;
        const name = nameForSpeaker(labels, seg.speaker);
        const idx = speakerIdx(seg.speaker);
        const inline = !editing && inlineAt === i;
        // Новая реплика пользователя (ещё без текста) — с выбором говорящего.
        const isNew = inline && seg.origin === "user" && !seg.text.trim();
        const why = !editing && seg.origin === "user" ? whyMissed?.(i) : undefined;
        return (
          <Fragment key={i}>
          <div
            ref={!editing && active ? activeRef : undefined}
            data-turn={i}
            className={`turn${!editing && active ? " active" : ""}${inline ? " inline-editing" : ""}${seg.origin === "user" ? " user-added" : ""}`}
            onClick={editing || inline ? undefined : () => onSeek(seg.start_secs)}
            title={editing || inline ? undefined : "Перейти к этому моменту"}
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
                {!editing && seg.origin && (
                  <span
                    className={`turn-origin ${seg.origin}`}
                    title={
                      seg.origin === "user"
                        ? "Реплику дописали вы — повторная расшифровка и уточнение её не трогают"
                        : "Текст исправлен вами — повторная расшифровка и уточнение его не трогают"
                    }
                  >
                    {seg.origin === "user" ? "добавлено вами" : "исправлено"}
                  </span>
                )}
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
              ) : inline ? (
                <div className="turn-inline" onClick={(e) => e.stopPropagation()}>
                  {isNew && speakerOptions.length > 0 && (
                    <div className="turn-who-pick" role="radiogroup" aria-label="Кто говорит">
                      <span className="turn-who-label">Кто говорит:</span>
                      {speakerOptions.map((sp) => {
                        const on = (inlineSpeaker ?? seg.speaker) === sp;
                        return (
                          <button
                            key={sp}
                            type="button"
                            role="radio"
                            aria-checked={on}
                            className={on ? "who-chip on" : "who-chip"}
                            onClick={() => setInlineSpeaker(sp)}
                          >
                            <span className="speaker-menu-dot" style={{ background: `var(--spk-${speakerIdx(sp)})` } as CSSProperties} />
                            {nameForSpeaker(labels, sp)}
                          </button>
                        );
                      })}
                      {newVoiceId && (
                        <button
                          type="button"
                          role="radio"
                          aria-checked={inlineSpeaker === newVoiceId}
                          className={inlineSpeaker === newVoiceId ? "who-chip on" : "who-chip"}
                          onClick={() => setInlineSpeaker(newVoiceId)}
                          title="Голоса этого человека ещё нет в списке"
                        >
                          ＋ Новый голос
                        </button>
                      )}
                    </div>
                  )}
                  <textarea
                    className="turn-edit"
                    autoFocus
                    value={inlineText}
                    rows={Math.max(2, Math.ceil(inlineText.length / 60))}
                    placeholder={isNew ? "Впишите, что слышно в записи…" : "Что было сказано…"}
                    onChange={(e) => setInlineText(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Escape") cancelInline();
                      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) commitInline();
                    }}
                  />
                  <div className="turn-inline-actions">
                    <span className="hint">Ctrl/⌘+Enter — сохранить, Esc — отмена. Пустой текст удалит реплику.</span>
                    <button type="button" className="btn ghost btn-sm" onClick={cancelInline}>
                      Отмена
                    </button>
                    <button type="button" className="btn primary btn-sm" onClick={commitInline}>
                      Сохранить
                    </button>
                  </div>
                </div>
              ) : (
                <>
                  <div className="turn-text">{seg.text}</div>
                  {why && <div className="turn-why">Почему не распозналось: {why}</div>}
                </>
              )}
            </div>
            {!editing && !inline && onSaveText && (
              <div className="turn-tools">
              {onAddAfter && (
                <button
                  type="button"
                  className="turn-edit-btn"
                  title="Добавить пропущенную реплику после этой"
                  aria-label="Добавить реплику после этой"
                  onClick={(e) => {
                    e.stopPropagation();
                    const at = onAddAfter(i);
                    if (at >= 0) openInline(at, "");
                  }}
                >
                  ＋
                </button>
              )}
              <button
                type="button"
                className="turn-edit-btn"
                title="Исправить текст реплики"
                aria-label="Исправить текст реплики"
                onClick={(e) => {
                  e.stopPropagation();
                  openInline(i, rawText ? rawText(i) : seg.text);
                }}
              >
                ✎
              </button>
              </div>
            )}
          </div>
          {!editing && gaps.filter((g) => g.after === i).map(gapRow)}
          </Fragment>
        );
      })}
    </div>
  );
}
