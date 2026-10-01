import { useEffect, useRef } from "react";

/// Пункты строки меню macOS, которые ведёт фронтенд (см. `setup_mac_menu`).
export type AppMenuId = "settings" | "updates" | "solo" | "import" | "find" | "theme" | "bug";

const EVENT = "memiro-menu";
let started = false;

/// Подписывается на событие бэкенда `app-menu` и пересылает его в окно как
/// DOM-событие — компоненты ловят нужные пункты через `useAppMenu`.
export async function initAppMenu(): Promise<void> {
  if (started) return;
  started = true;
  try {
    const { listen } = await import("@tauri-apps/api/event");
    await listen<string>("app-menu", (e) => emitAppMenu(e.payload as AppMenuId));
  } catch {
    /* вне Tauri (vite dev, тесты) — меню нет */
  }
}

export function emitAppMenu(id: AppMenuId): void {
  window.dispatchEvent(new CustomEvent<AppMenuId>(EVENT, { detail: id }));
}

/// Вызывает `handler`, когда в строке меню выбран пункт `id`.
export function useAppMenu(id: AppMenuId, handler: () => void): void {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => {
    const on = (e: Event) => {
      if ((e as CustomEvent<AppMenuId>).detail === id) ref.current();
    };
    window.addEventListener(EVENT, on);
    return () => window.removeEventListener(EVENT, on);
  }, [id]);
}
