/// Обучение на правках пользователя: из исправленной реплики выделяются
/// замены «как распознано => как правильно» и новые термины. Они попадают в
/// «Выучено из ваших правок» (настройки → Распознавание) и дальше работают
/// как словарь: подсказка Whisper + исправление написания при следующих
/// расшифровках (`core/src/vocab.rs`, правила `=>`).
///
/// Выучиваются только «термины» — латиница, цифры, имена с заглавной в
/// середине фразы. Обычные слова («над дубом» → «на дубе») — правка
/// конкретного места, а не правило: глобальная замена испортила бы другие
/// встречи.

const LATIN = /[A-Za-z]/;
const DIGIT = /\d/;
const MAX_LEARNED = 300;

interface Word {
  raw: string;
  norm: string;
  /// Начало предложения (первое слово или после . ! ?).
  sentenceStart: boolean;
}

function words(text: string): Word[] {
  const out: Word[] = [];
  let start = true;
  for (const raw of text.split(/\s+/).filter(Boolean)) {
    const norm = raw.replace(/[^\p{L}\p{N}]/gu, "").toLowerCase();
    if (norm) out.push({ raw: raw.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, ""), norm, sentenceStart: start });
    start = /[.!?…]["»)]*$/.test(raw);
  }
  return out;
}

/// Слово похоже на термин: латиница, цифры, заглавная внутри слова или
/// заглавная не в начале предложения.
function termLike(w: Word): boolean {
  if (LATIN.test(w.raw) || DIGIT.test(w.raw)) return true;
  if (/\p{Ll}\p{Lu}/u.test(w.raw)) return true;
  return !w.sentenceStart && /^\p{Lu}/u.test(w.raw);
}

/// Пары заменённых участков (LCS по нормализованным словам).
function replacedChunks(a: Word[], b: Word[]): Array<[Word[], Word[]]> {
  const n = a.length;
  const m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i][j] = a[i].norm === b[j].norm ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const out: Array<[Word[], Word[]]> = [];
  let i = 0;
  let j = 0;
  let ca: Word[] = [];
  let cb: Word[] = [];
  const flush = () => {
    if (ca.length && cb.length) out.push([ca, cb]);
    ca = [];
    cb = [];
  };
  while (i < n || j < m) {
    if (i < n && j < m && a[i].norm === b[j].norm) {
      flush();
      // То же слово, другое написание (регистр, дефис): «петрова» → «Петрова».
      if (a[i].raw !== b[j].raw) out.push([[a[i]], [b[j]]]);
      i++;
      j++;
    } else if (j < m && (i >= n || dp[i][j + 1] >= dp[i + 1][j])) {
      cb.push(b[j++]);
    } else {
      ca.push(a[i++]);
    }
  }
  flush();
  return out;
}

/// Что выучить из правки реплики: строки «было => стало» и термины.
/// `before` пустой — реплика дописана вручную (выучиваются только термины).
export function learnFromEdit(before: string, after: string): string[] {
  const a = words(before);
  const b = words(after);
  const out: string[] = [];
  const add = (line: string) => {
    if (!out.some((o) => o.toLowerCase() === line.toLowerCase())) out.push(line);
  };
  if (a.length === 0) {
    b.filter(termLike).forEach((w) => w.norm.length >= 2 && add(w.raw));
    return out;
  }
  for (const [from, to] of replacedChunks(a, b)) {
    if (from.length > 3 || to.length > 3 || !to.some(termLike)) continue;
    const left = from.map((w) => w.norm).join(" ");
    if (left.replace(/\s/g, "").length < 3) continue;
    add(`${left} => ${to.map((w) => w.raw).join(" ")}`);
  }
  return out;
}

/// Добавляет выученное к списку (без повторов, свежие — в конце; не больше
/// `MAX_LEARNED` строк — старые уходят). Новое правило для того же «было»
/// заменяет прежнее.
export function mergeLearned(current: string, lines: string[]): string {
  const key = (l: string) => (l.includes("=>") ? l.split("=>")[0] : l).trim().toLowerCase();
  let list = current
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  for (const line of lines) {
    list = list.filter((l) => key(l) !== key(line));
    list.push(line);
  }
  return list.slice(-MAX_LEARNED).join("\n");
}

/// Подпись выученного для уведомления: «жира → Jira, Kubernetes».
export function learnedLabel(lines: string[]): string {
  return lines.map((l) => l.replace(/\s*=>\s*/, " → ")).join(", ");
}
