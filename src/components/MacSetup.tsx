import { useState } from "react";
import { api } from "../api";
import { getSettings, saveSettings } from "../settings";
import { MacPermissionRows, useMacPermissions } from "./MacPermissions";
import { MemiroMark } from "./MemiroMark";

export const MAC_SETUP_KEY = "3uxo.macsetup.done";

/// Показывать ли «Подготовку Mac» при запуске (ещё не проходили).
export function macSetupPending(): boolean {
  try {
    return !localStorage.getItem(MAC_SETUP_KEY);
  } catch {
    return false;
  }
}

/// Первый запуск на Mac: два разрешения по шагам, с пояснением, — чтобы
/// системные запросы не появлялись внезапно посреди звонка. Уведомления —
/// по желанию (по умолчанию выключены).
export function MacSetup({ onClose }: { onClose: () => void }) {
  const [state, refresh] = useMacPermissions();
  const [notify, setNotify] = useState(() => getSettings().notifications);
  const ready = state?.mic === "granted" && state?.system_audio === "granted";

  const finish = () => {
    try {
      localStorage.setItem(MAC_SETUP_KEY, "1");
    } catch {
      /* не критично — покажем ещё раз */
    }
    onClose();
  };

  const toggleNotify = (on: boolean) => {
    setNotify(on);
    saveSettings({ ...getSettings(), notifications: on });
    void api.setNotifications(on).catch(() => {});
    if (on) void api.testNotification().catch(() => {});
  };

  return (
    <div className="overlay">
      <div className="modal mac-setup" role="dialog" aria-label="Подготовка Mac">
        <div className="mac-setup-head">
          <MemiroMark size={34} />
          <div>
            <h2>Подготовка Mac</h2>
            <p className="lead">
              Два разрешения — и Memiro готов записывать звонки. Звук обрабатывается только на этом Mac и никуда не
              отправляется.
            </p>
          </div>
        </div>

        <div className="mac-steps">
          <MacPermissionRows state={state} onChange={refresh} />
        </div>

        <label className="bug-check mac-notify">
          <input type="checkbox" checked={notify} onChange={(e) => toggleNotify(e.target.checked)} />
          Уведомления о начале и конце записи (необязательно — macOS спросит разрешение)
        </label>

        <div className="modal-actions">
          <span className="hint">{ready ? "Всё готово ✓" : "Можно вернуться к этому в Настройках → Запись."}</span>
          <div className="modal-actions-btns">
            {!ready && (
              <button className="btn ghost" onClick={finish}>
                Позже
              </button>
            )}
            <button className="btn primary" onClick={finish}>
              {ready ? "Начать" : "Готово"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
