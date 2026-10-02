//! Фоновое уточнение расшифровки. Первый проход (Parakeet) сразу даёт готовую
//! расшифровку; трудные места (шум, перебивания, «не тот язык») сохраняются
//! планом в `<встреча>/refine.json` и уточняются потом, в фоне, не задерживая
//! пользователя. Каждое уточнённое место сразу попадает в расшифровку.
//!
//! Реплики, которые пользователь успел поправить, не трогаются: заменяются
//! только реплики, совпадающие по времени с исходными репликами окна.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::transcript::{Segment, Transcript, TranscriptSegment};

const FILE: &str = "refine.json";

/// Почему окно уточняется.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RefineKind {
    /// Шум / неуверенность / пропущенная речь.
    Hard,
    /// Реплики не той письменности (латиница вместо русского и т.п.).
    Lang,
}

/// Окно для уточнения и исходные реплики первого прохода в нём.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefineWindow {
    pub start_secs: f64,
    pub end_secs: f64,
    pub kind: RefineKind,
    /// Чем больше, тем хуже (порядок обработки).
    pub severity: f32,
    /// Отношение сигнал/шум и доля речи — для решения о Whisper.
    #[serde(default)]
    pub speech_secs: f32,
    #[serde(default)]
    pub confidence: Option<f32>,
    /// Реплики первого прохода в окне (по ним находим, что заменить).
    pub originals: Vec<Segment>,
}

/// Окна одной дорожки (16 кГц моно WAV в папке встречи).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefineJob {
    pub wav: String,
    pub windows: Vec<RefineWindow>,
}

/// План уточнения встречи.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RefinePlan {
    pub jobs: Vec<RefineJob>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub vocabulary: Option<String>,
}

impl RefinePlan {
    pub fn total(&self) -> usize {
        self.jobs.iter().map(|j| j.windows.len()).sum()
    }
}

pub fn save(meeting_dir: &Path, plan: &RefinePlan) -> AppResult<()> {
    let path = meeting_dir.join(FILE);
    if plan.total() == 0 {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    std::fs::write(path, serde_json::to_vec(plan)?)?;
    Ok(())
}

pub fn load(meeting_dir: &Path) -> AppResult<Option<RefinePlan>> {
    let path = meeting_dir.join(FILE);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    Ok(Some(serde_json::from_slice(&bytes).map_err(|e| AppError::InvalidState(format!("refine.json: {e}")))?))
}

pub fn clear(meeting_dir: &Path) {
    let _ = std::fs::remove_file(meeting_dir.join(FILE));
}

fn same_time(a: &TranscriptSegment, b: &Segment) -> bool {
    (a.start_secs - b.start_secs).abs() < 1e-3 && (a.end_secs - b.end_secs).abs() < 1e-3
}

/// Заменяет в расшифровке реплики окна (совпадающие по времени с исходными)
/// новыми. Говорящий новой реплики — у исходной с наибольшим перекрытием
/// (ближайшей, если перекрытия нет). `None` — заменять нечего (реплики
/// окна правили или удалили).
pub fn apply_window(t: &Transcript, originals: &[Segment], fresh: &[Segment]) -> Option<Transcript> {
    let matched: Vec<&TranscriptSegment> =
        t.segments.iter().filter(|s| originals.iter().any(|o| same_time(s, o))).collect();
    if matched.is_empty() || matched.len() < originals.len() {
        return None;
    }
    let speaker_for = |a: f64, b: f64| -> String {
        let overlap = |s: &TranscriptSegment| (b.min(s.end_secs) - a.max(s.start_secs)).max(0.0);
        let dist = |s: &TranscriptSegment| {
            let mid = (a + b) / 2.0;
            ((s.start_secs + s.end_secs) / 2.0 - mid).abs()
        };
        matched
            .iter()
            .max_by(|x, y| {
                overlap(x)
                    .partial_cmp(&overlap(y))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(dist(y).partial_cmp(&dist(x)).unwrap_or(std::cmp::Ordering::Equal))
            })
            .map(|s| s.speaker.clone())
            .unwrap_or_default()
    };
    let mut segments: Vec<TranscriptSegment> =
        t.segments.iter().filter(|s| !originals.iter().any(|o| same_time(s, o))).cloned().collect();
    for f in fresh.iter().filter(|f| !f.text.trim().is_empty()) {
        segments.push(TranscriptSegment {
            speaker: speaker_for(f.start_secs, f.end_secs),
            start_secs: f.start_secs,
            end_secs: f.end_secs,
            text: f.text.clone(),
        });
    }
    segments.sort_by(|a, b| a.start_secs.partial_cmp(&b.start_secs).unwrap_or(std::cmp::Ordering::Equal));
    Some(Transcript { segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: f64, b: f64, t: &str) -> Segment {
        Segment { start_secs: a, end_secs: b, text: t.into() }
    }
    fn tseg(sp: &str, a: f64, b: f64, t: &str) -> TranscriptSegment {
        TranscriptSegment { speaker: sp.into(), start_secs: a, end_secs: b, text: t.into() }
    }

    #[test]
    fn replaces_window_and_keeps_speakers() {
        let t = Transcript {
            segments: vec![
                tseg("me", 0.0, 2.0, "Добрый день"),
                tseg("spk0", 3.0, 5.0, "Then we go"),
                tseg("spk1", 5.5, 8.0, "to the"),
                tseg("me", 9.0, 10.0, "Спасибо"),
            ],
        };
        let originals = vec![seg(3.0, 5.0, "Then we go"), seg(5.5, 8.0, "to the")];
        let fresh = vec![seg(3.1, 5.2, "Потом переходим"), seg(5.6, 8.0, "к следующему")];
        let out = apply_window(&t, &originals, &fresh).unwrap();
        let got: Vec<(&str, &str)> = out.segments.iter().map(|s| (s.speaker.as_str(), s.text.as_str())).collect();
        assert_eq!(got, [("me", "Добрый день"), ("spk0", "Потом переходим"), ("spk1", "к следующему"), ("me", "Спасибо")]);
    }

    #[test]
    fn skips_windows_the_user_edited() {
        let t = Transcript { segments: vec![tseg("me", 0.0, 2.0, "правка"), tseg("me", 2.5, 3.0, "ещё")] };
        // Пользователь удалил/сдвинул одну из исходных реплик — окно не трогаем.
        let originals = vec![seg(0.0, 2.0, "х"), seg(4.0, 5.0, "у")];
        assert!(apply_window(&t, &originals, &[seg(0.0, 2.0, "новое")]).is_none());
    }

    #[test]
    fn plan_roundtrip_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let plan = RefinePlan {
            jobs: vec![RefineJob {
                wav: "mic_norm.wav".into(),
                windows: vec![RefineWindow {
                    start_secs: 1.0,
                    end_secs: 15.0,
                    kind: RefineKind::Hard,
                    severity: 1.5,
                    speech_secs: 8.0,
                    confidence: Some(0.6),
                    originals: vec![seg(1.0, 3.0, "а")],
                }],
            }],
            language: Some("ru".into()),
            vocabulary: None,
        };
        save(dir.path(), &plan).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Some(plan));
        save(dir.path(), &RefinePlan::default()).unwrap();
        assert_eq!(load(dir.path()).unwrap(), None);
    }
}
