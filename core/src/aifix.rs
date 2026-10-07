//! ИИ-корректура расшифровки (через сервер ИИ пользователя, по желанию).
//!
//! Локальное распознавание ошибается в словах («над дубитом» вместо «на дубе
//! том»), терминах и пунктуации. Чат-модель видит текст целиком и исправляет
//! такие ошибки — но только их: каждая реплика возвращается на своё место,
//! время и говорящий не меняются. Защита от «творчества» модели: правка
//! принимается, только если она близка к исходной реплике (не пересказ), не
//! меняет язык и не пустая. Реплики, которые пользователь дописал или
//! исправил сам, не трогаются. На сервер уходит только текст, звук — нет.

use crate::ai::ChatBackend;
use crate::error::AppResult;
use crate::transcript::Transcript;

/// Реплик в одном запросе (и не больше ~4000 символов).
const BATCH: usize = 40;
const BATCH_CHARS: usize = 4000;
/// Доля изменённых символов, выше которой правка считается пересказом.
const MAX_CHANGE: f32 = 0.45;

const SYSTEM: &str = "Ты — корректор автоматической расшифровки речи. Тебе дают JSON-массив реплик \
разговора по порядку. Исправь ТОЛЬКО ошибки распознавания: неверно услышанные или разорванные слова \
(по смыслу соседних реплик), термины и имена (см. словарь, если он дан), пунктуацию и заглавные буквы. \
НЕЛЬЗЯ: пересказывать, сокращать, дополнять, убирать слова-паразиты и повторы, переводить на другой язык, \
объединять или делить реплики. Если реплика верна — верни её без изменений. Ответ — строго JSON-массив \
строк той же длины и в том же порядке, без пояснений.";

/// Итог корректуры: новая расшифровка и сколько реплик исправлено.
#[derive(Debug, Clone, PartialEq)]
pub struct Corrected {
    pub transcript: Transcript,
    pub changed: usize,
    /// Правок, отклонённых как пересказ / смена языка.
    pub rejected: usize,
}

/// Расстояние Левенштейна по символам (для оценки размера правки).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Принимаем ли правку модели: не пустая, тот же язык, не пересказ.
pub fn acceptable(original: &str, fixed: &str) -> bool {
    let (o, f) = (original.trim(), fixed.trim());
    if f.is_empty() {
        return false;
    }
    if o == f {
        return true;
    }
    if let (Some(a), Some(b)) = (crate::langguard::dominant_script(o), crate::langguard::dominant_script(f)) {
        if a != b {
            return false;
        }
    }
    let norm = |s: &str| s.to_lowercase();
    let d = edit_distance(&norm(o), &norm(f)) as f32;
    let len = o.chars().count().max(f.chars().count()).max(1) as f32;
    d / len <= MAX_CHANGE
}

/// Достаёт JSON-массив строк из ответа модели (с блоком кода или без).
pub fn parse_lines(answer: &str) -> Option<Vec<String>> {
    let a = answer.find('[')?;
    let b = answer.rfind(']')?;
    if b <= a {
        return None;
    }
    serde_json::from_str::<Vec<String>>(&answer[a..=b]).ok()
}

