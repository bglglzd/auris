import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type { MacPermissionsState, PermStatus } from "../types";

/// Текущий статус разрешений macOS; обновляется раз в 1.5 с, пока виден
/// (ответ на системный запрос приходит асинхронно) и при возврате в окно.
export function useMacPermissions(): [MacPermissionsState | null, () => void] {
  const [state, setState] = useState<MacPermissionsState | null>(null);
  const refresh = useCallback(() => {
    api
      .macPermissions()
      .then(setState)
      .catch(() => setState(null));
  }, []);
  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 1500);
    window.addEventListener("focus", refresh);
    return () => {
      clearInterval(id);
      window.removeEventListener("focus", refresh);
    };
  }, [refresh]);
  return [state, refresh];
}

function Dot({ status }: { status: PermStatus | undefined }) {
  const cls = status === "granted" ? "perm-dot ok" : status === "denied" ? "perm-dot bad" : "perm-dot";
  return <span className={cls} aria-hidden="true" />;
}

interface RowProps {
  title: string;
  hint: string;
  status: PermStatus | undefined;
  onRequest: () => void;
  onSettings: () => void;
}

function PermRow({ title, hint, status, onRequest, onSettings }: RowProps) {
  return (
    <div className="perm-row">
      <Dot status={status} />
      <div className="perm-text">
        <div className="perm-title">{title}</div>
        <div className="hint">{hint}</div>
      </div>
      {status === "granted" ? (
        <span className="perm-ok">✓ Разрешено</span>
      ) : status === "denied" ? (
        <button className="btn btn-sm" onClick={onSettings}>
          Открыть настройки…
        </button>
      ) : (
        <button className="btn btn-sm primary" onClick={onRequest}>
          Разрешить
        </button>
      )}
    </div>
  );
}

/// Два разрешения для записи на Mac: микрофон и звук собеседников. Каждая
/// кнопка показывает системный запрос macOS; при отказе ведёт в Настройки.
export function MacPermissionRows({ state, onChange }: { state: MacPermissionsState | null; onChange: () => void }) {
  const audioMode = state?.system_audio_mode ?? "audio";
  const [asked, setAsked] = useState(false);
  return (
    <>
      <PermRow
        title="Микрофон — ваш голос"
        hint={
          state?.mic === "granted"
            ? "Ваш голос записывается в отдельную дорожку «Я»."
            : state?.mic === "denied"
              ? "Доступ запрещён — включите Memiro AI в Настройках → Конфиденциальность и безопасность → Микрофон."
              : "macOS спросит доступ к микрофону."
        }
        status={state?.mic}
        onRequest={() => void api.requestMicAccess().finally(onChange)}
        onSettings={() => void api.openPrivacySettings("mic").catch(() => {})}
      />
      <PermRow
        title="Звук собеседников"
        hint={
          state?.system_audio === "granted"
            ? "Голоса собеседников записываются в отдельную дорожку. Экран не записывается."
            : state?.system_audio === "denied"
            ? audioMode === "audio"
              ? "Доступ запрещён — включите Memiro AI в Настройках → Запись экрана и системного звука → «Только запись системного звука»."
              : "Доступ запрещён — включите Memiro AI в Настройках → Запись экрана и системного звука, затем перезапустите Memiro."
            : audioMode === "audio"
              ? "macOS спросит «Только запись системного звука». Экран не записывается."
              : "macOS спросит «Запись экрана и системного звука» — Memiro записывает только звук. После разрешения перезапустите Memiro."
        }
        status={state?.system_audio}
        onRequest={() => {
          setAsked(true);
          void api.requestSystemAudioAccess().finally(onChange);
          // На macOS 13–14.1 системный запрос показывается один раз — сразу
          // открываем нужный раздел Настроек.
          if (audioMode === "screen") void api.openPrivacySettings("screen").catch(() => {});
        }}
        onSettings={() => void api.openPrivacySettings("screen").catch(() => {})}
      />
      {asked && state?.system_audio === "unknown" && (
        <p className="hint">
          Если macOS не показала запрос —{" "}
          <button type="button" className="link-btn" onClick={() => void api.openPrivacySettings("screen").catch(() => {})}>
            откройте Настройки
          </button>{" "}
          и включите Memiro AI в «Запись экрана и системного звука».
        </p>
      )}
    </>
  );
}

/// Блок «Доступы macOS» в Настройках → Запись.
export function MacPermissions({ onOpenSetup }: { onOpenSetup?: () => void }) {
  const [state, refresh] = useMacPermissions();
  return (
    <div className="field mac-perms">
      <label>Доступы macOS</label>
      <MacPermissionRows state={state} onChange={refresh} />
      {onOpenSetup && (
        <button type="button" className="link-btn" onClick={onOpenSetup}>
          Пошаговая подготовка Mac…
        </button>
      )}
    </div>
  );
}
