//! Объединение нескольких записей в один разговор: запись прервали и потом
//! продолжили — части сшиваются в новую встречу (исходные остаются).
//!
//! Записанные встречи (две дорожки) сшиваются по дорожкам: каждая часть
//! дополняется тишиной до общей длины, чтобы «Я» и «Собеседник» не
//! разъехались. Если среди частей есть импортированный файл (одна дорожка),
//! всё сводится в одну дорожку `audio.wav`. Между частями — секунда тишины.
//! Расшифровки, если они есть у всех частей, сшиваются со сдвигом времени —
//! повторно расшифровывать не нужно (правки пользователя целы).

use std::path::Path;

use crate::error::{AppError, AppResult};
use crate::model::Meeting;
use crate::storage::Repo;
use crate::transcript::{Transcript, TranscriptSegment};

const SR: u32 = 16_000;
/// Пауза между частями, секунды.
pub const GAP_SECS: f64 = 1.0;

/// Сшивает расшифровки частей: `parts` — (начало части в общей записи, текст).
pub fn join_transcripts(parts: &[(f64, Transcript)]) -> Transcript {
    let segments = parts
        .iter()
        .flat_map(|(offset, t)| {
            t.segments.iter().map(move |s| TranscriptSegment {
                start_secs: s.start_secs + offset,
                end_secs: s.end_secs + offset,
                ..s.clone()
            })
        })
        .collect();
    Transcript { segments }
}

/// Заголовок объединённой встречи: первый «настоящий» заголовок части.
pub fn merged_title(titles: &[&str]) -> String {
    titles
        .iter()
        .map(|t| t.trim())
        .find(|t| !t.is_empty() && *t != "Новая встреча")
        .map(|t| format!("{t} (объединено)"))
        .unwrap_or_else(|| "Объединённая встреча".to_string())
}

/// Отсчёты дорожки встречи (16 кГц моно i16); нет файла или он пуст — пусто.
fn read_track(dir: &Path, name: &str, tmp: &Path) -> Vec<i16> {
    let src = dir.join(name);
    if !src.exists() {
        return Vec::new();
    }
    // Через общий декодер — на случай другой частоты/формата.
    if crate::decode::decode_to_wav_16k_mono(&src, tmp).is_err() {
        return Vec::new();
    }
    let out = hound::WavReader::open(tmp)
        .ok()
        .map(|r| r.into_samples::<i16>().filter_map(Result::ok).collect())
        .unwrap_or_default();
    let _ = std::fs::remove_file(tmp);
    out
}

