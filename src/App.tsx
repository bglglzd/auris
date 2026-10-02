import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import { getSettings } from "./settings";
import { resolveProcesses } from "./autorecord";
import type { Meeting, TranscribeState, TrackLevels } from "./types";
import { Sidebar } from "./components/Sidebar";
import { MemiroMark } from "./components/MemiroMark";
import { MeetingView } from "./components/MeetingView";
import { RecordingMonitor } from "./components/RecordingMonitor";
import { SettingsModal } from "./components/SettingsModal";
import { ImportModal } from "./components/ImportModal";
import { findUpdate, UPDATE_INTERVAL_MS } from "./updater";
import type { UpdateInfo } from "./updater";
import { UpdateDialog } from "./components/UpdateDialog";
import type { MeetingPatch } from "./components/MeetingEditDialog";
import { runAutoAi } from "./aiauto";
import { AI_MODEL_EVENT, syncServerModel } from "./aimodel";
import type { AiModelChange } from "./aimodel";
import { useAppMenu } from "./appmenu";
import { BugReportModal } from "./components/BugReportModal";
import { MacSetup, macSetupPending } from "./components/MacSetup";
import { BUG_REPORT_EVENT } from "./bugreport";
import type { BugDraft } from "./bugreport";
import { isMac } from "./platform";

type ProgressEvent = {
  id: string;
  stage: string;
  percent: number;
  done: number;
  total: number;
};

