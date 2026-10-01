/// Нецензурная лексика: политика «скрывать» заменяет мат пометкой
/// «[нецензурно]». Расшифровка хранится дословно — замена при показе, копии,
/// экспорте и отправке ИИ, поэтому политику можно сменить в любой момент.
/// Та же логика — в ядре (`core/src/profanity.rs`); правила менять в обоих.

import type { Transcript } from "./types";

export type ProfanityPolicy = "censor" | "verbatim";
export const MARK = "[нецензурно]";

const HUI_PREFIXES = ["", "по", "на", "за", "от", "до", "вы", "из", "ис", "об", "о", "при", "про", "раз", "рас", "под", "пере", "недо", "ни", "не", "а", "охуе"];
const HUI_ROOTS = ["хуй", "хуе", "хуя", "хуи", "хую", "хул"];
const EB_PREFIXES = ["", "вы", "за", "на", "по", "от", "отъ", "до", "у", "при", "про", "пере", "недо", "долбо", "съ", "въ", "взъ", "объ", "подъ", "изъ", "разъ", "раз"];
const CONTAINS = ["пизд", "залуп", "гандон", "гондон", "бляд", "блят", "мандавош", "motherfuck", "fuck"];
const EXACT = new Set(["бля", "блять", "сука", "суки", "суку", "сукой", "сучара", "cunt", "asshole", "bullshit"]);
const STARTS = ["пидор", "пидар", "пидр", "мудак", "мудил", "мудач", "мудозвон", "шлюх", "сукин", "shit", "bitch"];

/// Матерное ли слово (регистр не важен, «ё» = «е»).
export function isObscene(word: string): boolean {
  const w = word.toLowerCase().replace(/ё/g, "е");
  if (!w) return false;
  if (CONTAINS.some((r) => w.includes(r))) return true;
  if (EXACT.has(w)) return true;
  if (STARTS.some((r) => w.startsWith(r))) return true;
  for (const p of HUI_PREFIXES) {
    if (!w.startsWith(p)) continue;
    const rest = w.slice(p.length);
    if (HUI_ROOTS.some((r) => rest.startsWith(r))) {
      if (rest.startsWith("хул") && !["хуля", "хули", "хуле"].includes(rest)) continue;
      return true;
    }
  }
  for (const p of EB_PREFIXES) {
    if (!w.startsWith(p)) continue;
    const rest = w.slice(p.length);
    if (rest.startsWith("еб") && !rest.startsWith("ебонит")) return true;
  }
  return false;
}

/// Заменяет нецензурные слова пометкой; остальной текст — как есть.
export function censor(text: string): string {
  return text.replace(/\p{L}+/gu, (w) => (isObscene(w) ? MARK : w));
}

/// Расшифровка для показа/экспорта по политике (исходник не меняется).
export function applyPolicy(t: Transcript, policy: ProfanityPolicy): Transcript {
  if (policy !== "censor") return t;
  return { ...t, segments: t.segments.map((s) => ({ ...s, text: censor(s.text) })) };
}
