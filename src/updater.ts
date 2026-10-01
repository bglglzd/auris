import { check } from "@tauri-apps/plugin-updater";
import type { Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { logError, logInfo } from "./log";
import { platform as currentPlatform } from "./platform";
import type { Platform } from "./platform";

/// Как часто перепроверять обновления, пока приложение открыто.
export const UPDATE_INTERVAL_MS = 6 * 60 * 60 * 1000;

type NotesTarget = "macos" | "windows" | null;

/// Платформа, к которой относится заголовок или метка строки заметок.
function noteTarget(label: string): NotesTarget {
  const l = label.toLowerCase();
  if (/\b(macos|mac os|mac)\b/.test(l)) return "macos";
  if (/\b(windows|win)\b/.test(l)) return "windows";
  return null;
}

/// Заметки релиза только для этой платформы. Разметка заметок:
/// заголовок «### macOS» / «### Windows» открывает раздел платформы (сам
/// заголовок не показывается), любой другой заголовок — общий раздел;
/// строки с меткой «[mac]» / «[win]» (можно после «- » / «• ») — только для
/// своей платформы. Пусто после отбора — общая фраза.
export function notesForPlatform(notes: string, p: Platform = currentPlatform): string {
  if (!notes.trim()) return "";
  const out: string[] = [];
  let section: NotesTarget = null;
  for (const line of notes.split("\n")) {
    const h = /^\s*#{1,6}\s+(.*)$/.exec(line);
    if (h) {
      section = noteTarget(h[1]);
      if (section === null) out.push(line);
      continue;
    }
    if (section !== null && section !== p) continue;
    const tag = /^(\s*(?:[-*•]\s+)?)\[(mac|macos|win|windows)\]\s*/i.exec(line);
    if (tag) {
      if (noteTarget(tag[2]) !== p) continue;
      out.push(tag[1] + line.slice(tag[0].length));
      continue;
    }
    out.push(line);
  }
  const text = out.join("\n").replace(/\n{3,}/g, "\n\n").trim();
  // Остались только общие заголовки без пунктов — для этой платформы нечего сказать.
  const meaningful = text.split("\n").some((l) => l.trim() && !/^\s*#{1,6}\s/.test(l));
  return meaningful ? text : "Исправления и улучшения стабильности.";
}

/// Найденное обновление для интерфейса.
export interface UpdateInfo {
  version: string;
  currentVersion: string;
  /// Что нового (тело релиза), может быть пустым.
  notes: string;
  date?: string;
  /// Ссылка на объект апдейтера — для установки после согласия.
  handle: Update;
}

/// Проверяет, есть ли новая версия. Ничего не ставит — только сообщает.
/// Ошибки (нет сети, апдейтер недоступен в превью) → null.
export async function findUpdate(): Promise<UpdateInfo | null> {
  try {
    const u = await check();
    if (!u) return null;
    logInfo(`update available: ${u.currentVersion} → ${u.version}`);
    return {
      version: u.version,
      currentVersion: u.currentVersion,
      notes: notesForPlatform(u.body ?? ""),
      date: u.date,
      handle: u,
    };
  } catch (e) {
    console.warn("update check failed", e);
    return null;
  }
}

/// Скачивает и ставит обновление (после согласия пользователя), сообщая
/// прогресс 0..100 (или -1, если размер неизвестен), затем перезапускает.
export async function installUpdate(
  info: UpdateInfo,
  onProgress: (percent: number) => void,
): Promise<void> {
  let total = 0;
  let got = 0;
  try {
    await info.handle.downloadAndInstall((ev) => {
      if (ev.event === "Started") {
        total = ev.data.contentLength ?? 0;
        onProgress(total ? 0 : -1);
      } else if (ev.event === "Progress") {
        got += ev.data.chunkLength;
        onProgress(total ? Math.min(100, (got / total) * 100) : -1);
      } else if (ev.event === "Finished") {
        onProgress(100);
      }
    });
  } catch (e) {
    logError("update install", e);
    throw e;
  }
  await relaunch();
}
