//! Второй проход по трудным участкам (шум, выкрики, перебивания) — решает,
//! какие окна перераспознавать и какой вариант оставить. Чистая логика;
//! движки и шумоподавление вызывает слой приложения.
//!
//! Что показали замеры (`--example noise_eval`): в умеренном шуме Parakeet
//! путает слова — шумоподавление снижает ошибки (WER 0.24 → 0.14); когда
//! чужие голоса громкие, Parakeet не ошибается, а МОЛЧИТ (пустое окно при
//! явной речи) — уверенность модели этого не видит. Поэтому трудное окно —
//! это и низкая уверенность, и «речь есть, а слов почти нет».

/// Окно первого прохода: что слышно и что распознано.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowStats {
    pub start_secs: f64,
    pub end_secs: f64,
    /// Отношение сигнал/шум, дБ (`None` — речи нет).
    pub snr_db: Option<f32>,
    /// Сколько секунд в окне звучит речь (кадры заметно громче фона).
    pub speech_secs: f32,
    /// Уверенность модели (средняя по токенам), `None` — токенов нет.
    pub confidence: Option<f32>,
    pub tokens: usize,
}

/// Секунды «звучащего» в отрывке: кадры 32 мс громче фона на 6 дБ (и не тише
/// −40 дБFS).
pub fn speech_secs(samples: &[f32]) -> f32 {
    const FRAME: usize = 512;
    let mut rms: Vec<f32> = samples
        .chunks(FRAME)
        .filter(|c| c.len() == FRAME)
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / FRAME as f32).sqrt())
        .collect();
    if rms.is_empty() {
        return 0.0;
    }
    let frames = rms.clone();
    rms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = rms[(rms.len() - 1) / 10].max(1e-5);
    let gate = (floor * 2.0).max(0.01);
    frames.iter().filter(|&&r| r > gate).count() as f32 * FRAME as f32 / 16_000.0
}

/// Минимум токенов на секунду речи: обычная речь — 3–5 токенов/с; меньше
/// 1.2 — модель пропустила сказанное.
pub const MIN_TOKENS_PER_SPEECH_SEC: f32 = 1.2;
/// Ниже этой уверенности окно перепроверяется.
pub const MIN_CONFIDENCE: f32 = 0.85;
/// Шумнее этого (дБ) — перепроверяется, даже если модель уверена.
pub const MIN_SNR_DB: f32 = 12.0;

/// Нужен ли окну второй проход.
pub fn is_hard(w: &WindowStats) -> bool {
    if w.snr_db.is_none() || w.speech_secs < 1.0 {
        return false; // тишина/пара звуков — перепроверять нечего
    }
    let rate = w.tokens as f32 / w.speech_secs;
    rate < MIN_TOKENS_PER_SPEECH_SEC
        || w.confidence.map(|c| c < MIN_CONFIDENCE).unwrap_or(true)
        || w.snr_db.map(|s| s < MIN_SNR_DB).unwrap_or(false)
}

/// Вариант распознавания окна.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub words: usize,
    pub confidence: Option<f32>,
}

/// Лучше ли вариант того же движка (Parakeet на очищенном звуке) исходного:
/// больше сказанного при той же уверенности или заметно увереннее почти без
/// потерь слов.
pub fn better_same_engine(orig: Candidate, alt: Candidate) -> bool {
    let (oc, ac) = (orig.confidence.unwrap_or(0.0), alt.confidence.unwrap_or(0.0));
    if alt.words == 0 {
        return false;
    }
    (alt.words > orig.words && ac >= oc - 0.05)
        || (ac >= oc + 0.03 && alt.words * 10 >= orig.words * 8)
}

/// Осталось ли окно трудным после лучшего варианта Parakeet.
pub fn still_hard(best: Candidate, speech_secs: f32) -> bool {
    let words_rate = best.words as f32 / speech_secs.max(0.1);
    // ~0.6 слова/с ≈ 1.2 токена/с.
    words_rate < 0.6 || best.confidence.map(|c| c < 0.8).unwrap_or(true)
}

/// Берём ли вариант Whisper: уверенный (средняя вероятность токенов ≥ 0.55)
/// и содержательнее лучшего варианта Parakeet.
pub fn accept_whisper(best: Candidate, whisper: Candidate) -> bool {
    whisper.words > best.words && whisper.confidence.map(|p| p >= 0.55).unwrap_or(false)
}

/// Число слов в тексте реплик.
pub fn word_count<'a>(texts: impl IntoIterator<Item = &'a str>) -> usize {
    texts.into_iter().map(|t| t.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(snr: Option<f32>, speech: f32, conf: Option<f32>, tokens: usize) -> WindowStats {
        WindowStats { start_secs: 0.0, end_secs: 10.0, snr_db: snr, speech_secs: speech, confidence: conf, tokens }
    }

    #[test]
    fn finds_dropped_and_unsure_windows() {
        // Чистая уверенная речь — не трогаем.
        assert!(!is_hard(&w(Some(25.0), 8.0, Some(0.97), 32)));
        // Речь есть, а слов нет — Parakeet «сдался» (перебивания).
        assert!(is_hard(&w(Some(8.0), 8.0, None, 0)));
        assert!(is_hard(&w(Some(20.0), 8.0, Some(0.95), 4)));
        // Неуверенно или шумно.
        assert!(is_hard(&w(Some(20.0), 8.0, Some(0.7), 30)));
        assert!(is_hard(&w(Some(9.0), 8.0, Some(0.95), 30)));
        // Тишина — нечего перепроверять.
        assert!(!is_hard(&w(None, 0.0, None, 0)));
        assert!(!is_hard(&w(Some(10.0), 0.4, None, 0)));
    }

    #[test]
    fn picks_denoised_only_when_better() {
        let o = Candidate { words: 18, confidence: Some(0.947) };
        // Замер noise_eval (fr, 5 дБ): после шумоподавления слов больше
        // (WER 0.24 → 0.14), уверенность чуть ниже (0.947 → 0.912) — берём.
        assert!(better_same_engine(o, Candidate { words: 19, confidence: Some(0.912) }));
        assert!(!better_same_engine(o, Candidate { words: 19, confidence: Some(0.85) }));
        assert!(!better_same_engine(o, Candidate { words: 12, confidence: Some(0.99) }));
        assert!(!better_same_engine(o, Candidate { words: 0, confidence: None }));
        let empty = Candidate { words: 0, confidence: None };
        assert!(better_same_engine(empty, Candidate { words: 5, confidence: Some(0.8) }));
    }

    #[test]
    fn whisper_rescues_dropped_speech_only_when_confident() {
        let empty = Candidate { words: 0, confidence: None };
        assert!(still_hard(empty, 8.0));
        assert!(!still_hard(Candidate { words: 20, confidence: Some(0.95) }, 8.0));
        assert!(accept_whisper(empty, Candidate { words: 14, confidence: Some(0.71) }));
        assert!(!accept_whisper(empty, Candidate { words: 14, confidence: Some(0.3) }));
        assert!(!accept_whisper(Candidate { words: 15, confidence: Some(0.7) }, Candidate { words: 12, confidence: Some(0.9) }));
    }

    #[test]
    fn speech_seconds_and_words() {
        let mut x = vec![0.001f32; 16_000 * 4];
        for (i, v) in x.iter_mut().enumerate().take(16_000 * 2) {
            *v = 0.2 * (i as f32 * 0.1).sin();
        }
        let s = speech_secs(&x);
        assert!((s - 2.0).abs() < 0.1, "{s}");
        assert_eq!(word_count(["Привет, мир!", " — ну да"]), 4);
    }
}