fn writer(path: &Path) -> AppResult<hound::WavWriter<std::io::BufWriter<std::fs::File>>> {
    let spec = hound::WavSpec { channels: 1, sample_rate: SR, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    hound::WavWriter::create(path, spec).map_err(|e| AppError::Audio(e.to_string()))
}

fn put(w: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>, x: &[i16], len: usize) -> AppResult<()> {
    for i in 0..len {
        w.write_sample(x.get(i).copied().unwrap_or(0)).map_err(|e| AppError::Audio(e.to_string()))?;
    }
    Ok(())
}

/// Объединяет встречи `ids` (в этом порядке) в новую встречу `new_id`.
pub fn merge_meetings(repo: &Repo, data_root: &Path, ids: &[String], new_id: &str) -> AppResult<Meeting> {
    if ids.len() < 2 {
        return Err(AppError::InvalidInput("для объединения нужно хотя бы две записи".into()));
    }
    let parts: Vec<Meeting> = ids.iter().map(|id| repo.get(id)).collect::<AppResult<_>>()?;
    let dir = crate::service::meeting_dir(data_root, new_id);
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("part.tmp.wav");
    let two_tracks = parts.iter().all(|m| m.source == "recorded");
    let gap = (GAP_SECS * SR as f64) as usize;
    let mut offsets = Vec::with_capacity(parts.len());
    let mut total = 0usize;
    let result = (|| -> AppResult<()> {
        if two_tracks {
            let (mut wm, mut ws) = (writer(&dir.join("mic.wav"))?, writer(&dir.join("system.wav"))?);
            for (k, m) in parts.iter().enumerate() {
                let src = crate::service::meeting_dir(data_root, &m.id);
                let mic = read_track(&src, "mic.wav", &tmp);
                let sys = read_track(&src, "system.wav", &tmp);
                let len = mic.len().max(sys.len()) + if k + 1 < parts.len() { gap } else { 0 };
                offsets.push(total as f64 / SR as f64);
                put(&mut wm, &mic, len)?;
                put(&mut ws, &sys, len)?;
                total += len;
            }
            wm.finalize().map_err(|e| AppError::Audio(e.to_string()))?;
            ws.finalize().map_err(|e| AppError::Audio(e.to_string()))?;
        } else {
            let mut wa = writer(&dir.join("audio.wav"))?;
            for (k, m) in parts.iter().enumerate() {
                let src = crate::service::meeting_dir(data_root, &m.id);
                let mixed: Vec<i16> = if m.source == "recorded" {
                    let (mic, sys) = (read_track(&src, "mic.wav", &tmp), read_track(&src, "system.wav", &tmp));
                    (0..mic.len().max(sys.len()))
                        .map(|i| {
                            let v = mic.get(i).copied().unwrap_or(0) as i32 + sys.get(i).copied().unwrap_or(0) as i32;
                            v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
                        })
                        .collect()
                } else {
                    read_track(&src, "audio.wav", &tmp)
                };
                let len = mixed.len() + if k + 1 < parts.len() { gap } else { 0 };
                offsets.push(total as f64 / SR as f64);
                put(&mut wa, &mixed, len)?;
                total += len;
            }
            wa.finalize().map_err(|e| AppError::Audio(e.to_string()))?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }

    // Расшифровки — только если они есть у всех частей (и для двух дорожек:
    // «Я»/«Собеседник» у сводной одной дорожки не различить — там тоже ок,
    // имена голосов остаются как были).
    let transcripts: Vec<Option<Transcript>> =
        parts.iter().map(|m| crate::service::load_transcript(data_root, &m.id).ok().flatten()).collect();
    let status = if transcripts.iter().all(Option::is_some) {
        let joined = join_transcripts(
            &offsets.iter().copied().zip(transcripts.into_iter().map(Option::unwrap)).collect::<Vec<_>>(),
        );
        crate::service::save_transcript(data_root, new_id, &joined)?;
        "transcribed"
    } else {
        "recorded"
    };
    let titles: Vec<&str> = parts.iter().map(|m| m.title.as_str()).collect();
    let notes: Vec<&str> = parts.iter().map(|m| m.notes.trim()).filter(|n| !n.is_empty()).collect();
    let first = &parts[0];
    let meeting = Meeting {
        id: new_id.to_string(),
        created_at: first.created_at.clone(),
        title: merged_title(&titles),
        participants: parts.iter().map(|m| m.participants.trim()).find(|p| !p.is_empty()).unwrap_or("").to_string(),
        topic: first.topic.clone(),
        duration_secs: (total as u64) / SR as u64,
        folder: new_id.to_string(),
        status: status.into(),
        source: if two_tracks { "recorded".into() } else { "imported".into() },
        notes: notes.join("\n\n"),
        collection: first.collection.clone(),
    };
    repo.insert(&meeting)?;
    Ok(meeting)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(path: &Path, secs: f64, amp: i16) {
        let mut w = writer(path).unwrap();
        for _ in 0..(secs * SR as f64) as usize {
            w.write_sample(amp).unwrap();
        }
        w.finalize().unwrap();
    }

    fn meeting(id: &str, source: &str, title: &str, at: &str) -> Meeting {
        Meeting {
            id: id.into(),
            created_at: at.into(),
            title: title.into(),
            participants: String::new(),
            topic: String::new(),
            duration_secs: 0,
            folder: id.into(),
            status: "recorded".into(),
            source: source.into(),
            notes: String::new(),
            collection: String::new(),
        }
    }

    fn seg(sp: &str, a: f64, b: f64, t: &str) -> TranscriptSegment {
        TranscriptSegment { speaker: sp.into(), start_secs: a, end_secs: b, text: t.into(), origin: None }
    }

    #[test]
    fn joins_parts_in_given_order_keeping_tracks_aligned() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repo::open_in_memory().unwrap();
        for (id, mic, sys) in [("a", 2.0, 1.0), ("b", 1.0, 3.0)] {
            let d = crate::service::meeting_dir(root.path(), id);
            std::fs::create_dir_all(&d).unwrap();
            wav(&d.join("mic.wav"), mic, 100);
            wav(&d.join("system.wav"), sys, 200);
            repo.insert(&meeting(id, "recorded", if id == "a" { "Планёрка" } else { "Новая встреча" }, "2026-10-01T10:00:00Z")).unwrap();
        }
        crate::service::save_transcript(root.path(), "a", &Transcript { segments: vec![seg("me", 0.5, 1.5, "начало")] }).unwrap();
        crate::service::save_transcript(root.path(), "b", &Transcript { segments: vec![seg("them", 0.2, 0.8, "продолжение")] }).unwrap();

        let m = merge_meetings(&repo, root.path(), &["a".into(), "b".into()], "ab").unwrap();
        assert_eq!(m.source, "recorded");
        assert_eq!(m.status, "transcribed");
        assert_eq!(m.title, "Планёрка (объединено)");
        let d = crate::service::meeting_dir(root.path(), "ab");
        let len = |f: &str| hound::WavReader::open(d.join(f)).unwrap().len();
        // Часть A: 2 с (по длинной дорожке) + 1 с паузы; часть B: 3 с.
        assert_eq!(len("mic.wav"), 6 * SR);
        assert_eq!(len("system.wav"), 6 * SR);
        let t = crate::service::load_transcript(root.path(), "ab").unwrap().unwrap();
        assert_eq!(t.segments[1].text, "продолжение");
        assert!((t.segments[1].start_secs - 3.2).abs() < 1e-6);
        // Исходные встречи на месте.
        assert_eq!(repo.list().unwrap().len(), 3);
    }

    #[test]
    fn imported_part_makes_one_track() {
        let root = tempfile::tempdir().unwrap();
        let repo = Repo::open_in_memory().unwrap();
        let d = crate::service::meeting_dir(root.path(), "r");
        std::fs::create_dir_all(&d).unwrap();
        wav(&d.join("mic.wav"), 1.0, 100);
        wav(&d.join("system.wav"), 1.0, 100);
        repo.insert(&meeting("r", "recorded", "", "2026-10-01T10:00:00Z")).unwrap();
        let d = crate::service::meeting_dir(root.path(), "i");
        std::fs::create_dir_all(&d).unwrap();
        wav(&d.join("audio.wav"), 2.0, 50);
        repo.insert(&meeting("i", "imported", "", "2026-10-01T11:00:00Z")).unwrap();

        let m = merge_meetings(&repo, root.path(), &["i".into(), "r".into()], "ir").unwrap();
        assert_eq!(m.source, "imported");
        assert_eq!(m.status, "recorded"); // расшифровок не было
        assert_eq!(m.title, "Объединённая встреча");
        let r = hound::WavReader::open(crate::service::meeting_dir(root.path(), "ir").join("audio.wav")).unwrap();
        assert_eq!(r.len(), 4 * SR);
        let s: Vec<i16> = r.into_samples::<i16>().map(Result::unwrap).collect();
        // Через декодер — с точностью до единицы округления.
        assert!((s[0] - 50).abs() <= 2, "{}", s[0]);
        let mid = s[(3.5 * SR as f64) as usize];
        assert!((mid - 200).abs() <= 2, "{mid}"); // «Я» + «Собеседник» сведены
        assert!(merge_meetings(&repo, root.path(), &["i".into()], "x").is_err());
    }
}
