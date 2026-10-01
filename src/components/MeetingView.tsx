import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import type { Meeting, ReportKind, Transcript, TranscribeState, TrackFile } from "../types";
import { api } from "../api";
import { getLabels, setLabels as saveLabels, nameForSpeaker } from "../labels";
import type { SpeakerLabels } from "../labels";
import { activeSegmentIndex } from "../playback";
import { clock, transcriptToPlain } from "../export";
import { mergeSpeakers, renumberSpeakers } from "../speakers";
import { REPORTS_EVENT } from "../reports";
import { applyPolicy } from "../profanity";
import { profanityPolicy, SETTINGS_EVENT } from "../settings";
import { TranscriptView } from "./TranscriptView";
import { SpeakersPanel } from "./SpeakersPanel";
import { ExportModal } from "./ExportModal";
import { AudioEditor } from "./AudioEditor";
import { AiPanel } from "./AiPanel";
import { CopyLogButton } from "./CopyLogButton";
import { openBugReport } from "../bugreport";
import { CopyButton } from "./CopyButton";

interface Props {
  meeting: Meeting;
  transState?: TranscribeState;
  onTranscribe: (speakerCount: number | null, solo: boolean) => void;
  onMetaSaved: () => void;
}

/// Русское склонение слова по числу: 1 голос, 2 голоса, 5 голосов.
function plural(n: number, one: string, few: string, many: string): string {
  const mod10 = n % 10;
  const mod100 = n % 100;
  if (mod10 === 1 && mod100 !== 11) return one;
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 10 || mod100 >= 20)) return few;
  return many;
}