/// Корректура расшифровки. `glossary` — термины словаря (подсказка модели).
/// `on_progress(done, total)` — после каждого запроса.
pub fn correct(
    backend: &dyn ChatBackend,
    t: &Transcript,
    glossary: Option<&str>,
    on_progress: &dyn Fn(usize, usize),
) -> AppResult<Corrected> {
    // Пачки подряд идущих реплик (контекст для модели).
    let mut batches: Vec<std::ops::Range<usize>> = Vec::new();
    let mut start = 0;
    let mut chars = 0;
    for (i, s) in t.segments.iter().enumerate() {
        chars += s.text.chars().count();
        if i + 1 - start >= BATCH || chars >= BATCH_CHARS {
            batches.push(start..i + 1);
            start = i + 1;
            chars = 0;
        }
    }
    if start < t.segments.len() {
        batches.push(start..t.segments.len());
    }
    let mut out = t.clone();
    let (mut changed, mut rejected) = (0, 0);
    let total = batches.len();
    for (k, range) in batches.into_iter().enumerate() {
        let lines: Vec<&str> = t.segments[range.clone()].iter().map(|s| s.text.as_str()).collect();
        let mut user = String::new();
        if let Some(g) = glossary.map(str::trim).filter(|g| !g.is_empty()) {
            user.push_str("Словарь (так пишутся термины и имена): ");
            user.push_str(&g.replace('\n', ", "));
            user.push_str("\n\n");
        }
        user.push_str(&serde_json::to_string(&lines).unwrap_or_default());
        let answer = backend.chat(SYSTEM, &user)?;
        // Ответ не той длины — пачку не трогаем (иначе реплики съедут).
        if let Some(fixed) = parse_lines(&answer).filter(|f| f.len() == lines.len()) {
            for (i, f) in range.clone().zip(fixed) {
                let seg = &mut out.segments[i];
                if seg.is_user() || seg.text.trim() == f.trim() {
                    continue;
                }
                if acceptable(&seg.text, &f) {
                    seg.text = f.trim().to_string();
                    changed += 1;
                } else {
                    rejected += 1;
                }
            }
        }
        on_progress(k + 1, total);
    }
    Ok(Corrected { transcript: out, changed, rejected })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::TranscriptSegment;

    struct Fake(Vec<String>);
    impl ChatBackend for Fake {
        fn chat(&self, _s: &str, _u: &str) -> AppResult<String> {
            Ok(format!("```json\n{}\n```", serde_json::to_string(&self.0).unwrap()))
        }
    }

    fn seg(t: &str, origin: Option<&str>) -> TranscriptSegment {
        TranscriptSegment { speaker: "me".into(), start_secs: 0.0, end_secs: 1.0, text: t.into(), origin: origin.map(Into::into) }
    }

    #[test]
    fn fixes_misheard_words_but_not_rewrites() {
        let t = Transcript {
            segments: vec![
                seg("Золотая цепь над дубитом", None),
                seg("заведём задачу в жиру", None),
                seg("ну короче я в общем думаю что надо", None),
                seg("Это я поправил сам", Some("edited")),
                seg("Давайте начнём", None),
            ],
        };
        let fake = Fake(vec![
            "Золотая цепь на дубе том".into(),
            "Заведём задачу в Jira.".into(),
            // Пересказ — отклоняется.
            "Предлагаю действовать".into(),
            "Совсем другое".into(),
            // Перевод — отклоняется.
            "Let's start".into(),
        ]);
        let r = correct(&fake, &t, Some("Jira"), &|_, _| {}).unwrap();
        let texts: Vec<&str> = r.transcript.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            texts,
            ["Золотая цепь на дубе том", "Заведём задачу в Jira.", "ну короче я в общем думаю что надо", "Это я поправил сам", "Давайте начнём"]
        );
        assert_eq!((r.changed, r.rejected), (2, 2));
    }

    #[test]
    fn wrong_length_answer_changes_nothing() {
        let t = Transcript { segments: vec![seg("раз", None), seg("два", None)] };
        let r = correct(&Fake(vec!["один".into()]), &t, None, &|_, _| {}).unwrap();
        assert_eq!(r.transcript, t);
        assert_eq!(parse_lines("нет массива"), None);
    }

    #[test]
    fn batches_long_transcripts() {
        let t = Transcript { segments: (0..95).map(|i| seg(&format!("реплика {i}"), None)).collect() };
        let calls = std::sync::Mutex::new(0);
        struct Echo<'a>(&'a std::sync::Mutex<usize>);
        impl ChatBackend for Echo<'_> {
            fn chat(&self, _s: &str, u: &str) -> AppResult<String> {
                *self.0.lock().unwrap() += 1;
                Ok(u.to_string())
            }
        }
        let r = correct(&Echo(&calls), &t, None, &|_, _| {}).unwrap();
        assert_eq!(*calls.lock().unwrap(), 3);
        assert_eq!(r.changed, 0);
    }
}
