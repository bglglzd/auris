import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { getSettings, saveSettings } from "../settings";
import type { AppSettings } from "../types";
import { api } from "../api";
import { AUTO_RECORD_APPS, customProcs, resolveProcesses } from "../autorecord";
import { CopyLogButton } from "./CopyLogButton";
import { HotkeyCapture } from "./HotkeyCapture";
import { ModelsManager } from "./ModelsManager";
import { DEFAULT_MODEL } from "../settings";
import { findUpdate } from "../updater";
import type { AiCheck } from "../types";
import { isMac } from "../platform";
import { MacPermissions } from "./MacPermissions";
import { openBugReport } from "../bugreport";

/// Переключатель-тумблер в стиле Memiro.
function Switch({
  on,
  onChange,
  label,
}: {
  on: boolean;
  onChange: (v: boolean) => void;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={on ? "switch on" : "switch"}
      onClick={() => onChange(!on)}
    >
      <span className="switch-knob" />
    </button>
  );
}

/// Языки речи для выбора в настройках (код Whisper → название).
const SPEECH_LANGUAGES: [string, string][] = [
  ["ru", "Русский"],
  ["en", "English"],
  ["uk", "Українська"],
  ["be", "Беларуская"],
  ["kk", "Қазақша"],
  ["de", "Deutsch"],
  ["fr", "Français"],
  ["es", "Español"],
  ["it", "Italiano"],
  ["pt", "Português"],
  ["pl", "Polski"],
  ["tr", "Türkçe"],
  ["zh", "中文"],
  ["ja", "日本語"],
  ["auto", "Определять автоматически"],
];

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const [s, setS] = useState<AppSettings>(getSettings());
  const [proc, setProc] = useState("");
  const [version, setVersion] = useState("");
  const [updState, setUpdState] = useState("");
  const [aiCheck, setAiCheck] = useState<AiCheck | null>(null);
  const [aiChecking, setAiChecking] = useState(false);

  // Проверка ИИ-сервера по введённым (ещё не сохранённым) параметрам.
  const checkAi = async () => {
    setAiChecking(true);
    setAiCheck(null);
    try {
      const r = await api.aiCheck(s.ai);
      setAiCheck(r);
      // Модель не указана или исчезла с сервера — подставляем актуальную.
      if (r.ok && r.model && (!s.ai.model.trim() || r.changed)) {
        setS((p) => ({ ...p, ai: { ...p.ai, model: r.model } }));
      }
    } catch (e) {
      setAiCheck({ ok: false, models: [], model: "", changed: false, latency_ms: 0, error: String(e) });
    } finally {
      setAiChecking(false);
    }
  };

  // Ручная проверка обновлений: нашлось — App покажет диалог обновления.
  const checkNow = async () => {
    setUpdState("Проверяю…");
    const u = await findUpdate();
    if (u) {
      setUpdState(`Доступна версия ${u.version}`);
      window.dispatchEvent(new Event("memiro-check-updates"));
    } else {
      setUpdState("У вас последняя версия");
    }
  };

  // Версия приложения (из tauri.conf). В dev-превью без Tauri вернёт ошибку —
  // тогда просто не показываем номер.
  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);

  const ai = (k: keyof AppSettings["ai"], v: string) =>
    setS({ ...s, ai: { ...s.ai, [k]: v } });
  const wh = (k: keyof AppSettings["whisper"], v: string) =>
    setS({ ...s, whisper: { ...s.whisper, [k]: v } });
  const ar = (patch: Partial<AppSettings["autoRecord"]>) =>
    setS({ ...s, autoRecord: { ...s.autoRecord, ...patch } });

  const toggleApp = (key: string) => {
    const has = s.autoRecord.apps.includes(key);
    ar({
      apps: has
        ? s.autoRecord.apps.filter((a) => a !== key)
        : [...s.autoRecord.apps, key],
    });
  };

  const addProc = () => {
    const p = proc.trim();
    if (!p || s.autoRecord.apps.includes(p)) {
      setProc("");
      return;
    }
    ar({ apps: [...s.autoRecord.apps, p] });
    setProc("");
  };

  const save = async () => {
    const wasNotify = getSettings().notifications;
    saveSettings(s);
    // Уведомления: включили — пробное уведомление (на Mac заодно запрос).
    void api.setNotifications(s.notifications).catch(() => {});
    if (s.notifications && !wasNotify) void api.testNotification().catch(() => {});
    // Сразу применяем горячую клавишу (рантайм-регистрация).
    try {
      await api.updateHotkey(s.hotkey);
    } catch {
      // регистрация может не удаться (занято/неверно) — настройка всё равно
      // сохранена; пользователь увидит при следующем старте/проверит.
    }
    // Применяем конфиг авто-записи к фоновому монитору.
    try {
      await api.setAutorecord(
        s.autoRecord.enabled,
        resolveProcesses(s.autoRecord.apps),
        s.autoRecord.autoStop,
        s.autoRecord.startDelaySecs,
        s.autoRecord.minKeepSecs,
      );
    } catch {
      // не критично — применится при следующем запуске
    }
    onClose();
  };

  const custom = customProcs(s.autoRecord.apps);

  return (
    <div className="overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2>Настройки</h2>
        <p className="lead">
          Запись и расшифровка работают локально. ИИ — опционально, через ваш
          ключ.
        </p>

        <div className="settings-privacy">
          <svg
            className="sp-shield"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M12 2l8 3v6c0 5-3.5 8.5-8 10-4.5-1.5-8-5-8-10V5z" />
            <path d="M9 12l2 2 4-4" />
          </svg>
          <div className="sp-text">
            <div className="sp-title">
              Хранить всё локально <span className="sp-always">Всегда вкл</span>
            </div>
            <p>
              Записи, расшифровка и разделение голосов не покидают устройство.
              ИИ-функции — опционально, через ваш ключ ниже.
            </p>
          </div>
        </div>

        {/* ---------- Запись / горячая клавиша ---------- */}
        <details className="settings-section" open>
          <summary>
            <span className="sec-title">Запись</span>
            <span className="sec-sub">Горячая клавиша старт/стоп</span>
            <span className="sec-chev" aria-hidden="true">
              ⌄
            </span>
          </summary>
          <div className="sec-body">
            <div className="field">
              <label>Глобальная горячая клавиша</label>
              <HotkeyCapture
                value={s.hotkey}
                onChange={(v) => setS({ ...s, hotkey: v })}
              />
              <span className="hint">
                Работает в любом приложении: нажми — начнётся запись, нажми ещё
                раз — остановится. Также доступно из значка{" "}
                {isMac ? "в строке меню" : "в трее"}.
              </span>
            </div>
            <div className="row-switch">
              <div>
                <div className="row-switch-title">Уведомления о записи</div>
                <div className="hint">
                  Системные уведомления о начале и конце записи — удобно, когда
                  запись запускают горячей клавишей.
                  {isMac ? " macOS спросит разрешение." : ""}
                </div>
              </div>
              <Switch
                on={s.notifications}
                onChange={(v) => setS({ ...s, notifications: v })}
                label="Уведомления о записи"
              />
            </div>
            {isMac && (
              <MacPermissions
                onOpenSetup={() => window.dispatchEvent(new Event("memiro-mac-setup"))}
              />
            )}
          </div>
        </details>

        {/* ---------- Авто-запись звонков (Windows: детектор звонков по WASAPI) ---------- */}
        {!isMac && (
        <details className="settings-section" open={s.autoRecord.enabled}>
          <summary>
            <span className="sec-title">Авто-запись звонков</span>
            <span className="sec-sub">Старт записи при звонке в мессенджерах</span>
            <span className="sec-chev" aria-hidden="true">
              ⌄
            </span>
          </summary>
          <div className="sec-body">
            <div className="row-switch">
              <div>
                <div className="row-switch-title">
                  Автоматически записывать звонки
                </div>
                <div className="hint">
                  Memiro следит за выбранными приложениями и сам начинает запись,
                  когда начинается звонок.
                </div>
              </div>
              <Switch
                on={s.autoRecord.enabled}
                onChange={(v) => ar({ enabled: v })}
                label="Автоматически записывать звонки"
              />
            </div>

            <div
              className={
                s.autoRecord.enabled ? "app-picker" : "app-picker disabled"
              }
            >
              <div className="app-grid">
                {AUTO_RECORD_APPS.map((app) => {
                  const checked = s.autoRecord.apps.includes(app.key);
                  return (
                    <label
                      key={app.key}
                      className={checked ? "app-item checked" : "app-item"}
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        disabled={!s.autoRecord.enabled}
                        onChange={() => toggleApp(app.key)}
                      />
                      <span className="app-check" aria-hidden="true" />
                      <span className="app-label">{app.label}</span>
                      {app.browser && <span className="app-tag">браузер</span>}
                    </label>
                  );
                })}
              </div>

              {custom.length > 0 && (
                <div className="custom-procs">
                  {custom.map((p) => (
                    <span className="proc-chip" key={p}>
                      {p}
                      <button
                        type="button"
                        onClick={() => toggleApp(p)}
                        aria-label={`Убрать ${p}`}
                      >
                        ✕
                      </button>
                    </span>
                  ))}
                </div>
              )}

              <div className="proc-add">
                <input
                  value={proc}
                  onChange={(e) => setProc(e.target.value)}
                  disabled={!s.autoRecord.enabled}
                  placeholder="Свой процесс, напр. Viber.exe"
                  onKeyDown={(e) => {
                    if (e.key === "Enter") addProc();
                  }}
                />
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={!s.autoRecord.enabled || !proc.trim()}
                  onClick={addProc}
                >
                  Добавить
                </button>
              </div>

              <div className="row-switch slim">
                <div className="row-switch-title">
                  Останавливать запись по завершении звонка
                </div>
                <Switch
                  on={s.autoRecord.autoStop}
                  onChange={(v) => ar({ autoStop: v })}
                  label="Останавливать запись по завершении звонка"
                />
              </div>

              <div className="field-row">
                <div className="field">
                  <label>Задержка перед стартом, сек</label>
                  <input
                    type="number"
                    min={0}
                    max={60}
                    value={s.autoRecord.startDelaySecs}
                    disabled={!s.autoRecord.enabled}
                    onChange={(e) =>
                      ar({ startDelaySecs: Math.max(0, Number(e.target.value) || 0) })
                    }
                  />
                  <span className="hint">
                    Звонок должен длиться столько секунд подряд, прежде чем
                    начнётся запись. Отсекает короткие звуки уведомлений
                    (Telegram «дзынь»). 0 — старт сразу.
                  </span>
                </div>
                <div className="field">
                  <label>Отбрасывать записи короче, сек</label>
                  <input
                    type="number"
                    min={0}
                    max={120}
                    value={s.autoRecord.minKeepSecs}
                    disabled={!s.autoRecord.enabled}
                    onChange={(e) =>
                      ar({ minKeepSecs: Math.max(0, Number(e.target.value) || 0) })
                    }
                  />
                  <span className="hint">
                    Авто-записи короче порога удаляются как мусорные огрызки
                    уведомлений. 0 — не удалять.
                  </span>
                </div>
              </div>

              <div className="hint note">
                Детект звонка — по активной аудио-сессии приложения (Windows).
                Для звонков в браузере (Meet, Телемост) учитывается активный
                микрофон браузера.
              </div>
            </div>
          </div>
        </details>
        )}

        {/* ---------- Распознавание (Whisper) ---------- */}
        <details className="settings-section">
          <summary>
            <span className="sec-title">Распознавание</span>
            <span className="sec-sub">Модели · локально, офлайн</span>
            <span className="sec-chev" aria-hidden="true">
              ⌄
            </span>
          </summary>
          <div className="sec-body">
            <p className="hint">
              Расшифровка и разделение голосов идут локально. Модели скачиваются
              один раз (можно заранее — здесь) и дальше работают без интернета.
            </p>
            <div className="field">
              <label>Модель распознавания</label>
              <ModelsManager
                selected={s.whisper.model || DEFAULT_MODEL}
                onSelect={(id) => wh("model", id)}
              />
            </div>
            <div className="field">
              <label htmlFor="asr-lang">Основной язык речи</label>
              <select id="asr-lang" value={s.whisper.language || "ru"} onChange={(e) => wh("language", e.target.value)}>
                {SPEECH_LANGUAGES.map(([code, name]) => (
                  <option key={code} value={code}>
                    {name}
                  </option>
                ))}
                {!SPEECH_LANGUAGES.some(([c]) => c === (s.whisper.language || "ru")) && (
                  <option value={s.whisper.language}>{s.whisper.language}</option>
                )}
              </select>
              <span className="hint">
                Приоритетный язык: если фрагмент распознался на другом языке (латиница вместо русского),
                Memiro перепроверит его на слух и оставит другой язык, только если он там действительно
                звучит. «Определять автоматически» — для встреч на нескольких языках.
              </span>
            </div>
            <div className="field">
              <label htmlFor="asr-vocab">Словарь: термины, имена, команды</label>
              <textarea
                id="asr-vocab"
                rows={3}
                value={s.whisper.vocabulary ?? ""}
                onChange={(e) => wh("vocabulary", e.target.value)}
                placeholder={"Например:\nJira, Kubernetes, CI/CD\nАлексей Петров, ООО «Вектор»"}
              />
              <span className="hint">
                По строке или через запятую. Memiro пишет эти слова так, как здесь (например, «джира» →
                Jira). Популярные сервисы — Jira, Slack, GitHub, Figma и др. — уже знает. Словарь хранится
                только на этом устройстве.
              </span>
            </div>
            <details className="adv">
              <summary>Использовать свой whisper (необязательно)</summary>
              <div className="field">
                <label>Путь к whisper-CLI</label>
                <input
                  value={s.whisper.whisperPath}
                  onChange={(e) => wh("whisperPath", e.target.value)}
                  placeholder="напр. C:\\tools\\whisper-cli.exe"
                />
                <span className="hint">
                  Если задано — используется он вместо встроенного движка.
                </span>
              </div>
            </details>
          </div>
        </details>

        {/* ---------- Искусственный интеллект ---------- */}
        <details className="settings-section">
          <summary>
            <span className="sec-title">Искусственный интеллект</span>
            <span className="sec-sub">Итоги, задачи, разбор · через ваш ключ</span>
            <span className="sec-chev" aria-hidden="true">
              ⌄
            </span>
          </summary>
          <div className="sec-body">
            <div className="field">
              <label>Base URL</label>
              <input
                value={s.ai.base_url}
                onChange={(e) => ai("base_url", e.target.value)}
                placeholder="http://ai-pc.lan:18080/v1"
              />
            </div>
            <div className="field">
              <label>API-ключ</label>
              <input
                type="password"
                value={s.ai.api_key}
                onChange={(e) => ai("api_key", e.target.value)}
                placeholder="sk-no-key-required"
              />
              <span className="hint">
                Если ключ не нужен — впиши любой, напр. sk-no-key-required
              </span>
            </div>
            <div className="field">
              <label>Модель</label>
              <div className="input-with-btn">
                <input
                  value={s.ai.model}
                  onChange={(e) => ai("model", e.target.value)}
                  placeholder="пусто — та, что сейчас на сервере"
                />
                <button
                  type="button"
                  className="btn"
                  onClick={checkAi}
                  disabled={aiChecking || !s.ai.base_url || !s.ai.api_key}
                  title="Проверить сервер и получить список моделей"
                >
                  {aiChecking ? "Проверяю…" : "Проверить подключение"}
                </button>
              </div>
              {aiCheck &&
                (aiCheck.ok ? (
                  <div className="ai-check ok">
                    <span>
                      ✓ Сервер отвечает · {aiCheck.latency_ms} мс
                      {aiCheck.changed && " · модель на сервере обновилась — выбрана актуальная"}
                    </span>
                    {aiCheck.models.length > 0 && (
                      <div className="model-chips">
                        {aiCheck.models.map((m) => (
                          <button
                            key={m}
                            type="button"
                            className={m === s.ai.model ? "seg-btn on" : "seg-btn"}
                            onClick={() => ai("model", m)}
                          >
                            {m}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                ) : (
                  <div className="ai-check err">✕ Не удалось подключиться: {aiCheck.error}</div>
                ))}
            </div>
            <div className="row-switch">
              <div>
                <div className="row-switch-title">Следить за моделью сервера</div>
                <div className="hint">
                  Если на сервере выкатили новую модель, Memiro сам переключится на неё
                  (проверка при запуске и каждые 6 часов) и сообщит об этом.
                </div>
              </div>
              <Switch
                on={s.aiAuto.followModel}
                onChange={(v) => setS({ ...s, aiAuto: { ...s.aiAuto, followModel: v } })}
                label="Следить за моделью сервера"
              />
            </div>
            <div className="row-switch">
              <div>
                <div className="row-switch-title">Заголовок сам</div>
                <div className="hint">
                  После расшифровки ИИ придумает заголовок, участников и тему (если
                  вы не меняли заголовок вручную).
                </div>
              </div>
              <Switch
                on={s.aiAuto.title}
                onChange={(v) => setS({ ...s, aiAuto: { ...s.aiAuto, title: v } })}
                label="Авто-заголовок"
              />
            </div>
            <div className="row-switch">
              <div>
                <div className="row-switch-title">Итоги встречи сразу</div>
                <div className="hint">
                  После расшифровки ИИ сразу подведёт итоги: главное, решения,
                  задачи.
                </div>
              </div>
              <Switch
                on={s.aiAuto.summary}
                onChange={(v) => setS({ ...s, aiAuto: { ...s.aiAuto, summary: v } })}
                label="Авто-итоги"
              />
            </div>
          </div>
        </details>

        {/* ---------- Диагностика ---------- */}
        <details className="settings-section">
          <summary>
            <span className="sec-title">Ошибки и диагностика</span>
            <span className="sec-sub">Сообщить об ошибке, лог для поддержки</span>
            <span className="sec-chev" aria-hidden="true">
              ⌄
            </span>
          </summary>
          <div className="sec-body">
            <p className="hint">
              Что-то работает не так? Сообщите об ошибке — отчёт попадёт
              разработчикам, ошибки разбираются и исправляются по плану. К отчёту
              можно приложить диагностику: версию, систему и журнал (без ключей и
              текста разговоров).
            </p>
            <div className="btn-row">
              <button type="button" className="btn primary" onClick={() => openBugReport()}>
                Сообщить об ошибке
              </button>
              <CopyLogButton className="btn" />
            </div>
          </div>
        </details>

        <div className="modal-actions">
          <span className="modal-version">
            {version ? `Memiro AI v${version}` : "Memiro AI"}{" "}
            <button type="button" className="link-btn" onClick={checkNow}>
              {updState || "Проверить обновления"}
            </button>
          </span>
          <div className="modal-actions-btns">
            <button className="btn ghost" onClick={onClose}>
              Отмена
            </button>
            <button className="btn primary" onClick={save}>
              Сохранить
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
