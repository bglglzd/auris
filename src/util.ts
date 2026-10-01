/// Форматирует секунды в "m:ss".
export function formatClock(secs: number): string {
  const safe = Math.max(0, Math.floor(secs));
  const m = Math.floor(safe / 60);
  const s = safe % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/// Сколько колонок дать сетке из `n` одинаковых плашек, чтобы ряды были
/// ровными (без пустых мест): наибольший делитель `n`, который помещается
/// в ширину. Если такой делитель слишком мал по сравнению с тем, что
/// помещается (n — простое), — сколько помещается.
export function balancedColumns(n: number, width: number, minItem: number, gap: number): number {
  if (n <= 1) return 1;
  const fit = Math.max(1, Math.floor((width + gap) / (minItem + gap)));
  if (fit >= n) return n;
  let best = 1;
  for (let c = fit; c >= 1; c--) {
    if (n % c === 0) {
      best = c;
      break;
    }
  }
  return best * 2 <= fit && best < n ? fit : best;
}
