/// Список встреч: папки и объединение записей — чистая логика.

import type { Collection, Meeting } from "./types";

export interface FolderGroup {
  collection: Collection;
  meetings: Meeting[];
}

/// Встречи по папкам (папки — по имени) и вне папок; порядок встреч внутри —
/// как в исходном списке. Встреча из несуществующей папки — вне папок.
export function groupMeetings(
  meetings: Meeting[],
  collections: Collection[],
): { folders: FolderGroup[]; loose: Meeting[] } {
  const known = new Set(collections.map((c) => c.id));
  const folders = [...collections]
    .sort((a, b) => a.name.localeCompare(b.name, "ru"))
    .map((collection) => ({ collection, meetings: meetings.filter((m) => m.collection === collection.id) }));
  const loose = meetings.filter((m) => !m.collection || !known.has(m.collection));
  return { folders, loose };
}

/// Порядок склейки: как выбирали (1, 2, 3…) или по времени записи (ранние раньше).
export function mergeOrder(selected: string[], meetings: Meeting[], byTime: boolean): string[] {
  if (!byTime) return [...selected];
  const at = new Map(meetings.map((m) => [m.id, m.created_at]));
  return [...selected].sort((a, b) => (at.get(a) ?? "").localeCompare(at.get(b) ?? ""));
}

/// Переключает запись в выборе (порядок нажатия сохраняется).
export function toggleSelected(selected: string[], id: string): string[] {
  return selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id];
}