export default function App() {
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [recording, setRecording] = useState(false);
  const [paused, setPaused] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [levels, setLevels] = useState<TrackLevels>({ mic: 0, system: 0 });
  const [showImport, setShowImport] = useState(false);
  // Боковая панель как выезжающее меню на узких экранах (телефон).
  const [navOpen, setNavOpen] = useState(false);
  // Соло-режим «я один»: запоминаем выбор между сессиями.
  const [solo, setSolo] = useState(
    () => localStorage.getItem("3uxo.solo.pref") === "1",
  );
  // Состояние расшифровок по id — живёт на уровне приложения.
  const [trans, setTrans] = useState<Record<string, TranscribeState>>({});

  const refresh = useCallback(async () => {
    setMeetings(await api.listMeetings());
    const st = await api.recordingState();
    setRecording(st.recording);
    setPaused(st.paused);
  }, []);

  const changeSolo = useCallback((v: boolean) => {
    setSolo(v);
    localStorage.setItem("3uxo.solo.pref", v ? "1" : "0");
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // Обновления: проверка при запуске и каждые 6 часов. Найдено — спрашиваем;
  // «Позже» откладывает эту версию до следующего запуска.
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [skipped, setSkipped] = useState<string | null>(null);
  const skippedRef = useRef<string | null>(null);
  skippedRef.current = skipped;
  const checkUpdates = useCallback(async (manual = false) => {
    const u = await findUpdate();
    if (u && (manual || u.version !== skippedRef.current)) setUpdate(u);
    return u;
  }, []);
  // Модель на ИИ-сервере: сверяем при запуске и вместе с проверкой обновлений.
  const [modelNote, setModelNote] = useState<AiModelChange | null>(null);
  useEffect(() => {
    const on = (e: Event) => setModelNote((e as CustomEvent<AiModelChange>).detail);
    window.addEventListener(AI_MODEL_EVENT, on);
    return () => window.removeEventListener(AI_MODEL_EVENT, on);
  }, []);
  useEffect(() => {
    if (!modelNote) return;
    const t = setTimeout(() => setModelNote(null), 12000);
    return () => clearTimeout(t);
  }, [modelNote]);

  // Запись стартовала неполной (macOS без доступа к системному звуку).
  const [recWarn, setRecWarn] = useState<string | null>(null);
  useEffect(() => {
    const un = listen<string>("recording-warning", (e) => setRecWarn(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  // Mac: «Подготовка» при первом запуске (разрешения по шагам) и из Настроек.
  const [macSetup, setMacSetup] = useState(() => isMac && macSetupPending());
  useEffect(() => {
    const on = () => setMacSetup(true);
    window.addEventListener("memiro-mac-setup", on);
    return () => window.removeEventListener("memiro-mac-setup", on);
  }, []);

  // «Сообщить об ошибке» — из сайдбара, настроек, меню, трея и баннеров ошибок.
  const [bug, setBug] = useState<Partial<BugDraft> | null>(null);
  useEffect(() => {
    const on = (e: Event) => setBug((e as CustomEvent<Partial<BugDraft>>).detail ?? {});
    window.addEventListener(BUG_REPORT_EVENT, on);
    return () => window.removeEventListener(BUG_REPORT_EVENT, on);
  }, []);
  useAppMenu("bug", () => setBug({}));

  // Строка меню macOS.
  useAppMenu("settings", () => setShowSettings(true));
  useAppMenu("updates", () => void checkUpdates(true));
  useAppMenu("import", () => {
    if (!recording) setShowImport(true);
  });

  useEffect(() => {
    void checkUpdates();
    void syncServerModel();
    const id = setInterval(() => {
      void checkUpdates();
      void syncServerModel();
    }, UPDATE_INTERVAL_MS);
    const onManual = () => void checkUpdates(true);
    window.addEventListener("memiro-check-updates", onManual);
    return () => {
      clearInterval(id);
      window.removeEventListener("memiro-check-updates", onManual);
    };
  }, [checkUpdates]);

  // При запуске применяем сохранённые настройки записи: горячую клавишу
  // (бэкенд по умолчанию ставит Ctrl+Shift+R) и конфиг авто-записи звонков.
  useEffect(() => {
    const s = getSettings();
    api.updateHotkey(s.hotkey).catch(() => {});
    api.setNotifications(s.notifications).catch(() => {});
    api
      .setAutorecord(
        s.autoRecord.enabled,
        resolveProcesses(s.autoRecord.apps),
        s.autoRecord.autoStop,
        s.autoRecord.startDelaySecs,
        s.autoRecord.minKeepSecs,
      )
      .catch(() => {});
  }, []);

  useAppMenu("solo", () => {
    if (!recording) changeSolo(!solo);
  });

  // Сброс таймера при старте новой записи (false → true).
  useEffect(() => {
    if (recording) setElapsed(0);
  }, [recording]);

  // Таймер записи: тикает, пока идёт запись и она не на паузе.
  useEffect(() => {
    if (!recording || paused) return;
    const id = setInterval(() => setElapsed((p) => p + 1), 1000);
    return () => clearInterval(id);
  }, [recording, paused]);

  // Живые уровни дорожек: опрос, пока идёт запись и не на паузе.
  useEffect(() => {
    if (!recording || paused) return;
    const id = setInterval(async () => {
      try {
        setLevels(await api.recordingLevels());
      } catch {
        /* бэкенд недоступен — молча пропускаем кадр */
      }
    }, 60);
    return () => clearInterval(id);
  }, [recording, paused]);

  // События старта/стопа по горячей клавише / трею.
  useEffect(() => {
    const un = listen("recording-changed", () => refresh());
    return () => {
      un.then((f) => f());
    };
  }, [refresh]);

  // Прогресс расшифровки из бэкенда. mic → 0–50%, system → 50–100%.
  useEffect(() => {
    const un = listen<ProgressEvent>("transcribe-progress", (e) => {
      const { id, stage, percent, done, total } = e.payload;
      setTrans((t) => ({
        ...t,
        [id]: { ...t[id], running: true, percent, stage, done, total, error: undefined },
      }));
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const startTranscription = useCallback(
    async (id: string, speakerCount: number | null, soloFlag: boolean, totalVoices = false) => {
      setTrans((t) => ({ ...t, [id]: { running: true, percent: 0 } }));
      try {
        await api.transcribe(id, getSettings().whisper, speakerCount, soloFlag, totalVoices);
        setTrans((t) => ({
          ...t,
          [id]: { running: false, percent: 100, doneToken: (t[id]?.doneToken ?? 0) + 1 },
        }));
        await refresh();
        // Трудные места (шум, перебивания) уточняются в фоне — расшифровка
        // уже готова и доступна.
        void api.refineTranscript(id).catch(() => {});
        // ИИ сам придумывает заголовок и подводит итоги (если подключён).
        const m = await api.getMeeting(id).catch(() => null);
        if (m) {
          void runAutoAi(m).then((changed) => {
            if (changed) void refresh();
          });
        }
      } catch (e) {
        setTrans((t) => ({
          ...t,
          [id]: { running: false, percent: 0, error: String(e) },
        }));
      }
    },
    [refresh],
  );

  const handleStart = async () => {
    const id = await api.startRecording();
    // Помечаем встречу как соло, если включён режим «я один» (фронт читает это
    // при расшифровке — ключ в стиле 3uxo.speakers.*/3uxo.labels.*).
    if (solo && id) localStorage.setItem(`3uxo.solo.${id}`, "1");
    setRecording(true);
    setPaused(false);
  };

  const handleStop = async () => {
    const m = await api.stopRecording();
    setRecWarn(null);
    setRecording(false);
    setPaused(false);
    await refresh();
    if (m?.id) setSelectedId(m.id);
  };

  const handlePause = async () => {
    await api.pauseRecording();
    setPaused(true);
  };

  const handleResume = async () => {
    await api.resumeRecording();
    setPaused(false);
  };

  const handleImported = async (id: string) => {
    await refresh();
    setSelectedId(id);
    setShowImport(false);
  };

  // Правка встречи из меню «⋯» в списке.
  const handleEdit = async (id: string, patch: MeetingPatch) => {
    await api.updateMeetingMeta(id, patch.title, patch.participants, patch.topic);
    await api.updateMeetingNotes(id, patch.notes);
    if (patch.title !== meetings.find((m) => m.id === id)?.title) {
      localStorage.setItem(`3uxo.titleEdited.${id}`, "1");
    }
    await refresh();
  };

  const handleDelete = async (id: string) => {
    await api.deleteMeeting(id);
    if (selectedId === id) setSelectedId(null);
    await refresh();
  };

  const selected = meetings.find((m) => m.id === selectedId) ?? null;

  return (
    <div className="app">
      <button
        className="nav-toggle"
        aria-label={navOpen ? "Закрыть меню" : "Меню"}
        onClick={() => setNavOpen((v) => !v)}
      >
        {navOpen ? "✕" : "☰"}
      </button>

      <Sidebar
        meetings={meetings}
        activeId={selectedId}
        recording={recording}
        paused={paused}
        elapsed={elapsed}
        solo={solo}
        progress={trans}
        open={navOpen}
        onStart={handleStart}
        onStop={handleStop}
        onPause={handlePause}
        onResume={handleResume}
        onSoloChange={changeSolo}
        onImport={() => {
          setNavOpen(false);
          setShowImport(true);
        }}
        onSelect={(id) => {
          setSelectedId(id);
          setNavOpen(false);
        }}
        onDelete={handleDelete}
        onEdit={handleEdit}
        onOpenSettings={() => {
          setNavOpen(false);
          setShowSettings(true);
        }}
      />
      {navOpen && (
        <div className="nav-backdrop" onClick={() => setNavOpen(false)} />
      )}

      <main className="content">
        {isMac && <div className="mac-drag" data-tauri-drag-region />}
        {recording ? (
          <RecordingMonitor levels={levels} solo={solo} />
        ) : selected ? (
          <MeetingView
            key={selected.id}
            meeting={selected}
            transState={trans[selected.id]}
            onTranscribe={(speakerCount, soloFlag, totalVoices) =>
              startTranscription(selected.id, speakerCount, soloFlag, totalVoices)
            }
            onMetaSaved={refresh}
          />
        ) : (
          <div className="empty">
            <div className="empty-mark">
              <span className="ripple-ring" />
              <span className="ripple-ring d1" />
              <span className="ripple-ring d2" />
              <MemiroMark size={72} />
            </div>
            <h2>Выбери встречу</h2>
            <p>
              Нажми «Начать запись», чтобы записать созвон, или выбери встречу
              слева.
            </p>
            <div className="privacy-card">
              <svg
                className="privacy-shield"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <path d="M12 3 4 6v5c0 5 3.4 8 8 10 4.6-2 8-5 8-10V6l-8-3Z" />
                <path d="m9 12 2 2 4-4" />
              </svg>
              <div className="privacy-card-text">
                <strong>Приватно по умолчанию</strong>
                <p>
                  Запись, расшифровка и разделение голосов выполняются{" "}
                  <b>локально на вашем устройстве</b> — аудио и тексты никуда не
                  загружаются. ИИ-функции (итоги, задачи, инструкции для ИИ) — по
                  желанию и через
                  ваш ключ. Открытый код.
                </p>
              </div>
            </div>
          </div>
        )}
      </main>

      {showSettings && <SettingsModal onClose={() => setShowSettings(false)} />}
      <div className="toast-stack">
      {recWarn && (
        <div className="toast warn" role="alert">
          <span className="toast-icon">!</span>
          <span>
            Запись идёт только с микрофона — голоса собеседников не записываются.
            <br />
            {recWarn}
            {isMac && (
              <>
                <br />
                <button
                  className="btn btn-sm toast-action"
                  onClick={() => void api.openPrivacySettings("screen").catch(() => {})}
                >
                  Открыть настройки macOS
                </button>
              </>
            )}
          </span>
          <button className="toast-close" onClick={() => setRecWarn(null)} aria-label="Закрыть">
            ✕
          </button>
        </div>
      )}
      {modelNote && (
        <div className="toast" role="status">
          <span className="toast-icon">✦</span>
          <span>
            Модель ИИ на сервере обновилась: <b>{modelNote.from}</b> → <b>{modelNote.to}</b>.
            Memiro переключился на новую.
          </span>
          <button className="toast-close" onClick={() => setModelNote(null)} aria-label="Закрыть">
            ✕
          </button>
        </div>
      )}
      </div>
      {update && (
        <UpdateDialog
          info={update}
          recording={recording}
          onLater={() => {
            setSkipped(update.version);
            setUpdate(null);
          }}
        />
      )}
      {bug && <BugReportModal prefill={bug} onClose={() => setBug(null)} />}
      {macSetup && <MacSetup onClose={() => setMacSetup(false)} />}
      {showImport && (
        <ImportModal
          onClose={() => setShowImport(false)}
          onImported={handleImported}
        />
      )}
    </div>
  );
}
