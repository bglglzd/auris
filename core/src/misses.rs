//! Почему реплика не распозналась: разбор звука места, которое пользователь
//! дописал сам. Причина показывается под репликой и пишется в лог и в
//! `<встреча>/corrections.jsonl` — по ним видно, что чинить в распознавании
//! (тишина в дорожке, перебивания, тихий голос, шум или «чистый звук, но
//! незнакомые слова»).

use serde::Serialize;

const SR: f64 = 16_000.0;
const FRAME: usize = 512;

/// Уровень одной дорожки в месте пропуска.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackStat {
    pub track: String,
    /// Громкость речи (90-й перцентиль RMS кадров), дБFS.
    pub level_db: f32,
    pub snr_db: Option<f32>,
    /// Секунды, где дорожка звучит (кадры громче −38 дБFS).
    pub active_secs: f32,
}

/// Итог разбора.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnosis {
    /// short | silent | overlap | quiet | noisy | clear
    pub code: &'static str,
    /// Причина по-русски — для интерфейса.
    pub text: String,
    pub tracks: Vec<TrackStat>,
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-6).log10()
}

fn stat(name: &str, clip: &[f32]) -> TrackStat {
    let mut rms: Vec<f32> = clip
        .chunks(FRAME)
        .filter(|c| c.len() == FRAME)
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / FRAME as f32).sqrt())
        .collect();
    let active = rms.iter().filter(|&&r| r >= 0.0126).count() as f32 * FRAME as f32 / SR as f32;
    rms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let level = rms.get((rms.len().max(1) - 1) * 9 / 10).copied().unwrap_or(0.0);
    TrackStat { track: name.to_string(), level_db: db(level), snr_db: crate::enhance::snr_db(clip), active_secs: active }
}

/// Разбирает отрезок `start..end` (секунды) дорожек встречи (16 кГц моно).
pub fn diagnose(tracks: &[(&str, &[f32])], start: f64, end: f64) -> Diagnosis {
    let (a, b) = ((start.max(0.0) * SR) as usize, (end.max(start) * SR) as usize);
    let stats: Vec<TrackStat> = tracks
        .iter()
        .map(|(name, s)| stat(name, s.get(a.min(s.len())..b.min(s.len())).unwrap_or(&[])))
        .collect();
    let best = stats.iter().max_by(|x, y| x.level_db.partial_cmp(&y.level_db).unwrap_or(std::cmp::Ordering::Equal));
    let (code, text) = match best {
        _ if end - start < 0.8 => ("short", "короткая реплика (меньше секунды) — такие модель часто пропускает".to_string()),
        None => ("silent", "звука встречи нет".to_string()),
        Some(t) if t.level_db < -45.0 => (
            "silent",
            format!("в записи здесь почти тишина ({:.0} дБ) — звук, похоже, не попал в дорожку", t.level_db),
        ),
        _ if stats.iter().filter(|t| t.active_secs >= 0.3).count() >= 2 => {
            ("overlap", "говорили одновременно — речь в обеих дорожках, модель уловила только одну".to_string())
        }
        Some(t) if t.level_db < -32.0 => {
            ("quiet", format!("тихо ({:.0} дБ) — голос далеко от микрофона", t.level_db))
        }
        Some(t) if t.snr_db.map(|s| s < crate::rescue::MIN_SNR_DB).unwrap_or(false) => {
            ("noisy", format!("шумно (сигнал/шум {:.0} дБ) — речь тонет в фоне", t.snr_db.unwrap_or(0.0)))
        }
        Some(_) => (
            "clear",
            "звук чистый — модель не узнала слова (термины, имена, быстрая речь); ваш текст запомнен".to_string(),
        ),
    };
    Diagnosis { code, text, tracks: stats }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «Речь»: тон слогами (0.2 с звучит, 0.1 с пауза).
    fn tone(secs: f64, amp: f32) -> Vec<f32> {
        (0..(secs * SR) as usize)
            .map(|i| if (i as f64 / SR) % 0.3 < 0.2 { amp * (i as f32 * 0.07).sin() } else { 0.0 })
            .collect()
    }

    #[test]
    fn explains_the_miss() {
        let loud = tone(4.0, 0.3);
        let silent = vec![0.0f32; 64_000];
        let quiet = tone(4.0, 0.02);
        assert_eq!(diagnose(&[("mic", &silent)], 1.0, 3.0).code, "silent");
        assert_eq!(diagnose(&[("mic", &loud)], 1.0, 1.5).code, "short");
        assert_eq!(diagnose(&[("mic", &loud), ("system", &loud)], 1.0, 3.0).code, "overlap");
        assert_eq!(diagnose(&[("mic", &quiet)], 1.0, 3.0).code, "quiet");
        assert_eq!(diagnose(&[("mic", &loud), ("system", &silent)], 1.0, 3.0).code, "clear");
        let noisy: Vec<f32> = loud.iter().enumerate().map(|(i, v)| v + 0.12 * ((i * 7919 % 101) as f32 / 50.0 - 1.0)).collect();
        assert_eq!(diagnose(&[("mic", &noisy)], 1.0, 3.0).code, "noisy");
        // Отрезок за концом записи — не паника.
        assert_eq!(diagnose(&[("mic", &loud)], 10.0, 12.0).code, "silent");
    }
}
