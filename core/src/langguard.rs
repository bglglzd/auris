//! Контроль языка расшифровки. Parakeet определяет язык сам и на неразборчивом
//! участке может «услышать» другой язык (латиница вместо русского). Здесь —
//! чистая логика: найти реплики не той письменности и заменить их
//! перераспознанными (движок с заданным языком — в слое приложения).

use std::path::Path;

use crate::error::{AppError, AppResult};
use crate::transcript::Segment;

/// Письменность языка.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Cyrillic,
    Latin,
}

/// Языки на кириллице (остальные распознаваемые — на латинице).
const CYRILLIC_LANGS: &[&str] = &["ru", "uk", "be", "bg", "sr", "mk", "kk", "ky", "tg", "mn"];

/// Письменность для кода языка из настроек; `auto`/пусто → `None`.
pub fn script_for_language(lang: Option<&str>) -> Option<Script> {
    let l = lang?.trim().to_lowercase();
    if l.is_empty() || l == "auto" {
        return None;
    }
    Some(if CYRILLIC_LANGS.contains(&l.as_str()) { Script::Cyrillic } else { Script::Latin })
}

/// Число букв кириллицы и латиницы в тексте.
pub fn letters(text: &str) -> (usize, usize) {
    let mut cyr = 0;
    let mut lat = 0;
    for c in text.chars() {
        if ('\u{0400}'..='\u{04FF}').contains(&c) {
            cyr += 1;
        } else if c.is_ascii_alphabetic() || ('\u{00C0}'..='\u{024F}').contains(&c) {
            lat += 1;
        }
    }
    (cyr, lat)
}

/// Письменность, которой должна быть расшифровка: язык из настроек, а при
/// автоопределении — преобладающая (≥ 70 % букв) письменность всей дорожки.
/// Смешанная речь при автоопределении не трогается.
pub fn target_script(language: Option<&str>, segs: &[Segment]) -> Option<Script> {
    if let Some(s) = script_for_language(language) {
        return Some(s);
    }
    let (cyr, lat) = segs.iter().fold((0, 0), |(c, l), s| {
        let (a, b) = letters(&s.text);
        (c + a, l + b)
    });
    let total = cyr + lat;
    if total < 40 {
        return None;
    }
    if cyr * 10 >= total * 7 {
        Some(Script::Cyrillic)
    } else if lat * 10 >= total * 7 {
        Some(Script::Latin)
    } else {
        None
    }
}

/// Реплика не той письменности: в ней ≥ 4 букв и меньше половины — нужной.
/// Короткие вставки (имена, термины латиницей) не считаются.
pub fn is_mismatch(text: &str, target: Script) -> bool {
    let (cyr, lat) = letters(text);
    let (good, bad) = match target {
        Script::Cyrillic => (cyr, lat),
        Script::Latin => (lat, cyr),
    };
    bad >= 4 && good * 2 < good + bad
}

/// Участки (секунды) с репликами не той письменности: соседние (зазор < 1 с)
/// склеиваются, края расширяются на `pad` в пределах записи.
pub fn mismatch_spans(segs: &[Segment], target: Script, pad: f64, duration: f64) -> Vec<(f64, f64)> {
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for s in segs.iter().filter(|s| is_mismatch(&s.text, target)) {
        let a = (s.start_secs - pad).max(0.0);
        let b = (s.end_secs + pad).min(duration.max(s.end_secs));
        match spans.last_mut() {
            Some(last) if a - last.1 < 1.0 => last.1 = last.1.max(b),
            _ => spans.push((a, b)),
        }
    }
    spans
}