export function MeetingView({ meeting, transState, onTranscribe, onMetaSaved }: Props) {
  // Импортированная запись — одна дорожка audio.wav (без разделения «Я/Собеседник»).
  const isImported = meeting.source === "imported";
  // Соло-режим «я один»: помечается при старте записи (см. App.handleStart).
  // Расшифровываем только микрофон, один голос «Я», без диаризации.
  const isSolo =
    !isImported && localStorage.getItem(`3uxo.solo.${meeting.id}`) === "1";

  const micRef = useRef<HTMLAudioElement>(null);
  const sysRef = useRef<HTMLAudioElement>(null);

  const [micUrl, setMicUrl] = useState("");
  const [sysUrl, setSysUrl] = useState("");
  const [transcript, setTranscript] = useState<Transcript | null>(null);
  const [error, setError] = useState("");
  // Правка расшифровки: черновик живёт отдельно, пишется в файл по «Сохранить».
  const [editing, setEditing] = useState(false);
  // Политика нецензурной лексики: расшифровка хранится дословно, показ,
  // копия и экспорт — по политике (меняется в настройках на лету).
  const [policy, setPolicy] = useState(() => profanityPolicy());
  useEffect(() => {
    const on = () => setPolicy(profanityPolicy());
    window.addEventListener(SETTINGS_EVENT, on);
    return () => window.removeEventListener(SETTINGS_EVENT, on);
  }, []);
  const [draft, setDraft] = useState<Transcript | null>(null);

  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  // Отдельный режим — правка аудио (вырезание фрагментов) на таймлайне.
  const [editorOpen, setEditorOpen] = useState(false);
  // Версия файлов дорожек: меняется после правки аудио, чтобы <audio> и
  // длительность перечитались, а не остались в кеше webview.
  const [audioVersion, setAudioVersion] = useState(0);

  const [title, setTitle] = useState(meeting.title);
  const [participants, setParticipants] = useState(meeting.participants);
  const [topic, setTopic] = useState(meeting.topic);
  const [notes, setNotes] = useState(meeting.notes ?? "");
  const [labels, setLbls] = useState<SpeakerLabels>(() => getLabels(meeting.id));

  // Сколько голосов (для разделения): "auto" (по умолчанию) | "1".."8".
  // Импорт — всего говорящих, запись — собеседников (кроме «Я»).
  const [speakerSel, setSpeakerSel] = useState<string>(
    () => localStorage.getItem(`3uxo.speakers.${meeting.id}`) ?? "auto",
  );
  // ИИ-отчёты встречи (общие для ИИ-панели и экспорта).
  const [reports, setReports] = useState<Partial<Record<ReportKind, string>>>({});
  const [showExport, setShowExport] = useState(false);
  // Есть сохранённый анализ голосов → число голосов меняется мгновенно.
  const [hasVoices, setHasVoices] = useState(false);
  const [reclustering, setReclustering] = useState(false);
  const [notice, setNotice] = useState("");
  // Прослушивание образца голоса: где остановиться.
  const stopAtRef = useRef<number | null>(null);
  const updateSpeakerSel = (v: string) => {
    setSpeakerSel(v);
    localStorage.setItem(`3uxo.speakers.${meeting.id}`, v);
  };
  const speakerCountValue = (): number | null =>
    speakerSel === "auto" ? null : Number(speakerSel);

  const updateLabel = (key: string, value: string) => {
    const next = { ...labels, [key]: value };
    setLbls(next);
    saveLabels(meeting.id, next);
  };

  useEffect(() => {
    setTitle(meeting.title);
    setParticipants(meeting.participants);
    setTopic(meeting.topic);
    setNotes(meeting.notes ?? "");
    setTime(0);
    setPlaying(false);
    setError("");
    setEditing(false);
    setDraft(null);
    setLbls(getLabels(meeting.id));
    setEditorOpen(false);
    setNotice("");
    setReports({});
    api.getTranscript(meeting.id).then(setTranscript).catch(() => {});
    api.getReports(meeting.id).then(setReports).catch(() => {});
    api.hasVoiceAnalysis(meeting.id).then(setHasVoices).catch(() => setHasVoices(false));
  }, [meeting.id, isImported]);

  // Встречу поправили извне (меню «⋯» в списке, авто-заголовок) — подтягиваем.
  useEffect(() => setTitle(meeting.title), [meeting.title]);
  useEffect(() => setParticipants(meeting.participants), [meeting.participants]);
  useEffect(() => setTopic(meeting.topic), [meeting.topic]);
  useEffect(() => setNotes(meeting.notes ?? ""), [meeting.notes]);

  // Отчёты обновились в фоне (авто-итоги после расшифровки).
  useEffect(() => {
    const on = (e: Event) => {
      if ((e as CustomEvent<{ id: string }>).detail?.id !== meeting.id) return;
      api.getReports(meeting.id).then(setReports).catch(() => {});
    };
    window.addEventListener(REPORTS_EVENT, on);
    return () => window.removeEventListener(REPORTS_EVENT, on);
  }, [meeting.id]);

  // Ссылки на дорожки. `audioVersion` подмешивает метку версии в asset-URL:
  // после правки аудио путь тот же, и без неё webview отдаёт старый файл.
  useEffect(() => {
    const bust = audioVersion || undefined;
    if (isImported) {
      api.trackUrl(meeting.id, "audio.wav", bust).then(setMicUrl).catch(() => {});
      setSysUrl("");
    } else {
      api.trackUrl(meeting.id, "mic.wav", bust).then(setMicUrl).catch(() => {});
      api
        .trackUrl(meeting.id, "system.wav", bust)
        .then(setSysUrl)
        .catch(() => {});
    }
  }, [meeting.id, isImported, audioVersion]);

  // Перезагрузка расшифровки, когда фоновая задача завершилась.
  useEffect(() => {
    if (transState?.doneToken) {
      // Новая расшифровка перетирает черновик правок — выходим из режима правки.
      setEditing(false);
      setDraft(null);
      setNotice("");
      api.getTranscript(meeting.id).then(setTranscript).catch(() => {});
      api.hasVoiceAnalysis(meeting.id).then(setHasVoices).catch(() => {});
    }
  }, [transState?.doneToken, meeting.id]);

  const transcribing = transState?.running ?? false;
  const percent = transState?.percent ?? 0;
  const stage = transState?.stage;
  const done = transState?.done ?? 0;
  const total = transState?.total ?? 0;
  const stageLabel =
    stage === "download"
      ? "Скачивание модели распознавания (один раз)"
      : stage === "download-voices"
        ? "Скачивание модели голосов (один раз)"
        : stage === "loading"
          ? "Подготовка модели"
          : stage === "diarize"
            ? "Разделение голосов"
            : stage === "system"
              ? "Расшифровка собеседника"
              : isImported
                ? "Расшифровка"
                : isSolo
                  ? "Расшифровка"
                  : "Расшифровка «Я»";
  const downloading = stage === "download" || stage === "download-voices";
  const shownError = error || transState?.error || "";

  const activeIndex = useMemo(
    () => (transcript ? activeSegmentIndex(transcript.segments, time) : -1),
    [transcript, time],
  );

  // Уникальные говорящие в расшифровке (для переименования импортированных).
  const speakers = useMemo(
    () =>
      transcript
        ? Array.from(new Set(transcript.segments.map((s) => s.speaker)))
        : [],
    [transcript],
  );

  const togglePlay = () => {
    stopAtRef.current = null;
    const mic = micRef.current;
    if (!mic) return;
    if (playing) {
      mic.pause();
      sysRef.current?.pause();
      setPlaying(false);
    } else {
      void mic.play();
      void sysRef.current?.play().catch(() => {});
      setPlaying(true);
    }
  };

  // Образец голоса: перемотать к реплике и проиграть только её.
  const playSample = (start: number, end: number) => {
    stopAtRef.current = end;
    seek(start);
    const mic = micRef.current;
    if (!mic) return;
    void mic.play().catch(() => {});
    void sysRef.current?.play().catch(() => {});
    setPlaying(true);
  };

  const seek = (t: number) => {
    if (micRef.current) micRef.current.currentTime = t;
    if (sysRef.current) sysRef.current.currentTime = t;
    setTime(t);
  };

  const saveMeta = async () => {
    // Пользователь поменял заголовок сам — авто-заголовок его больше не трогает.
    if (title !== meeting.title) localStorage.setItem(`3uxo.titleEdited.${meeting.id}`, "1");
    try {
      await api.updateMeetingMeta(meeting.id, title, participants, topic);
      onMetaSaved();
    } catch (e) {
      setError(String(e));
    }
  };

  const saveNotes = async () => {
    if (notes === (meeting.notes ?? "")) return;
    try {
      await api.updateMeetingNotes(meeting.id, notes);
      onMetaSaved();
    } catch (e) {
      setError(String(e));
    }
  };

  // Аудио изменилось в редакторе (вырезали фрагменты или вернули оригинал):
  // перечитываем дорожки, длительность и расшифровку — её времена сдвинулись
  // вместе со звуком; список встреч обновляем ради новой длительности.
  const handleAudioApplied = () => {
    setAudioVersion(Date.now());
    setDuration(0);
    setTime(0);
    setPlaying(false);
    setEditing(false);
    setDraft(null);
    api.getTranscript(meeting.id).then(setTranscript).catch(() => {});
    onMetaSaved();
  };

  const downloadAudio = async (track: TrackFile, label: string) => {
    try {
      const path = await save({
        defaultPath: `${meeting.title || "meeting"} — ${label}.wav`,
        filters: [{ name: "WAV", extensions: ["wav"] }],
      });
      if (path) await api.exportAudio(meeting.id, track, path);
    } catch (e) {
      setError(String(e));
    }
  };

  const nameOf = (id: string) => nameForSpeaker(labels, id);

  // ---- Правка расшифровки ----
  const startEdit = () => {
    if (!transcript) return;
    setDraft({ segments: transcript.segments.map((s) => ({ ...s })) });
    setEditing(true);
  };
  const cancelEdit = () => {
    setEditing(false);
    setDraft(null);
  };
  const editText = (i: number, text: string) =>
    setDraft((d) =>
      d
        ? { segments: d.segments.map((s, j) => (j === i ? { ...s, text } : s)) }
        : d,
    );
  const editSpeaker = (i: number, speaker: string) =>
    setDraft((d) =>
      d
        ? { segments: d.segments.map((s, j) => (j === i ? { ...s, speaker } : s)) }
        : d,
    );
  const deleteSegment = (i: number) =>
    setDraft((d) => (d ? { segments: d.segments.filter((_, j) => j !== i) } : d));
  const saveEdit = async () => {
    if (!draft) return;
    // Пустые после правки реплики убираем (очистка текста = удалить строку).
    const cleaned: Transcript = {
      segments: draft.segments.filter((s) => s.text.trim().length > 0),
    };
    try {
      await api.saveTranscript(meeting.id, cleaned);
      setTranscript(cleaned);
      setEditing(false);
      setDraft(null);
    } catch (e) {
      setError(String(e));
    }
  };

  // ---- Голоса ----
  const changeCount = async (v: string) => {
    updateSpeakerSel(v);
    if (!hasVoices || !transcript) {
      setNotice(
        transcript
          ? "Для этой встречи нет анализа голосов — число применится при «↻ Заново»."
          : "",
      );
      return;
    }
    setReclustering(true);
    setNotice("");
    try {
      const t = await api.reclusterSpeakers(meeting.id, v === "auto" ? null : Number(v));
      // Номера голосов поменялись — старые подписи «Спикер N» к ним не относятся.
      const kept: SpeakerLabels = {};
      for (const [k, val] of Object.entries(labels)) if (!/^spk\d+$/.test(k)) kept[k] = val;
      setLbls(kept);
      saveLabels(meeting.id, kept);
      setTranscript(t);
    } catch (e) {
      setError(String(e));
    } finally {
      setReclustering(false);
    }
  };

  const mergeVoice = async (from: string, into: string) => {
    if (!transcript) return;
    const merged = mergeSpeakers(transcript, from, into);
    const { transcript: t, labels: l } = renumberSpeakers(merged, labels);
    try {
      await api.saveTranscript(meeting.id, t);
      setTranscript(t);
      setLbls(l);
      saveLabels(meeting.id, l);
    } catch (e) {
      setError(String(e));
    }
  };

  const pct = duration > 0 ? (time / duration) * 100 : 0;
  const hasTranscript = !!transcript && transcript.segments.length > 0;
  const shown = useMemo(() => (transcript ? applyPolicy(transcript, policy) : null), [transcript, policy]);

  // Соло-режим расшифровывает только микрофон одним голосом — выбор числа
  // собеседников не нужен. Вызов передаёт флаг соло в бэкенд.
  const doTranscribe = () => onTranscribe(isSolo ? 1 : speakerCountValue(), isSolo);

  const speakerOptions = [1, 2, 3, 4, 5, 6, 7, 8];
  const speakerSelect = isSolo ? (
    <span className="solo-badge" title="Заметка для себя — один голос «Я»">
      🎙 Заметка · один голос
    </span>
  ) : (
    <select
      className="speaker-count"
      value={speakerSel}
      onChange={(e) => updateSpeakerSel(e.target.value)}
      title="Сколько голосов в записи — для разделения говорящих"
    >
      <option value="auto">{isImported ? "Голосов: авто" : "Собеседников: авто"}</option>
      {speakerOptions.map((n) => (
        <option key={n} value={String(n)}>
          {isImported
            ? `${n} ${plural(n, "голос", "голоса", "голосов")}`
            : `${n} ${plural(n, "собеседник", "собеседника", "собеседников")}`}
        </option>
      ))}
    </select>
  );

  // Правка аудио — отдельный интерфейс на всё окно (переключение из плеера).
  if (editorOpen) {
    return (
      <AudioEditor
        meeting={meeting}
        onClose={() => setEditorOpen(false)}
        onApplied={handleAudioApplied}
      />
    );
  }

  return (
    <div className="mv">
      <div className="mv-head">
        <div className="eyebrow">
          {new Date(meeting.created_at).toLocaleString()}{" "}
          <span className="status-tag">{meeting.status}</span>
          <span className="local-badge" title="Расшифровка — локально на устройстве">
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
            >
              <rect x="5" y="11" width="14" height="10" rx="2" />
              <path d="M8 11V8a4 4 0 0 1 8 0v3" />
            </svg>
            Локально
          </span>
        </div>
        <input
          className="mv-title"
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={saveMeta}
          placeholder="Без названия"
        />
        <div className="mv-meta-row">
          <input
            className="chip-input"
            value={participants}
            onChange={(e) => setParticipants(e.target.value)}
            onBlur={saveMeta}
            placeholder="участники"
          />
          <input
            className="chip-input"
            value={topic}
            onChange={(e) => setTopic(e.target.value)}
            onBlur={saveMeta}
            placeholder="тема"
          />
        </div>
        <textarea
          className="mv-notes"
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          onBlur={saveNotes}
          rows={Math.min(8, Math.max(1, notes.split("\n").length))}
          placeholder="🗒 Заметки к встрече — что важно не забыть…"
          aria-label="Заметки к встрече"
        />
      </div>

      {shownError && (
        <div className="ai-error error-banner">
          <span>{shownError}</span>
          <button
            type="button"
            className="link-btn"
            onClick={() => openBugReport({ title: "Ошибка во встрече", what: shownError })}
          >
            Сообщить об ошибке
          </button>
          <CopyLogButton className="btn ghost" />
        </div>
      )}

      <div className="card">
        <div className="player">
          <button
            className="play-btn"
            onClick={togglePlay}
            aria-label={playing ? "Пауза" : "Играть"}
          >
            {playing ? (
              <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                <rect x="6" y="5" width="4" height="14" rx="1.2" />
                <rect x="14" y="5" width="4" height="14" rx="1.2" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                <path d="M8 5.5v13a1 1 0 0 0 1.52.85l10.5-6.5a1 1 0 0 0 0-1.7L9.52 4.65A1 1 0 0 0 8 5.5Z" />
              </svg>
            )}
          </button>
          <div className="scrub">
            <input
              type="range"
              min={0}
              max={duration || 0}
              step={0.1}
              value={time}
              onChange={(e) => seek(parseFloat(e.target.value))}
              style={{ ["--p" as string]: `${pct}%` } as CSSProperties}
            />
            <div className="times">
              <span>{clock(time)}</span>
              <span>{clock(duration)}</span>
            </div>
          </div>
          <div className="track-tools">
            <button
              className="btn ghost small"
              onClick={() => setEditorOpen(true)}
              disabled={transcribing}
              title={
                transcribing
                  ? "Идёт расшифровка — правка аудио будет доступна после неё"
                  : "Редактор аудио: вырезать лишнее из записи"
              }
            >
              ✂ Редактор
            </button>
            {isImported ? (
              <button
                className="btn ghost small"
                onClick={() => downloadAudio("audio.wav", "запись")}
                title="Скачать аудио (WAV)"
              >
                ⬇ Аудио
              </button>
            ) : (
              <>
                <button
                  className="btn ghost small"
                  onClick={() => downloadAudio("mic.wav", "Я")}
                  title="Скачать дорожку «Я» (WAV)"
                >
                  ⬇ «Я»
                </button>
                <button
                  className="btn ghost small"
                  onClick={() => downloadAudio("system.wav", "Собеседник")}
                  title="Скачать дорожку собеседника (WAV)"
                >
                  ⬇ Собеседник
                </button>
              </>
            )}
          </div>
        </div>
        <audio
          ref={micRef}
          src={micUrl || undefined}
          onTimeUpdate={() => {
            const mic = micRef.current;
            if (!mic) return;
            setTime(mic.currentTime);
            // Конец образца голоса — пауза.
            if (stopAtRef.current !== null && mic.currentTime >= stopAtRef.current) {
              stopAtRef.current = null;
              mic.pause();
              sysRef.current?.pause();
              setPlaying(false);
            }
          }}
          onLoadedMetadata={() => {
            const d = micRef.current?.duration;
            if (d && Number.isFinite(d)) setDuration(d);
          }}
          onEnded={() => setPlaying(false)}
          preload="metadata"
        />
        <audio ref={sysRef} src={sysUrl || undefined} preload="metadata" />
      </div>

      <div className="card">
        <div className="card-head">
          <h3>Расшифровка</h3>
          <div className="spacer" />
          {transcribing ? (
            <span className="muted transcribing">
              <span className="spin">◜</span> {stageLabel}…
            </span>
          ) : editing ? (
            <div className="btn-row">
              <button className="btn ghost" onClick={cancelEdit}>
                Отмена
              </button>
              <button
                className="btn primary"
                onClick={saveEdit}
                title="Сохранить правки расшифровки"
              >
                ✓ Сохранить
              </button>
            </div>
          ) : hasTranscript ? (
            <div className="btn-row">
              <button
                className="btn ghost"
                onClick={() => setShowExport(true)}
                title="Word, Markdown, текст или субтитры — со стенограммой и ИИ-отчётами"
              >
                ⬇ Экспорт
              </button>
              <CopyButton
                className="btn ghost icon-btn"
                text={() => transcriptToPlain(shown!, nameOf)}
                label="📋"
                doneLabel="✓"
                title="Скопировать текст расшифровки без Markdown"
              />
              <button
                className="btn ghost icon-btn"
                onClick={startEdit}
                title="Исправить ошибки распознавания"
                aria-label="Редактировать расшифровку"
              >
                ✎
              </button>
              <button
                className="btn ghost icon-btn"
                onClick={doTranscribe}
                title="Расшифровать заново (текущие правки текста пропадут)"
                aria-label="Расшифровать заново"
              >
                ↻
              </button>
            </div>
          ) : (
            <div className="btn-row">
              {speakerSelect}
              <button className="btn primary" onClick={doTranscribe}>
                Расшифровать
              </button>
            </div>
          )}
        </div>
        {transcribing ? (
          <div className="card-body">
            <div className="progress">
              <div className="progress-bar" style={{ width: `${percent}%` }} />
            </div>
            <p className="muted" style={{ marginTop: 10 }}>
              {stageLabel}
              {total > 0 ? ` · фрагмент ${done}/${total}` : ""} ·{" "}
              {Math.round(percent)}%.{" "}
              {downloading
                ? "Модель скачивается только один раз — дальше всё работает офлайн."
                : "Можно открыть другие встречи — расшифровка не прервётся."}
            </p>
          </div>
        ) : (
          <>
          {hasTranscript && !editing && !isSolo && (
            <SpeakersPanel
              transcript={transcript!}
              labels={labels}
              recorded={!isImported}
              count={speakerSel}
              canRecluster={hasVoices}
              busy={reclustering}
              onRename={updateLabel}
              onCount={changeCount}
              onMerge={mergeVoice}
              onPlaySample={playSample}
            />
          )}
          {notice && <p className="hint voices-notice">{notice}</p>}
          <TranscriptView
            transcript={editing ? draft : shown}
            activeIndex={activeIndex}
            labels={labels}
            onSeek={seek}
            editing={editing}
            speakerOptions={speakers}
            onEditText={editText}
            onEditSpeaker={editSpeaker}
            onDeleteSegment={deleteSegment}
          />
          </>
        )}
      </div>

      <AiPanel
        meeting={meeting}
        labels={labels}
        hasTranscript={hasTranscript}
        reports={reports}
        onReport={(kind, text) => setReports((r) => ({ ...r, [kind]: text }))}
        onMetaSaved={onMetaSaved}
      />
      {showExport && (
        <ExportModal
          meeting={meeting}
          transcript={shown}
          reports={reports}
          nameOf={nameOf}
          onClose={() => setShowExport(false)}
        />
      )}
    </div>
  );
}