/// Заменяет реплики, попавшие (серединой) в участки `spans`, новыми
/// репликами `fixed` (уже со временем записи); результат — по времени.
pub fn replace_spans(segs: Vec<Segment>, spans: &[(f64, f64)], fixed: Vec<Segment>) -> Vec<Segment> {
    let inside = |s: &Segment| {
        let mid = (s.start_secs + s.end_secs) / 2.0;
        spans.iter().any(|&(a, b)| mid >= a && mid <= b)
    };
    let mut out: Vec<Segment> = segs.into_iter().filter(|s| !inside(s)).collect();
    out.extend(fixed.into_iter().filter(|s| !s.text.trim().is_empty()));
    out.sort_by(|a, b| a.start_secs.partial_cmp(&b.start_secs).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Длительность WAV в секундах.
pub fn wav_duration(path: &Path) -> AppResult<f64> {
    let r = hound::WavReader::open(path).map_err(|e| AppError::Audio(e.to_string()))?;
    let spec = r.spec();
    Ok(r.duration() as f64 / spec.sample_rate.max(1) as f64)
}

/// Вырезает `[from, to)` секунд из WAV (16 кГц моно i16) в `dst`.
pub fn cut_wav(src: &Path, dst: &Path, from: f64, to: f64) -> AppResult<()> {
    let mut r = hound::WavReader::open(src).map_err(|e| AppError::Audio(e.to_string()))?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as u64;
    let a = (from.max(0.0) * spec.sample_rate as f64) as u64 * ch;
    let b = (to.max(from) * spec.sample_rate as f64) as u64 * ch;
    r.seek((a / ch) as u32).map_err(|e| AppError::Audio(e.to_string()))?;
    let mut w = hound::WavWriter::create(dst, spec).map_err(|e| AppError::Audio(e.to_string()))?;
    for s in r.samples::<i16>().take((b - a) as usize) {
        w.write_sample(s.map_err(|e| AppError::Audio(e.to_string()))?)
            .map_err(|e| AppError::Audio(e.to_string()))?;
    }
    w.finalize().map_err(|e| AppError::Audio(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: f64, b: f64, t: &str) -> Segment {
        Segment { start_secs: a, end_secs: b, text: t.into() }
    }

    #[test]
    fn language_to_script() {
        assert_eq!(script_for_language(Some("ru")), Some(Script::Cyrillic));
        assert_eq!(script_for_language(Some(" UK ")), Some(Script::Cyrillic));
        assert_eq!(script_for_language(Some("en")), Some(Script::Latin));
        assert_eq!(script_for_language(Some("auto")), None);
        assert_eq!(script_for_language(None), None);
    }

    #[test]
    fn mismatch_ignores_short_terms() {
        assert!(is_mismatch("So we can do that tomorrow", Script::Cyrillic));
        assert!(!is_mismatch("Открой Zoom и созвонимся", Script::Cyrillic));
        assert!(!is_mismatch("Ок", Script::Cyrillic));
        assert!(is_mismatch("Привет, как дела", Script::Latin));
    }

    #[test]
    fn auto_uses_dominant_script_only() {
        let ru = vec![
            seg(0.0, 3.0, "Давайте обсудим план на следующую неделю"),
            seg(3.0, 6.0, "Да, я подготовил презентацию и расчёты"),
            seg(6.0, 8.0, "I think so"),
        ];
        assert_eq!(target_script(Some("auto"), &ru), Some(Script::Cyrillic));
        let mixed = vec![
            seg(0.0, 3.0, "Давайте обсудим план на неделю"),
            seg(3.0, 6.0, "Sure, let us talk about the plan"),
        ];
        assert_eq!(target_script(None, &mixed), None);
        // Явный язык важнее содержимого.
        assert_eq!(target_script(Some("ru"), &mixed), Some(Script::Cyrillic));
    }

    #[test]
    fn spans_merge_and_replace() {
        let segs = vec![
            seg(0.0, 2.0, "Добрый день, начинаем"),
            seg(2.5, 4.0, "Then we go to the"),
            seg(4.2, 6.0, "next slide please now"),
            seg(9.0, 11.0, "Отлично, спасибо всем"),
        ];
        let spans = mismatch_spans(&segs, Script::Cyrillic, 0.3, 11.0);
        assert_eq!(spans.len(), 1);
        assert!((spans[0].0 - 2.2).abs() < 1e-9 && (spans[0].1 - 6.3).abs() < 1e-9);
        let fixed = vec![seg(2.4, 6.0, "Потом переходим к следующему слайду")];
        let out = replace_spans(segs, &spans, fixed);
        let texts: Vec<&str> = out.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Добрый день, начинаем", "Потом переходим к следующему слайду", "Отлично, спасибо всем"]);
    }

    #[test]
    fn cut_wav_takes_the_range() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&src, spec).unwrap();
        for i in 0..32_000 {
            w.write_sample((i / 16) as i16).unwrap();
        }
        w.finalize().unwrap();
        assert!((wav_duration(&src).unwrap() - 2.0).abs() < 1e-9);
        let dst = dir.path().join("b.wav");
        cut_wav(&src, &dst, 0.5, 1.0).unwrap();
        let v: Vec<i16> = hound::WavReader::open(&dst).unwrap().samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(v.len(), 8_000);
        assert_eq!(v[0], 500);
    }
}
